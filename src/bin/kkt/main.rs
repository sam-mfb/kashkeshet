//! `cli` defines the command line and turns it into plain inputs. `run`
//! builds the parts of kkt's three assemblies in one place,
//! kkt-install-commands' for the install's commands,
//! kkt-operator-commands' for the operator's and kkt-agent-commands' for
//! the agents', from the implementations this program chooses, and runs
//! each command; `kkt serve` is composed by the library's `commands::serve`
//! over kkt-server.

mod cli;

use kashkeshet::commands::serve;
use kashkeshet::machine::agent_parts::agent_parts;
use kashkeshet::machine::connection::ConnectionOptions;
use kashkeshet::machine::native::NativeParts;
use kashkeshet::machine::settings::LOG_OWN_TARGETS;
use kashkeshet::machine::settings::ServeArgs;
use kashkeshet::machine::settings::ServeOptions;
use kkt_agent_commands::{self as agent_commands, connect, watch::WatchTask};
use kkt_install_commands::record::InstallReads;
use kkt_install_commands::service::{ServiceAction, ServiceInstall};
use kkt_install_commands::setup::InstallOptions;
use kkt_log::{Level, Log};
use kkt_log_tracing::TracingLog;
use kkt_log_tracing::{DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL};
use kkt_log_tracing::{init_logging, init_stderr_logging};
use kkt_machine::{Env, Machine, Platform};
use kkt_machine_os::OsMachine;
use kkt_operator_commands::agent::AgentAction;
use kkt_operator_commands::keys::KeyKind;
use kkt_operator_commands::lanes::LanesAction;
use kkt_setup as setup;
use kkt_terminal::Terminal;
use kkt_terminal::end_on_closed_output;
use std::path::PathBuf;

// Many threads: `kkt setup` and `kkt switch` block on this runtime from
// inside it (`block_in_place`), which only a runtime of many threads allows.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::process::ExitCode {
    // A flag `cli` could not turn into a plain value is an error of a
    // command other than `serve`, `watch`, `register` or `reclaim`, so it
    // is handled as theirs is.
    let task = cli::task();
    if !matches!(task, Ok(Task::Serve { .. })) {
        end_on_closed_output();
    }
    let env = OsMachine.env(&kkt_llm_agent_config::ENV_FOLDERS);
    let platform = OsMachine.platform();
    // `serve` starts logging once its settings say where the log goes.
    let logging = match &task {
        Ok(Task::Serve { .. }) => Ok(()),
        // Standard output contains only the command's result, with no library logs.
        Ok(Task::Watch(_) | Task::Register { .. } | Task::Reclaim { .. }) => {
            init_stderr_logging(LOG_OWN_TARGETS, DEFAULT_LOG_LEVEL)
        }
        _ => init_logging(
            None,
            LOG_OWN_TARGETS,
            DEFAULT_LOG_LEVEL,
            DEFAULT_DEPENDENCY_LOG_LEVEL,
        ),
    };
    let result = match (logging, task) {
        (Err(e), _) => Err(e.to_string()),
        (Ok(()), Err(e)) => Err(e),
        (Ok(()), Ok(task)) => run(&env, platform, cli::serve_args, task).await,
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            // An error from before logging started still needs saying.
            if kkt_log_tracing::logging_started()
                || init_logging(
                    None,
                    LOG_OWN_TARGETS,
                    DEFAULT_LOG_LEVEL,
                    DEFAULT_DEPENDENCY_LOG_LEVEL,
                )
                .is_ok()
            {
                TracingLog.record(Level::Error, module_path!(), format_args!("{e}"), &[]);
            } else {
                Terminal.errors(format_args!("error: {e}\n"));
            }
            std::process::ExitCode::FAILURE
        }
    }
}

