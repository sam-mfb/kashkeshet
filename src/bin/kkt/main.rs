//! `cli` defines the command line and turns it into plain inputs; the
//! library `kashkeshet` has one module per command, what kkt keeps on this
//! machine, and the terminal; everything else is in the `kkt-*` crates.

mod cli;

use kashkeshet::commands::agent;
use kashkeshet::commands::lanes;
use kashkeshet::commands::lifecycle;
use kashkeshet::commands::serve;
use kashkeshet::commands::service;
use kashkeshet::commands::setup::InstallOptions;
use kashkeshet::commands::{self, doctor, key};
use kashkeshet::machine::agent_parts::agent_parts;
use kashkeshet::machine::connection::ConnectionOptions;
use kashkeshet::machine::record::check_setup;
use kashkeshet::machine::settings::ServeArgs;
use kashkeshet::machine::settings::ServeOptions;
use kkt_agent_commands::{self as agent_commands, connect, watch::WatchTask};
use kkt_clock_system::SystemClock;
use kkt_logs::{DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL};
use kkt_logs_os::{init_logging, init_stderr_logging};
use kkt_machine::{Env, Machine, Platform};
use kkt_machine_os::OsMachine;
use kkt_operate::agent::AgentAction;
use kkt_operate::keys::KeyKind;
use kkt_operate::lanes::LanesAction;
use kkt_operate::service::{ServiceAction, ServiceInstall};
use kkt_setup as setup;
use kkt_terminal_os::Terminal;
use kkt_terminal_os::end_on_closed_output;
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
    let machine = OsMachine;
    let env = machine.env();
    let platform = machine.platform();
    // `serve` starts logging once its settings say where the log goes.
    let logging = match &task {
        Ok(Task::Serve { .. }) => Ok(()),
        // Standard output contains only the command's result, with no library logs.
        Ok(Task::Watch(_) | Task::Register { .. } | Task::Reclaim { .. }) => {
            init_stderr_logging(DEFAULT_LOG_LEVEL)
        }
        _ => init_logging(
            None,
            DEFAULT_LOG_LEVEL,
            DEFAULT_DEPENDENCY_LOG_LEVEL,
            Arc::new(SystemClock),
        ),
    };
    let result = match (logging, task) {
        (Err(e), _) => Err(e),
        (Ok(()), Err(e)) => Err(e),
        (Ok(()), Ok(task)) => run(&env, platform, cli::serve_args, task).await,
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            // An error from before logging started still needs saying.
            if tracing::dispatcher::has_been_set()
                || init_logging(
                    None,
                    DEFAULT_LOG_LEVEL,
                    DEFAULT_DEPENDENCY_LOG_LEVEL,
                    Arc::new(SystemClock),
                )
                .is_ok()
            {
                tracing::error!("{e}");
            } else {
                Terminal.errors(format_args!("error: {e}\n"));
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

/// Runs one command.
async fn run(
    env: &Env,
    platform: Platform,
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
            serve::run(env, platform, config, options).await
        }
        Task::KeyCreate {
            path,
            force,
            show,
            kind,
            name,
        } => key::create(
            env,
            platform,
            serve_args,
            kkt_operate::key::NewKey {
                kind,
                path,
                name: name.as_deref(),
            },
            force,
            show,
        ),
        Task::Agent { action, db, config } => agent::run(env, platform, action, db, config).await,
        Task::Lanes { action, db, config } => lanes::run(env, platform, action, db, config).await,
        Task::Service(action) => service::run(env, platform, action).await,
        Task::Backup { folder, db, config } => {
            lifecycle::backup(env, platform, folder.as_deref(), db, config)
        }
        Task::Status => lifecycle::status(env, platform, serve_args).await,
        Task::Upgrade { allow_downgrade } => {
            lifecycle::upgrade(env, platform, serve_args, allow_downgrade).await
        }
        Task::Uninstall => lifecycle::uninstall(env, platform, serve_args).await,
        Task::Switch(how) => commands::setup::run_switch(env, how).await,
        Task::KeyHeader { key_file, json } => {
            key::header(env, platform, serve_args, key_file, json)
        }
        Task::KeyShow { key_file, kind } => {
            return key::show(env, platform, serve_args, key_file, kind);
        }
        Task::Connect {
            client,
            connection,
            client_config,
            write,
        } => connect::run(
            &agent_parts(env, platform, serve_args),
            env,
            &client,
            connection,
            client_config,
            write,
        ),
        Task::Doctor(connection) => doctor::run(env, platform, serve_args, connection).await,
        Task::Setup { answers, options } => {
            commands::setup::run(env, platform, serve_args, answers, options).await
        }
        Task::Register {
            token_file,
            recovery_file,
            description,
            handle,
            lane,
            connection,
        } => {
            agent_commands::register(
                &agent_parts(env, platform, serve_args),
                connection,
                token_file,
                recovery_file,
                agent_commands::NewAvatar {
                    handle: handle.as_deref(),
                    lane: lane.as_deref(),
                    description,
                },
            )
            .await
        }
        Task::Reclaim {
            handle,
            recovery_file,
            token_file,
            connection,
        } => {
            agent_commands::reclaim(
                &agent_parts(env, platform, serve_args),
                connection,
                &handle,
                recovery_file,
                token_file,
            )
            .await
        }
        // The only command with an exit code of its own.
        Task::Watch(task) => {
            return agent_commands::watch::run(&mut agent_parts(env, platform, serve_args), task)
                .await;
        }
    };
    done.map(|()| std::process::ExitCode::SUCCESS)
}
