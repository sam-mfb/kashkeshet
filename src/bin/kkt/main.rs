mod cli;

use kashkeshet::commands::lanes::{
    docker_import_lanes, existing_db, local_import_lanes, local_operator, read_lanes_warning,
};
use kashkeshet::commands::lifecycle::write_backup;
use kashkeshet::commands::serve;
use kashkeshet::commands::service::{self, ServiceAction, ServiceInstall, service_plan};
use kashkeshet::commands::setup::InstallOptions;
use kashkeshet::commands::{self, doctor};
use kashkeshet::logging::{init_logging, init_stderr_logging};
use kashkeshet::machine::connection::{ConnectionOptions, admin_client, connect, remote_client};
use kashkeshet::machine::credentials::{CredentialFile, StateFile, watch_token};
use kashkeshet::machine::keys::{
    KeyKind, admin_name_line, create_key, key_header, key_show, local_operator_key, local_web_key,
    new_key_file,
};
use kashkeshet::machine::record::{
    check_setup, read_setup, read_unfinished, read_unrecorded, recorded_setup, unfinished_mode,
    write_setup_record,
};
use kashkeshet::machine::settings::{ServeArgs, load_serve_options, read_config};
use kashkeshet::process::{end_on_closed_output, process_env};
use kkt_agent_client as mcp_client;
use kkt_backup as backup;
use kkt_clock::SystemClock;
use kkt_config::{self as config, DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL, Limits};
use kkt_container as docker;
use kkt_lane_files as lane_files;
use kkt_names as names;
use kkt_provenance::Build;
use kkt_remote_access as tailscale;
use kkt_rules::{LaneSource, Operator, Revoked};
use kkt_service::{Manager, OsSystem};
use kkt_settings::{self as settings, Env, Platform, ServeOptions};
use kkt_setup as setup;
use kkt_store::Db;
use kkt_watch as watch;
use std::io::Write;
use std::path::{Path, PathBuf};
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
        Task::Backup { folder, db, config } => mode_backup(env, folder.as_deref(), db, config),
        Task::Status => mode_status(env, serve_args),
        Task::Upgrade { allow_downgrade } => mode_upgrade(env, serve_args, allow_downgrade).await,
        Task::Uninstall => mode_uninstall(env, serve_args),
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

fn mode_backup(
    env: &Env,
    folder: Option<&Path>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    match read_setup(env)?.map(|record| record.mode) {
        Some(setup::Mode::Docker(install)) => {
            if db.is_some() || config.is_some() {
                return Err("Docker backup uses the configured container; --db and --config apply only to a local service".into());
            }
            let written = docker::backup(&docker::OsHost, &docker::Folder(install.folder), folder)
                .map_err(|e| e.to_string())?;
            println!("Backed up to {}", written.display());
            Ok(())
        }
        Some(setup::Mode::Client) => {
            Err("this machine is a client; back up the server on the machine where it runs".into())
        }
        Some(setup::Mode::Service { .. }) | None => {
            let folder = folder.ok_or("a local backup needs FOLDER; run `kkt backup <folder>`")?;
            backup(env, folder, db, config)
        }
    }
}