/// One command, with its inputs as plain values. `cli` makes it from the
/// command line.
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
    // The three assemblies' parts, built once, here. None reads anything
    // until a command asks.
    let native = NativeParts::new(env, platform).with_service_args(serve_args);
    let setup_lanes = native.setup_lanes();
    let install = native.install_parts(&setup_lanes);
    let installed = InstallReads::new(&install, env, platform);
    let installed = kashkeshet::machine::native::InstalledOnThisMachine(&installed);
    let lane_servers = native.lane_servers(&installed);
    let operator = native.operator_parts(&installed, &lane_servers);
    let mut agents = agent_parts(env, platform, serve_args);
    let done = match task {
        Task::Serve {
            config,
            ignore_setup,
            options,
        } => {
            if !ignore_setup {
                kkt_install_commands::record::check_setup(
                    install.serve,
                    install.setup_files,
                    install.service,
                    install.output,
                )
                .map_err(|e| e.to_string())?;
            }
            serve::run(env, platform, config, options).await
        }
        Task::KeyCreate {
            path,
            force,
            show,
            kind,
            name,
        } => kkt_operator_commands::key::create(
            &operator,
            kkt_operator_commands::key::NewKey {
                kind,
                path,
                name: name.as_deref(),
            },
            force,
            show,
        )
        .map_err(|e| e.to_string()),
        Task::Agent { action, db, config } => {
            kkt_operator_commands::agent::run(&operator, action, db, config)
                .await
                .map_err(|e| e.to_string())
        }
        Task::Lanes { action, db, config } => {
            kkt_operator_commands::lanes::run(&operator, action, db, config)
                .await
                .map_err(|e| e.to_string())
        }
        Task::Service(action) => kkt_install_commands::service::run(&install, action)
            .await
            .map_err(|e| e.to_string()),
        Task::Backup { folder, db, config } => {
            kkt_install_commands::lifecycle::backup(&install, folder.as_deref(), db, config)
                .map_err(|e| e.to_string())
        }
        Task::Status => kkt_install_commands::lifecycle::status(&install, env, platform)
            .await
            .map_err(|e| e.to_string()),
        Task::Upgrade { allow_downgrade } => {
            kkt_install_commands::lifecycle::upgrade(&install, env, platform, allow_downgrade)
                .await
                .map_err(|e| e.to_string())
        }
        Task::Uninstall => kkt_install_commands::lifecycle::uninstall(&install, env, platform)
            .await
            .map_err(|e| e.to_string()),
        Task::Switch(how) => kkt_install_commands::setup::run_switch(&install, env, how)
            .await
            .map_err(|e| e.to_string()),
        Task::KeyHeader { key_file, json } => {
            kkt_operator_commands::key::header(&operator, key_file, json).map_err(|e| e.to_string())
        }
        Task::KeyShow { key_file, kind } => {
            return kkt_operator_commands::key::show(&operator, key_file, kind)
                .map_err(|e| e.to_string());
        }
        Task::Connect {
            client,
            connection,
            client_config,
            write,
        } => connect::run(&agents, env, &client, connection, client_config, write)
            .map_err(|e| e.to_string()),
        Task::Doctor(connection) => {
            kkt_install_commands::doctor::run(&install, env, platform, connection)
                .await
                .map_err(|e| e.to_string())
        }
        Task::Setup { answers, options } => {
            kkt_install_commands::setup::run(env, platform, &install, answers, options)
                .await
                .map_err(|e| e.to_string())
        }
        Task::Register {
            token_file,
            recovery_file,
            description,
            handle,
            lane,
            connection,
        } => agent_commands::register(
            &agents,
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
        .map_err(|e| e.to_string()),
        Task::Reclaim {
            handle,
            recovery_file,
            token_file,
            connection,
        } => agent_commands::reclaim(&agents, connection, &handle, recovery_file, token_file)
            .await
            .map_err(|e| e.to_string()),
        // The only command with an exit code of its own.
        Task::Watch(task) => {
            return agent_commands::watch::run(&mut agents, task)
                .await
                .map_err(|e| e.to_string());
        }
    };
    done.map(|()| std::process::ExitCode::SUCCESS)
}
