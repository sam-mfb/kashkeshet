mod cli;

use kashkeshet::commands::lanes::{
    docker_import_lanes, existing_db, local_import_lanes, local_operator, read_lanes_warning,
};
use kashkeshet::commands::lifecycle;
use kashkeshet::commands::serve;
use kashkeshet::commands::service::{self, ServiceAction, ServiceInstall};
use kashkeshet::commands::setup::InstallOptions;
use kashkeshet::commands::{self, doctor};
use kashkeshet::logging::{init_logging, init_stderr_logging};
use kashkeshet::machine::connection::{ConnectionOptions, admin_client, connect, remote_client};
use kashkeshet::machine::credentials::{CredentialFile, StateFile, watch_token};
use kashkeshet::machine::keys::{KeyKind, create_key, key_header, key_show};
use kashkeshet::machine::record::{check_setup, read_setup};
use kashkeshet::machine::settings::{ServeArgs, read_config};
use kashkeshet::process::{end_on_closed_output, process_env};
use kkt_agent_client as mcp_client;
use kkt_clock::SystemClock;
use kkt_config::{self as config, DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL};
use kkt_container as docker;
use kkt_lane_files as lane_files;
use kkt_names as names;
use kkt_rules::{LaneSource, Operator, Revoked};
use kkt_settings::{Env, ServeOptions};
use kkt_setup as setup;
use kkt_store::Db;
use kkt_watch as watch;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // A flag `cli` could not turn into a plain value is an error of a
    // command other than `serve`, `watch`, `register` or `reclaim`, so it
    // is handled as theirs is.
    let task = cli::task();
    if !matches!(task, Ok(Task::Serve { .. })) {
        end_on_closed_output();
    }
    let env = process_env();
    // `serve` starts logging once its settings say where the log goes.
    let logging = match &task {
        Ok(Task::Serve { .. }) => Ok(()),
        // Standard output contains only the command's result, with no library logs.
        Ok(Task::Watch(_) | Task::Register { .. } | Task::Reclaim { .. }) => init_stderr_logging(),
        _ => init_logging(None, DEFAULT_LOG_LEVEL, DEFAULT_DEPENDENCY_LOG_LEVEL),
    };
    let result = match (logging, task) {
        (Err(e), _) => Err(e),
        (Ok(()), Err(e)) => Err(e),
        (Ok(()), Ok(task)) => run(&env, cli::serve_args, task).await,
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            // An error from before logging started still needs saying.
            if tracing::dispatcher::has_been_set()
                || init_logging(None, DEFAULT_LOG_LEVEL, DEFAULT_DEPENDENCY_LOG_LEVEL).is_ok()
            {
                tracing::error!("{e}");
            } else {
                eprintln!("error: {e}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

/// One command, with its inputs as plain values. `cli` makes it from the
/// command line.
// Made once at startup, so the size of the largest variant costs nothing.
#[allow(clippy::large_enum_variant)]
enum Task {
    Status,
    Upgrade {
        allow_downgrade: bool,
    },
    Uninstall,
    Serve {
        config: Option<PathBuf>,
        ignore_setup: bool,
        options: ServeOptions,
    },
    KeyCreate {
        path: Option<PathBuf>,
        force: bool,
        show: bool,
        kind: KeyKind,
        name: Option<String>,
    },
    KeyHeader {
        key_file: Option<PathBuf>,
        json: bool,
    },
    KeyShow {
        key_file: Option<PathBuf>,
        kind: KeyKind,
    },
    Agent {
        action: AgentAction,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    },
    Lanes {
        action: LanesAction,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    },
    Backup {
        folder: Option<PathBuf>,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    },
    Service(ServiceAction),
    Setup {
        answers: setup::Answers,
        options: InstallOptions,
    },
    Switch(setup::Switch),
    Connect {
        client: String,
        connection: ConnectionOptions,
        client_config: Option<PathBuf>,
        write: bool,
    },
    Doctor(ConnectionOptions),
    Watch(WatchTask),
    Register {
        token_file: PathBuf,
        recovery_file: Option<PathBuf>,
        description: Option<String>,
        handle: Option<String>,
        lane: Option<String>,
        connection: ConnectionOptions,
    },
    Reclaim {
        handle: String,
        recovery_file: PathBuf,
        token_file: PathBuf,
        connection: ConnectionOptions,
    },
}

/// What `kkt agent` does.
enum AgentAction {
    List,
    Revoke { handle: String },
}

/// What `kkt lanes` does.
enum LanesAction {
    Export {
        folder: PathBuf,
    },
    Import {
        folder: PathBuf,
        retire_missing: bool,
        source_folder: Option<String>,
        source_commit: Option<String>,
    },
}

/// What `kkt watch` watches, and where it keeps its place.
struct WatchTask {
    /// Through the admin endpoint, with the admin key, instead of as an agent.
    admin: bool,
    token_file: Option<PathBuf>,
    state_file: Option<PathBuf>,
    connection: ConnectionOptions,
    options: watch::Options,
}

/// Runs one command.
async fn run(
    env: &Env,
    serve_args: ServeArgs,
    task: Task,
) -> Result<std::process::ExitCode, String> {
    let done = match task {
        Task::Serve {
            config,
            ignore_setup,
            options,
        } => {
            if !ignore_setup {
                check_setup(env)?;
            }
            serve::run(env, config, options).await
        }
        Task::KeyCreate {
            path,
            force,
            show,
            kind,
            name,
        } => create_key(env, serve_args, path, force, show, kind, name.as_deref()),
        Task::Agent { action, db, config } => agent(env, action, db, config).await,
        Task::Lanes { action, db, config } => lanes(env, action, db, config).await,
        Task::Service(action) => service::run(env, action),
        Task::Backup { folder, db, config } => {
            lifecycle::backup(env, folder.as_deref(), db, config)
        }
        Task::Status => lifecycle::status(env, serve_args),
        Task::Upgrade { allow_downgrade } => {
            lifecycle::upgrade(env, serve_args, allow_downgrade).await
        }
        Task::Uninstall => lifecycle::uninstall(env, serve_args),
        Task::Switch(how) => commands::setup::run_switch(env, serve_args, how).await,
        Task::KeyHeader { key_file, json } => key_header(env, serve_args, key_file, json),
        Task::KeyShow { key_file, kind } => {
            return key_show(env, serve_args, key_file, kind);
        }
        Task::Connect {
            client,
            connection,
            client_config,
            write,
        } => connect(env, &client, connection, client_config, write),
        Task::Doctor(connection) => doctor::run(env, serve_args, connection).await,
        Task::Setup { answers, options } => {
            commands::setup::run(env, serve_args, answers, options).await
        }
        Task::Register {
            token_file,
            recovery_file,
            description,
            handle,
            lane,
            connection,
        } => {
            register_agent(
                env,
                connection,
                token_file,
                recovery_file,
                handle.as_deref(),
                lane.as_deref(),
                description,
            )
            .await
        }
        Task::Reclaim {
            handle,
            recovery_file,
            token_file,
            connection,
        } => reclaim_agent(env, connection, &handle, recovery_file, token_file).await,
        // The only command with an exit code of its own.
        Task::Watch(task) => return watch(env, serve_args, task).await,
    };
    done.map(|()| std::process::ExitCode::SUCCESS)
}

async fn register_agent(
    env: &Env,
    connection: ConnectionOptions,
    token_file: PathBuf,
    recovery_file: Option<PathBuf>,
    handle: Option<&str>,
    lane: Option<&str>,
    description: Option<String>,
) -> Result<(), String> {
    // The server checks the handle and lane too; checking first gives the
    // reason, and leaves no files behind.
    if let Some(handle) = handle {
        names::check_proposed_handle(handle, config::MAX_HANDLE_CHARS)?;
    }
    if let Some(lane) = lane {
        names::check_lane_name(lane, config::MAX_LANE_NAME_CHARS)?;
    }
    let client = remote_client(env, connection)?;
    let mut token = CredentialFile::create(token_file)?;
    let mut recovery = recovery_file.map(CredentialFile::create).transpose()?;
    let registered = client
        .register(handle, lane)
        .await
        .map_err(|e| match (&e, lane) {
            // Which lane, and why, is not a credential.
            (mcp_client::Failure::Refused { code, message }, Some(_))
                if code == "not_found" || message.contains("lane") =>
            {
                format!("the server refused the lane: {message}")
            }
            _ => avatar_error(e),
        })?;
    // Save credentials before any optional call that might fail.
    token.save(&registered.token)?;
    if let Some(recovery) = &mut recovery {
        recovery.save(&registered.recovery_code)?;
    }
    if let Some(description) = description {
        client
            .set_description(&registered.token, &description)
            .await
            .map_err(avatar_error)?;
    }
    println!("{}", registered.handle);
    Ok(())
}

async fn reclaim_agent(
    env: &Env,
    connection: ConnectionOptions,
    handle: &str,
    recovery_file: PathBuf,
    token_file: PathBuf,
) -> Result<(), String> {
    let recovery = watch_token(Some(&recovery_file))?;
    let client = remote_client(env, connection)?;
    let mut token = CredentialFile::create(token_file)?;
    let reclaimed = client
        .reclaim(handle, &recovery)
        .await
        .map_err(avatar_error)?;
    token.save(&reclaimed.token)?;
    println!("{}", reclaimed.handle);
    Ok(())
}

// Do not echo server or transport error details for credential-bearing calls.
fn avatar_error(error: mcp_client::Failure) -> String {
    match error {
        mcp_client::Failure::Lost(_) => "could not reach the server or read its reply".into(),
        mcp_client::Failure::Refused { code, .. } => {
            format!("the server refused the avatar request ({code})")
        }
    }
}

async fn watch(
    env: &Env,
    serve_args: ServeArgs,
    task: WatchTask,
) -> Result<std::process::ExitCode, String> {
    let server: Box<dyn watch::Server> = if task.admin {
        Box::new(mcp_client::AdminRemote::new(
            admin_client(env, serve_args, task.connection)?,
            Arc::new(SystemClock),
        ))
    } else {
        let token = watch_token(task.token_file.as_deref())?;
        Box::new(mcp_client::Remote::from_client(
            remote_client(env, task.connection)?,
            token,
        ))
    };
    let place = task.state_file.map(StateFile);
    let outcome = watch::run(
        server.as_ref(),
        &SystemClock,
        place.as_ref().map(|p| p as &dyn watch::Place),
        &mut WatchOutput,
        &task.options,
    )
    .await?;
    Ok(match outcome {
        watch::Outcome::Received => std::process::ExitCode::SUCCESS,
        watch::Outcome::TimedOut => std::process::ExitCode::from(config::WATCH_TIMED_OUT_EXIT),
    })
}

/// Message lines to standard output, flushed one by one; notes to
/// standard error.
struct WatchOutput;

impl watch::Report for WatchOutput {
    fn message(&mut self, line: &str) -> Result<(), String> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}")
            .and_then(|()| out.flush())
            .map_err(|e| format!("could not print a message: {e}"))
    }

    fn note(&mut self, text: &str) {
        eprintln!("kkt watch: {text}");
    }
}

/// `kkt agent`: the operator's commands, on the database `kkt serve` would
/// use unless `--db` names another. Never creates a database.
async fn agent(
    env: &Env,
    command: AgentAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    // An explicit --db needs nothing from the config file, so it is not read.
    let config_db = match db {
        Some(_) => None,
        None => read_config(env, config)?.1.db,
    };
    let path = existing_db(env, db, config_db)?;
    let db = Db::open(&path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let operator = Operator::new(db, Arc::new(SystemClock));
    match command {
        AgentAction::List => {
            let agents = operator.agents().await.map_err(|e| e.to_string())?;
            if agents.is_empty() {
                println!("no agents in {}", path.display());
            }
            for a in agents {
                let seen = a.last_seen.as_deref().unwrap_or("never");
                let revoked = match &a.revoked_at {
                    Some(at) => format!("  revoked {at}"),
                    None => String::new(),
                };
                println!(
                    "{}  created {}  last seen {seen}{revoked}",
                    a.handle, a.created_at
                );
            }
        }
        AgentAction::Revoke { handle } => {
            match operator
                .revoke(handle.clone())
                .await
                .map_err(|e| e.to_string())?
            {
                Revoked::Now { at } => println!("revoked {handle} at {at}"),
                Revoked::Already { at } => println!("{handle} was already revoked at {at}"),
            }
        }
    }
    Ok(())
}

/// `kkt lanes`: on the database `kkt serve` would use unless `--db` names
/// another, or for kkt in Docker, in its container.
async fn lanes(
    env: &Env,
    command: LanesAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    match read_setup(env)?.map(|record| record.mode) {
        Some(setup::Mode::Docker(install)) => {
            if db.is_some() || config.is_some() {
                return Err(
                    "kkt in Docker keeps its lanes in the container; --db and --config \
                            apply only to a local server"
                        .into(),
                );
            }
            let folder = docker::Folder(install.folder);
            match command {
                LanesAction::Export { folder: dest } => {
                    let dest = std::path::absolute(&dest)
                        .map_err(|e| format!("could not resolve {}: {e}", dest.display()))?;
                    let said = docker::export_lanes(&docker::OsHost, &folder, &dest)
                        .map_err(|e| e.to_string())?;
                    print!("{said}");
                    eprintln!("Exported to {}", dest.display());
                }
                LanesAction::Import {
                    folder: from,
                    retire_missing,
                    source_folder,
                    ..
                } => {
                    if source_folder.is_some() {
                        return Err(
                            "--source-folder is for `kkt lanes import` in the container \
                                    itself"
                                .into(),
                        );
                    }
                    let read = read_lanes_warning(&from, retire_missing)?;
                    print!("{}", docker_import_lanes(&folder, read, retire_missing)?);
                }
            }
            Ok(())
        }
        Some(setup::Mode::Client) => Err("this machine is a client; run `kkt lanes` on the \
                                          machine where the server runs"
            .into()),
        Some(setup::Mode::Service { .. }) | None => match command {
            LanesAction::Export { folder } => {
                let (operator, _) = local_operator(env, db, config)?;
                let lanes = operator.current_lanes().await.map_err(|e| e.to_string())?;
                let files: Vec<lane_files::LaneFile> = lanes
                    .iter()
                    .map(|l| lane_files::LaneFile {
                        name: l.name.clone(),
                        description: l.current.description.clone(),
                        text: l.current.text.clone(),
                    })
                    .collect();
                let written = lane_files::write_folder(&lane_files::OsDisk, &folder, &files)?;
                for l in &lanes {
                    println!(
                        "{} (version {})",
                        lane_files::file_name(&l.name),
                        l.current.version
                    );
                }
                eprintln!("Exported {} lanes to {}", lanes.len(), written.display());
                Ok(())
            }
            LanesAction::Import {
                folder,
                retire_missing,
                source_folder,
                source_commit,
            } => {
                let source = source_folder.map(|folder| LaneSource {
                    folder,
                    commit: source_commit,
                });
                let read = read_lanes_warning(&folder, retire_missing)?;
                let report =
                    local_import_lanes(env, db, config, read, retire_missing, source).await?;
                print!("{report}");
                Ok(())
            }
        },
    }
}