fn mode_status(env: &Env, serve_args: ServeArgs) -> Result<(), String> {
    let (unfinished, unfinished_error) = match read_unfinished(env) {
        Ok(unfinished) => (unfinished, None),
        Err(e) => (None, Some(e)),
    };
    let recorded = read_setup(env)?;
    let (unrecorded, unrecorded_error) = match (&recorded, &unfinished, &unfinished_error) {
        (None, None, None) => match read_unrecorded(env, serve_args) {
            Ok(found) => (found, None),
            Err((_, e)) => (None, Some(e)),
        },
        _ => (None, None),
    };
    if let Some(path) = local_web_key(env, unfinished.as_ref(), unrecorded.as_ref())? {
        println!(
            "Web key: {} (read with `kkt key show --web`)",
            path.display()
        );
    }
    if let Some(path) = local_operator_key(
        env,
        unfinished.as_ref(),
        unrecorded.as_ref(),
        KeyKind::Admin,
    )?
    .filter(|path| path.exists())
    {
        println!(
            "Admin key: {} (read with `kkt key show --admin`; agents holding it use /mcp-admin)",
            path.display()
        );
        let key = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read key file {}: {e}", path.display()))?;
        if let Some(line) = admin_name_line(&path, &key, "Admin name")? {
            println!("{line}");
        }
    }
    if let Some(e) = unfinished_error {
        println!("Unfinished setup: unknown ({e}). Run `kkt setup` again.");
    }
    if let Some(unfinished) = &unfinished {
        println!(
            "Unfinished setup: {} Run `kkt setup` again to finish it.",
            unfinished.summary()
        );
    }
    if let Some(e) = &unrecorded_error {
        println!("Unrecorded install: {e}");
    }
    if let Some(found) = &unrecorded {
        println!(
            "Unrecorded install: {} Run `kkt setup` to record it, so `kkt upgrade` manages it.",
            found.summary()
        );
    }
    let (mode, url) = match (recorded, unfinished, unrecorded) {
        (Some(record), _, _) => {
            if let Some(clients) = &record.clients {
                println!(
                    "Clients use: {} (switched from this machine's server; \
                     `kkt switch --local` points them back)",
                    clients.url
                );
            }
            (record.mode, record.url)
        }
        (None, Some(unfinished), _) => match unfinished.record.clone() {
            Some(record) => (record.mode, record.url),
            None => (
                unfinished_mode(env, &unfinished)?,
                "not known until the install finishes".to_string(),
            ),
        },
        (None, None, Some(found)) => (found.mode, found.url),
        (None, None, None) => {
            println!(
                "No setup record; checking this machine's kkt service. Run `kkt setup` to configure this machine."
            );
            print_configured_name_pack(env)?;
            return service::run(env, ServiceAction::Status);
        }
    };
    println!("Mode: {}\nServer: {url}", mode.name());
    match mode {
        setup::Mode::Service { .. } => {
            print_configured_name_pack(env)?;
            service::run(env, ServiceAction::Status)
        }
        setup::Mode::Docker(install) => {
            let folder = docker::Folder(install.folder);
            let state = docker::status(&docker::OsHost, &folder).map_err(|e| e.to_string())?;
            match &state.name_pack {
                Ok(pack) => println!("Name pack: {pack}"),
                Err(reason) => println!("Name pack: unavailable ({reason})"),
            }
            println!("Docker folder: {}", folder.path().display());
            println!(
                "Settings present: {}\nRunning: {}\nVersion: {}\nTailscale: {}",
                state.files_present,
                state.running,
                state.version.as_deref().unwrap_or("unknown"),
                state.tailscale
            );
            match &state.login {
                Some(docker::Login::LoggedIn) => println!("Tailscale login: logged in"),
                Some(docker::Login::NeedsLogin) => println!(
                    "Tailscale login: needed. The machine's key expired, or it was logged out, so it is off the tailnet. Run `kkt setup` to log in again."
                ),
                Some(docker::Login::Unreachable) => {
                    println!("Tailscale login: unknown; the Tailscale container does not answer")
                }
                Some(docker::Login::Other(state)) => println!("Tailscale login: {state}"),
                None => {}
            }
            Ok(())
        }
        setup::Mode::Client => {
            println!("Client only; the server runs on another machine.");
            Ok(())
        }
    }
}

/// Status reads local configuration, not a running server's command line.
fn print_configured_name_pack(env: &Env) -> Result<(), String> {
    let (_, options) = read_config(env, None)?;
    println!(
        "Name pack (configured; serve flags may override): {}",
        options
            .name_pack
            .as_deref()
            .unwrap_or(Limits::DEFAULT.name_pack.name())
    );
    Ok(())
}

async fn mode_upgrade(
    env: &Env,
    serve_args: ServeArgs,
    allow_downgrade: bool,
) -> Result<(), String> {
    let mut record = recorded_setup(env, serve_args)?;
    let current = match &record.mode {
        setup::Mode::Client => {
            return Err(
                "this machine is a client; upgrade the server on the machine where it runs".into(),
            );
        }
        setup::Mode::Docker(install) => {
            let path = install.folder.join(docker::ENV_FILE);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            docker::version_in_env(&text)
                .ok_or_else(|| format!("{} has no KKT_VERSION", path.display()))?
        }
        setup::Mode::Service { .. } => record.written_by.clone(),
    };
    if setup::upgrade(&current, Build::CURRENT.version, allow_downgrade)? == setup::Upgrade::Already
    {
        println!("Server is already at kkt {}", Build::CURRENT.version);
        return Ok(());
    }
    match &record.mode {
        setup::Mode::Docker(install) => {
            docker::upgrade(
                &docker::OsHost,
                &SystemClock,
                &docker::Folder(install.folder.clone()),
                Build::CURRENT.version,
                record.key_file.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { .. } => {
            // Validate the replacement service before doing anything. Its
            // program is current_exe, i.e. the binary just installed by the user.
            let plan = service_plan(env, None, None, None, false)?;
            let folder = env
                .config_dir()
                .ok_or("set HOME to find the backup folder")?
                .join(config::SERVICE_BACKUP_FOLDER);
            backup(env, &folder, None, None)?;
            if let Some(key) = &plan.new_web_key {
                new_key_file(key, false)?;
                println!(
                    "Created a new web key at {}. Read it with `kkt key show --web`.",
                    key.display()
                );
            }
            let manager = Manager::for_this_os(env).map_err(|e| e.to_string())?;
            for line in manager
                .install(&OsSystem, &plan.spec)
                .map_err(|e| e.to_string())?
            {
                println!("{line}");
            }
        }
        setup::Mode::Client => {
            return Err(
                "this machine is a client; upgrade the server on the machine where it runs".into(),
            );
        }
    }
    record.written_by = Build::CURRENT.version.to_string();
    write_setup_record(env, &record)?;
    println!("Upgraded the server to kkt {}", Build::CURRENT.version);
    Ok(())
}

fn mode_uninstall(env: &Env, serve_args: ServeArgs) -> Result<(), String> {
    let record = recorded_setup(env, serve_args)?;
    // Clients switched to another server keep using it: the machine stays
    // recorded as its client.
    let stays_client = record.clients.clone().map(|server| setup::Record {
        mode: setup::Mode::Client,
        written_by: Build::CURRENT.version.to_string(),
        url: server.url,
        key_file: server.key_file,
        clients: None,
    });
    match record.mode {
        setup::Mode::Docker(install) => {
            docker::down(&docker::OsHost, &docker::Folder(install.folder), false)
                .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { tailscale: shared } => {
            if shared {
                let loaded = load_serve_options(env, None, ServeOptions::NONE)?;
                let sources = loaded.sources();
                let settings = loaded
                    .from_file
                    .resolve(
                        env.default_db(Platform::CURRENT).unwrap_or_default(),
                        env.access_key_file(),
                    )
                    .map_err(|e| sources.invalid(e))?;
                tailscale::stop(&tailscale::OsHost, settings.bind)?;
            }
            service::run(env, ServiceAction::Uninstall)?;
        }
        setup::Mode::Client => {}
    }
    if let Some(client) = stays_client {
        write_setup_record(env, &client)?;
        println!(
            "Removed this machine's server. Data, keys, backups and client configurations were \
             kept, and this machine stays set up as a client of {}.",
            client.url
        );
        return Ok(());
    }
    let path = setup::record_path(env).ok_or("set HOME to find the setup record")?;
    std::fs::remove_file(&path).map_err(|e| format!("could not remove {}: {e}", path.display()))?;
    println!(
        "Removed this machine's setup. Data, keys, backups and client configurations were kept."
    );
    Ok(())
}

/// `kkt backup`: backs up the database `kkt serve` would use (or `db`),
/// with the config file and existing keys it would read, into `folder`.
fn backup(
    env: &Env,
    folder: &Path,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let (config_file, options) = read_config(env, config)?;
    let database = existing_db(env, db, options.db.clone())?;
    let access_key = match (&options.access_key_file, options.no_access_key) {
        (Some(path), _) => Some(path.clone()),
        (None, Some(true)) => None,
        (None, _) => env.access_key_file().filter(|path| path.exists()),
    };
    let web_key = match access_key.as_deref().map(settings::web_key_path) {
        Some(path) => match path.try_exists() {
            Ok(true) => Some(path),
            // Installations from before separate web keys have none until
            // their first start or service upgrade.
            Ok(false) => None,
            Err(e) => return Err(format!("could not inspect {}: {e}", path.display())),
        },
        None => None,
    };
    // Only servers whose operator asked for one have an admin key.
    let admin_key = match access_key.as_deref().and_then(settings::admin_key_path) {
        Some(path) => match path.try_exists() {
            Ok(true) => Some(path),
            Ok(false) => None,
            Err(e) => return Err(format!("could not inspect {}: {e}", path.display())),
        },
        None => None,
    };
    // And only an admin key made with a name has one.
    let admin_name = match admin_key.as_deref().map(settings::admin_name_path) {
        Some(path) => match path.try_exists() {
            Ok(true) => Some(path),
            Ok(false) => None,
            Err(e) => return Err(format!("could not inspect {}: {e}", path.display())),
        },
        None => None,
    };
    let contents = backup::Contents {
        database: &database,
        config: config_file.as_deref(),
        access_key: access_key.as_deref(),
        web_key: web_key.as_deref(),
        admin_key: admin_key.as_deref(),
        admin_name: admin_name.as_deref(),
    };
    let written = write_backup(folder, &SystemClock, &contents)?;
    if let Some(path) = &web_key {
        println!("  web key {}", path.display());
    }
    if let Some(path) = &admin_key {
        println!("  admin key {}", path.display());
    }
    if let Some(path) = &admin_name {
        println!("  admin name {}", path.display());
    }
    println!("Backed up to {}", written.display());
    println!("  database {}", database.display());
    if let Some(path) = &config_file {
        println!("  config file {}", path.display());
    }
    match &access_key {
        Some(path) => println!("  access key {}", path.display()),
        None => println!("  no access key"),
    }
    Ok(())
}
