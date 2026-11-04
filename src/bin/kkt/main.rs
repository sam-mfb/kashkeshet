mod cli;

use kashkeshet::commands::lifecycle::write_backup;
use kashkeshet::commands::serve;
use kashkeshet::commands::service::{self, ServiceAction, ServiceInstall, service_plan};
use kashkeshet::logging::{init_logging, init_stderr_logging};
use kashkeshet::machine::connection::{
    ConnectionOptions, admin_client, client_connection, connect, recorded_connection,
    remote_client, serve_connection,
};
use kashkeshet::machine::credentials::{CredentialFile, StateFile, watch_token};
use kashkeshet::machine::keys::{
    KeyKind, admin_key_collision, admin_name_line, create_key, key_header, key_show,
    local_operator_key, local_web_key, new_key_file, write_admin_name, write_key_file,
};
use kashkeshet::machine::record::{
    check_setup, docker_folder, docker_url, read_setup, read_unfinished, read_unrecorded,
    recorded_setup, unfinished_mode, write_setup_record,
};
use kashkeshet::machine::settings::{ServeArgs, load_serve_options, read_config, serve_bind};
use kashkeshet::process::{end_on_closed_output, kkt_path, process_env, write_whole};
use kashkeshet::terminal::ask::TerminalAsk;
use kashkeshet::terminal::browser_login;
use kkt_agent_client as mcp_client;
use kkt_backup as backup;
use kkt_client_config::{self as clients, Client, Setup};
use kkt_clock::{Clock, SystemClock};
use kkt_config::{
    self as config, DEFAULT_BIND, DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL, Limits,
};
use kkt_container as docker;
use kkt_diagnostics as diagnostics;
use kkt_lane_files as lane_files;
use kkt_names::{self as names, AdminName};
use kkt_provenance::Build;
use kkt_remote_access as tailscale;
use kkt_rules::{ImportedLane, LaneImport, LaneOutcome, LaneSave, LaneSource, Operator, Revoked};
use kkt_service::{Manager, OsSystem};
use kkt_settings::{self as settings, Env, Platform, ServeOptions};
use kkt_setup as setup;
use kkt_store::Db;
use kkt_watch as watch;
use sha2::{Digest, Sha256};
use std::io::{IsTerminal, Write};
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

/// The name of the doctor line about the service on this machine.
const SERVICE_CHECK: &str = "this machine's kkt service";

/// The name of the doctor line about a setup that installed a server but
/// did not record it.
const SETUP_CHECK: &str = "this machine's setup";

async fn doctor(env: &Env, serve_args: ServeArgs, flags: ConnectionOptions) -> Result<(), String> {
    // With no record, a server setup installed but did not record stands in
    // for it; with no note of one either, a server kkt installed here anyway.
    let unfinished = read_unfinished(env);
    let recorded = read_setup(env)?;
    let unrecorded = match (&recorded, &unfinished) {
        (None, Ok(None)) => read_unrecorded(env, serve_args),
        _ => Ok(None),
    };
    let found = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let record = match recorded {
        Some(record) => Some(record),
        None => match &unfinished {
            Ok(Some(unfinished)) => unfinished.record.clone(),
            _ => found.map(|found| found.record(Build::CURRENT.version)),
        },
    };
    let connection = match &record {
        Some(record) if flags.config.is_none() => recorded_connection(record.for_clients(), flags)?,
        _ => client_connection(env, flags, false)?,
    };
    let mut checks = diagnostics::inspect(&connection.url, connection.key_file.as_deref()).await?;
    // A server machine whose clients were switched checks the other server
    // above, and so gets a client's fix.
    let switched = record.as_ref().is_some_and(|r| r.clients.is_some());
    let mode = match (record, &unfinished) {
        (Some(record), _) => Some(record.mode),
        (None, Ok(Some(unfinished))) => Some(unfinished_mode(env, unfinished)?),
        (None, _) => None,
    };
    let fix = match switched {
        true => server_fix(&setup::Mode::Client),
        false => mode.as_ref().and_then(server_fix),
    };
    if let Some(fix) = fix {
        point_server_fix_at(&mut checks, &fix);
    }
    let web_key = local_web_key(
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
    )?;
    if let Some(path) = &web_key {
        let fix = match &mode {
            Some(setup::Mode::Docker(install)) => docker_web_key_fix(&install.folder),
            _ => "check the operator web key on the server; `kkt key create --web` creates it if missing, then restart the server".to_string(),
        };
        checks.extend(diagnostics::inspect_web_key(path, &fix));
    }
    // Most servers have no admin key; only one that exists is checked.
    let admin_key = local_operator_key(
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
        KeyKind::Admin,
    )?
    .filter(|path| path.exists());
    if let Some(path) = &admin_key {
        let fix = match &mode {
            Some(setup::Mode::Docker(install)) => docker_admin_key_fix(&install.folder),
            _ => "check the admin key on the server; `kkt key create --admin --force` replaces it, then restart the server".to_string(),
        };
        checks.extend(diagnostics::inspect_admin_key(path, &fix));
    }
    // The last lines are about this machine, whatever server `--url` points
    // at: how `kkt setup` set it up (or installed it, unfinished), or else
    // its kkt service.
    match mode {
        // The server is elsewhere; the lines above are about it.
        Some(setup::Mode::Client) => {}
        Some(setup::Mode::Docker(install)) => {
            let folder = docker::Folder(install.folder);
            let state = docker::status(&docker::OsHost, &folder);
            let running = matches!(&state, Ok(state) if state.running);
            checks.extend(docker_checks(state, &folder));
            // A missing or unreadable copy has its own line above.
            if running
                && let Some(path) = &web_key
                && path.is_file()
            {
                let compared = docker::compare_web_key(&docker::OsHost, &folder, path);
                checks.push(docker_web_key_check(compared, &folder));
            }
        }
        Some(setup::Mode::Service { .. }) | None => checks.push(service_check(env)),
    }
    match unfinished {
        Ok(None) => {}
        Ok(Some(unfinished)) => checks.push(diagnostics::Check::problem(
            SETUP_CHECK,
            unfinished.summary(),
            "run `kkt setup` again to finish it",
        )),
        Err(e) => checks.push(diagnostics::Check::problem(
            SETUP_CHECK,
            e,
            "run `kkt setup` again",
        )),
    }
    checks.extend(unrecorded_check(unrecorded));
    let (report, all_ok) = doctor_report(checks);
    print!("{report}");
    match clients::codex_password_store_hint(env) {
        Ok(true) => println!(
            "info: Codex: mcp_oauth_credentials_store is unset; a password-store timeout can prevent kkt from connecting. See https://github.com/sam-mfb/kashkeshet/blob/main/docs/clients.md#codex for the optional file-store setting and what it changes."
        ),
        Ok(false) => {}
        Err(error) => println!("info: Codex config could not be checked: {error}"),
    }
    if all_ok {
        Ok(())
    } else {
        Err("doctor found problems; see the fixes above".into())
    }
}

/// How to record a server found installed with no record.
const ADOPT_FIX: &str = "run `kkt setup` to record it (`kkt setup --adopt` with no terminal), so `kkt upgrade` manages it";

/// The doctor's line about a server kkt installed here with no record, if
/// one was looked for and found. kkt in Docker unrecorded is a problem:
/// only setup makes the managed folder, and `kkt upgrade` needs the record.
/// A service may have been installed with `kkt service install`, which
/// never needed a record, so for one it is only said.
fn unrecorded_check(
    unrecorded: Result<Option<setup::Unrecorded>, (setup::Purpose, String)>,
) -> Option<diagnostics::Check> {
    Some(match unrecorded {
        Ok(None) => return None,
        Ok(Some(found)) => match found.mode {
            setup::Mode::Docker(_) => {
                diagnostics::Check::problem(SETUP_CHECK, found.summary(), ADOPT_FIX)
            }
            _ => diagnostics::Check::ok(
                SETUP_CHECK,
                format!(
                    "{} To have `kkt upgrade` manage it, run `kkt setup` to record it.",
                    found.summary()
                ),
            ),
        },
        Err((setup::Purpose::Docker, e)) => {
            diagnostics::Check::problem(SETUP_CHECK, e, "run `kkt setup`, which sets it up again")
        }
        Err((_, e)) => diagnostics::Check::ok(SETUP_CHECK, e),
    })
}

/// The fix for a server that does not answer, on a machine set up as `mode`
/// where starting the kkt service is not how kkt runs; `None` where it is.
fn server_fix(mode: &setup::Mode) -> Option<String> {
    match mode {
        setup::Mode::Client => Some(
            "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)"
                .to_string(),
        ),
        // From inside the folder: Compose reads a relative COMPOSE_FILE in
        // .env against the current folder.
        setup::Mode::Docker(install) => Some(format!(
            "this machine runs kkt in Docker: run `kkt status`, or start it with `cd {} && docker compose up -d`",
            diagnostics::shell_word(&install.folder.display().to_string())
        )),
        setup::Mode::Service { .. } => None,
    }
}

/// Replaces the fix on a failed server check, which tells the person to
/// start the kkt service, with `fix`, for a machine where that is not how kkt
/// runs.
fn point_server_fix_at(checks: &mut [diagnostics::Check], fix: &str) {
    for check in checks.iter_mut() {
        if check.name == diagnostics::SERVER_CHECK && !check.ok {
            check.fix = Some(fix.to_string());
        }
    }
}

/// The doctor's line about the kkt service on this machine.
fn service_check(env: &Env) -> diagnostics::Check {
    match Manager::for_this_os(env).and_then(|manager| manager.state(&OsSystem)) {
        Ok(state) if !state.installed => diagnostics::Check::ok(
            SERVICE_CHECK,
            "not installed; optional when running `kkt serve` in a terminal",
        ),
        Ok(state) if state.running => {
            diagnostics::Check::ok(SERVICE_CHECK, "installed and running")
        }
        Ok(_) => diagnostics::Check::problem(
            SERVICE_CHECK,
            "installed but not running",
            "run `kkt service start`",
        ),
        Err(error) => diagnostics::Check::problem(
            SERVICE_CHECK,
            error.to_string(),
            "check `kkt service status` and your user service manager",
        ),
    }
}

/// The name of the doctor line about kkt in Docker on this machine.
const DOCKER_CHECK: &str = "this machine's kkt in Docker";

/// The name of the doctor line about the Tailscale login of kkt in Docker.
const DOCKER_TAILSCALE_CHECK: &str = "this machine's Tailscale login";

/// The name of the doctor line comparing the web key's host copy with the
/// key of kkt in Docker.
const DOCKER_WEB_KEY_CHECK: &str = "web key host copy";

/// The doctor's lines about kkt in Docker, from what `docker::status` found
/// in the managed `folder`: kkt itself, then, if it is served over Tailscale,
/// the Tailscale login.
fn docker_checks(
    state: Result<docker::State, docker::DockerError>,
    folder: &docker::Folder,
) -> Vec<diagnostics::Check> {
    let login = match &state {
        Ok(state) => state.login.clone(),
        Err(_) => None,
    };
    let mut checks = vec![docker_check(state, folder)];
    checks.extend(login.map(|login| tailscale_login_check(&login, folder)));
    checks
}

/// How to save, or read, the web key of kkt in Docker in `folder`.
fn docker_web_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup` (run again with the same answers) or `kkt upgrade` (to a new version) saves the server's web key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --web`",
        diagnostics::shell_word(&folder.display().to_string())
    )
}

/// How to save, or read, the admin key of kkt in Docker in `folder`.
fn docker_admin_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup --admin-key` (run again with the same answers) saves the server's admin key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --admin`",
        diagnostics::shell_word(&folder.display().to_string())
    )
}

/// The doctor's line about whether the web key's host copy is the Docker
/// server's key, from what `docker::compare_web_key` found.
fn docker_web_key_check(
    compared: Result<docker::KeyCopy, docker::DockerError>,
    folder: &docker::Folder,
) -> diagnostics::Check {
    match compared {
        Ok(docker::KeyCopy::Same) => {
            diagnostics::Check::ok(DOCKER_WEB_KEY_CHECK, "the same as the server's")
        }
        Ok(docker::KeyCopy::Differs) => diagnostics::Check::problem(
            DOCKER_WEB_KEY_CHECK,
            "differs from the server's web key, which was changed after the copy was saved; it no longer logs in",
            docker_web_key_fix(folder.path()),
        ),
        Err(e) => diagnostics::Check::problem(
            DOCKER_WEB_KEY_CHECK,
            format!("could not compare it with the server's web key: {e}"),
            format!(
                "check kkt with `cd {} && docker compose logs kkt`",
                diagnostics::shell_word(&folder.path().display().to_string())
            ),
        ),
    }
}

/// The doctor's line about the Tailscale container's login.
fn tailscale_login_check(login: &docker::Login, folder: &docker::Folder) -> diagnostics::Check {
    match login {
        docker::Login::LoggedIn => diagnostics::Check::ok(DOCKER_TAILSCALE_CHECK, "logged in"),
        docker::Login::NeedsLogin => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "not logged in: the machine's key expired, or it was logged out, so it is off the tailnet and its HTTPS address does not answer",
            "run `kkt setup` again and log in with the link it shows",
        ),
        docker::Login::Unreachable => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "the Tailscale container does not answer",
            format!(
                "start it with `cd {} && docker compose up -d`",
                diagnostics::shell_word(&folder.path().display().to_string())
            ),
        ),
        docker::Login::Other(state) => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            format!("Tailscale says: {state}"),
            "check the machine in the Tailscale admin console, or run `kkt setup` again",
        ),
    }
}

/// The doctor's line about kkt in Docker, from what `docker::status` found
/// in the managed `folder`.
fn docker_check(
    state: Result<docker::State, docker::DockerError>,
    folder: &docker::Folder,
) -> diagnostics::Check {
    let shown = folder.path().display().to_string();
    match state {
        Ok(state) if !state.files_present => diagnostics::Check::problem(
            DOCKER_CHECK,
            format!("its files are missing from {shown}"),
            "run `kkt setup` again",
        ),
        Ok(state) if state.running => diagnostics::Check::ok(
            DOCKER_CHECK,
            format!(
                "running kkt {}",
                state.version.as_deref().unwrap_or("(version unknown)")
            ),
        ),
        Ok(_) => diagnostics::Check::problem(
            DOCKER_CHECK,
            "not running",
            format!(
                // From inside the folder: Compose reads a relative
                // COMPOSE_FILE in .env against the current folder.
                "start it with `cd {} && docker compose up -d`",
                diagnostics::shell_word(&shown)
            ),
        ),
        Err(error) => diagnostics::Check::problem(
            DOCKER_CHECK,
            error.to_string(),
            "check that Docker is running, then run `kkt status`",
        ),
    }
}

/// The doctor's lines for `checks`, and whether all of them are ok.
fn doctor_report(checks: Vec<diagnostics::Check>) -> (String, bool) {
    let mut report = String::new();
    let mut all_ok = true;
    for check in checks {
        all_ok &= check.ok;
        report.push_str(&format!(
            "{}: {}: {}\n",
            if check.ok { "ok" } else { "problem" },
            check.name,
            check.detail
        ));
        if let Some(fix) = check.fix {
            report.push_str(&format!("  fix: {fix}\n"));
        }
    }
    (report, all_ok)
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
        Task::Switch(how) => run_switch(env, serve_args, how).await,
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
        Task::Doctor(connection) => doctor(env, serve_args, connection).await,
        Task::Setup { answers, options } => run_setup(env, serve_args, answers, options).await,
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

/// `kkt setup`: asks on the terminal if there is one, and does the work
/// through [`OsSteps`].
async fn run_setup(
    env: &Env,
    serve_args: ServeArgs,
    answers: setup::Answers,
    options: InstallOptions,
) -> Result<(), String> {
    let ask = TerminalAsk {
        interactive: std::io::stdin().is_terminal(),
    };
    let steps = OsSteps {
        env,
        serve_args,
        options,
    };
    // Steps are synchronous; doctor's checks are async, so they block on
    // this runtime from inside.
    tokio::task::block_in_place(|| {
        setup::run(&answers, Build::CURRENT.version, &ask, &steps).map(|_| ())
    })
}

/// `kkt switch`: asks on the terminal if there is one, and does the work
/// through [`OsSteps`].
async fn run_switch(env: &Env, serve_args: ServeArgs, how: setup::Switch) -> Result<(), String> {
    let ask = TerminalAsk {
        interactive: std::io::stdin().is_terminal(),
    };
    // Switching installs nothing, so these are never used.
    let options = InstallOptions {
        kkt_version: Build::CURRENT.version.to_string(),
        docker_project: config::DOCKER_PROJECT.to_string(),
        docker_port: DEFAULT_BIND.port(),
    };
    let steps = OsSteps {
        env,
        serve_args,
        options,
    };
    tokio::task::block_in_place(|| {
        setup::switch(&how, Build::CURRENT.version, &ask, &steps).map(|_| ())
    })
}

/// How to install, beyond what setup asks.
struct InstallOptions {
    kkt_version: String,
    docker_project: String,
    docker_port: u16,
}

/// What `kkt setup` does to this machine.
struct OsSteps<'a> {
    env: &'a Env,
    serve_args: ServeArgs,
    options: InstallOptions,
}

/// Adds `host` to `allow_hosts` in the default config file, making the file
/// if there is none.
fn allow_host_in_config(env: &Env, host: &str) -> Result<(), String> {
    let path = env
        .config_file()
        .ok_or("set HOME to keep kkt's config file")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let updated = settings::with_allowed_host(&text, host)
        .map_err(|e| format!("invalid setting: {e} (in {})", path.display()))?;
    if updated == text {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, updated).map_err(|e| format!("could not write {}: {e}", path.display()))
}

impl setup::Steps for OsSteps<'_> {
    fn install_service(
        &self,
        tailscale: bool,
        allow_hosts: &[String],
    ) -> Result<setup::Installed, String> {
        // Before anything changes: Tailscale must be logged in, with HTTPS.
        let node = match tailscale {
            true => Some(tailscale::prepare(&tailscale::OsHost)?),
            false => None,
        };
        if let Some(node) = &node {
            allow_host_in_config(self.env, &node.dns_name)?;
        }
        for host in allow_hosts {
            allow_host_in_config(self.env, host)?;
        }
        service::run(
            self.env,
            ServiceAction::Install(ServiceInstall {
                config: None,
                db: None,
                access_key_file: None,
                no_access_key: false,
                dry_run: false,
            }),
        )?;
        let connection = serve_connection(self.env, ConnectionOptions::NONE, false)?;
        let url = match &node {
            Some(node) => {
                tailscale::serve(&tailscale::OsHost, serve_bind(self.env)?)?;
                format!("https://{}/mcp", node.dns_name)
            }
            None => connection.url,
        };
        if let Some(access) = &connection.key_file {
            println!(
                "The web key is in {}. Read it with `kkt key show --web`.",
                settings::web_key_path(access).display()
            );
        }
        Ok(setup::Installed {
            url,
            key_file: connection.key_file,
        })
    }

    fn install_docker(
        &self,
        tailscale: Option<&setup::TailscaleLogin>,
        allow_hosts: &[String],
    ) -> Result<(setup::Installed, setup::DockerSetup), String> {
        let options = &self.options;
        let folder = docker_folder(self.env)?;
        let key_file = self
            .env
            .config_dir()
            .ok_or("set HOME to keep the access key")?
            .join(config::DOCKER_KEY_FILE_NAME);
        let tailscale_inputs = match tailscale {
            Some(login) => Some(docker::Tailscale {
                hostname: login.hostname.clone(),
                auth_key: match &login.auth_key_file {
                    Some(path) => Some(
                        std::fs::read_to_string(path)
                            .map_err(|e| format!("could not read {}: {e}", path.display()))?
                            .trim()
                            .to_string(),
                    ),
                    None => None,
                },
            }),
            None => None,
        };
        let inputs = docker::Inputs {
            version: options.kkt_version.clone(),
            project: options.docker_project.clone(),
            port: options.docker_port,
            allow_hosts: allow_hosts.to_vec(),
            tailscale: tailscale_inputs,
        };
        let tailscale = tailscale.is_some();
        let provisioned = tokio::runtime::Handle::current()
            .block_on(docker::provision(
                &docker::OsHost,
                &SystemClock,
                &browser_login::TerminalLink::new(),
                &docker::Folder(folder.clone()),
                &inputs,
                &key_file,
            ))
            .map_err(|e| e.to_string())?;
        let url = docker_url(options.docker_port, provisioned.node.as_ref());
        let (hostname, tailnet) = provisioned.node.unzip();
        Ok((
            setup::Installed {
                url,
                key_file: Some(key_file),
            },
            setup::DockerSetup {
                folder,
                tailscale,
                hostname,
                tailnet,
            },
        ))
    }

    fn stage_key(&self, key: &str) -> Result<PathBuf, String> {
        let dir = self
            .env
            .config_dir()
            .ok_or("set HOME to keep the key in ~/.config/kkt")?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let path = dir.join(format!("{}.partial", config::CLIENT_KEY_FILE_NAME));
        write_key_file(&path, key, true)?;
        Ok(path)
    }

    fn keep_key(&self, staged: &Path, url: &str) -> Result<PathBuf, String> {
        let dir = staged.parent().unwrap_or(Path::new("."));
        // One file per server, so a key for another server, which an
        // earlier setup record may name, is never replaced.
        let server: String = Sha256::digest(url.as_bytes())
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect();
        let path = dir.join(format!("{}-{server}", config::CLIENT_KEY_FILE_NAME));
        std::fs::rename(staged, &path)
            .map_err(|e| format!("could not move the key to {}: {e}", path.display()))?;
        Ok(path)
    }

    fn discard_key(&self, staged: &Path) -> Result<(), String> {
        std::fs::remove_file(staged)
            .map_err(|e| format!("could not remove {}: {e}", staged.display()))
    }

    fn detect_clients(&self) -> Vec<Client> {
        let dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let on_path = |program: &str| dirs.iter().any(|dir| dir.join(program).is_file());
        Client::ALL
            .into_iter()
            .filter(|client| client.installed(self.env, &on_path))
            .collect()
    }

    fn doctor(&self, record: &setup::Record) -> Result<String, String> {
        // The server the clients use: on a machine whose clients were
        // switched, another machine's, checked as a client's.
        let server = record.for_clients();
        let mode = match record.clients {
            Some(_) => setup::Mode::Client,
            None => record.mode.clone(),
        };
        // A server set up just now may not answer yet; a client's server
        // either answers or the address is wrong, so it isn't waited for.
        let wait = match mode {
            setup::Mode::Client => std::time::Duration::ZERO,
            _ => config::SETUP_SERVER_WAIT,
        };
        let mut checks = tokio::runtime::Handle::current().block_on(inspect_until_reachable(
            &server.url,
            server.key_file.as_deref(),
            &waiting_note(&server.url, &mode),
            &SystemClock,
            wait,
        ))?;
        if let Some(fix) = server_fix(&mode) {
            point_server_fix_at(&mut checks, &fix);
        }
        match doctor_report(checks) {
            (report, true) => Ok(report.trim_end().to_string()),
            (report, false) => Err(report.trim_end().to_string()),
        }
    }

    fn unfinished(&self) -> Result<Option<setup::Unfinished>, String> {
        read_unfinished(self.env)
    }

    fn save_unfinished(&self, unfinished: &setup::Unfinished) -> Result<(), String> {
        let path = setup::unfinished_path(self.env).ok_or("set HOME to note the install")?;
        write_whole(&path, &unfinished.render()?)
    }

    fn forget_unfinished(&self) -> Result<(), String> {
        let Some(path) = setup::unfinished_path(self.env) else {
            return Ok(());
        };
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("could not remove {}: {e}", path.display())),
        }
    }

    fn add_admin_key(&self, record: &setup::Record, name: &AdminName) -> Result<PathBuf, String> {
        let access = record
            .key_file
            .as_deref()
            .ok_or("an admin key needs the server to have an access key")?;
        match &record.mode {
            setup::Mode::Docker(install) => tokio::runtime::Handle::current()
                .block_on(docker::add_admin_key(
                    &docker::OsHost,
                    &SystemClock,
                    &docker::Folder(install.folder.clone()),
                    access,
                    name,
                ))
                .map_err(|e| e.to_string()),
            setup::Mode::Service { .. } => {
                let admin = settings::admin_key_path(access).ok_or_else(admin_key_collision)?;
                // A key file that cannot be inspected is not replaced:
                // `new_key_file` refuses to write over one. Only a new key
                // is given the name.
                let made = Manager::for_this_os(self.env)
                    .and_then(|manager| {
                        manager.add_key(&OsSystem, &admin, &|path| {
                            let key = new_key_file(path, false)?;
                            write_admin_name(path, &key, Some(name)).map(drop)
                        })
                    })
                    .map_err(|e| e.to_string())?;
                println!(
                    "{} admin key for /mcp-admin is in {}, and the server was restarted to use it. Read it with `kkt key show --admin`.",
                    if made { "A new" } else { "The" },
                    admin.display()
                );
                Ok(admin)
            }
            setup::Mode::Client => Err("an admin key is made on the server's machine".into()),
        }
    }

    fn check_lanes(&self, folder: &Path, purpose: setup::Purpose) -> Result<usize, String> {
        let max_lane_chars = match purpose {
            // The service runs on the config file `kkt serve` finds here.
            setup::Purpose::Service => local_lane_limit(self.env)?,
            setup::Purpose::Docker => docker_lane_limit(self.env)?,
            setup::Purpose::Client => {
                return Err("lanes are imported on the server's machine".into());
            }
        };
        check_lanes_folder(folder, max_lane_chars)
    }

    fn import_lanes(&self, record: &setup::Record, folder: &Path) -> Result<String, String> {
        // `check_lanes` warned of the skipped files before setup began.
        let read = lane_files::read_folder(&lane_files::OsDisk, folder)?;
        let report = match &record.mode {
            setup::Mode::Docker(install) => {
                docker_import_lanes(&docker::Folder(install.folder.clone()), read, false)
            }
            // The service runs on the config file and database `kkt serve`
            // finds by default.
            setup::Mode::Service { .. } => tokio::runtime::Handle::current()
                .block_on(local_import_lanes(self.env, None, None, read, false, None)),
            setup::Mode::Client => Err("lanes are imported on the server's machine".into()),
        }?;
        Ok(report.trim_end().to_string())
    }
}

impl setup::SwitchSteps for OsSteps<'_> {
    fn connected_clients(&self) -> Result<Vec<Client>, String> {
        let mut connected = Vec::new();
        for client in Client::ALL {
            if client.connected(self.env)? {
                connected.push(client);
            }
        }
        Ok(connected)
    }

    fn recorded(&self) -> Result<Option<setup::Record>, String> {
        read_setup(self.env)
    }

    fn write_record(&self, record: &setup::Record) -> Result<(), String> {
        write_setup_record(self.env, record)
    }

    fn connect(&self, client: Client, record: &setup::Record) -> Result<String, String> {
        let executable = kkt_path()?;
        let client_config = client.file(self.env)?;
        let server = record.for_clients();
        let setup = Setup {
            env: self.env,
            url: &server.url,
            executable: &executable,
            key_file: server.key_file.as_deref(),
            client_config: &client_config,
            source: client.source(self.env),
        };
        clients::configure(client, &setup, true)
    }

    fn check_address(&self, url: &str) -> Result<(), String> {
        let mut checks =
            tokio::runtime::Handle::current().block_on(diagnostics::inspect_address(url))?;
        point_server_fix_at(
            &mut checks,
            "check the address, and that the server is running on its machine",
        );
        only_problems(checks)
    }

    fn check_key(&self, url: &str, key: &setup::ClientKey) -> Result<(), String> {
        let runtime = tokio::runtime::Handle::current();
        let checks = match key {
            setup::ClientKey::None => runtime.block_on(diagnostics::inspect_with_key(url, None)),
            setup::ClientKey::Typed(key) => {
                runtime.block_on(diagnostics::inspect_with_key(url, Some(key)))
            }
            setup::ClientKey::File(path) => runtime.block_on(diagnostics::inspect(url, Some(path))),
        }?;
        only_problems(checks)
    }
}

impl setup::AdoptSteps for OsSteps<'_> {
    fn unrecorded(&self) -> Result<Option<setup::Unrecorded>, String> {
        if read_setup(self.env)?.is_some() || read_unfinished(self.env)?.is_some() {
            return Ok(None);
        }
        read_unrecorded(self.env, self.serve_args).map_err(|(_, e)| e)
    }
}

/// The doctor's lines for the problems in `checks` as an error, if any.
fn only_problems(checks: Vec<diagnostics::Check>) -> Result<(), String> {
    let problems: Vec<_> = checks.into_iter().filter(|check| !check.ok).collect();
    if problems.is_empty() {
        return Ok(());
    }
    Err(doctor_report(problems).0.trim_end().to_string())
}

/// What setup says while it waits for the server at `url`, set up as `mode`,
/// to answer.
fn waiting_note(url: &str, mode: &setup::Mode) -> String {
    let tailscale = match mode {
        setup::Mode::Docker(install) => install.tailscale,
        setup::Mode::Service { tailscale } => *tailscale,
        setup::Mode::Client => false,
    };
    if tailscale {
        format!(
            "Waiting for the server to answer at {url} (a new Tailscale node gets its \
             HTTPS certificate on first use)..."
        )
    } else {
        format!("Waiting for the server to answer at {url}...")
    }
}

/// Runs doctor's connection checks on `url`, again every
/// `SETUP_SERVER_POLL` while the server doesn't answer at all, for up to
/// `wait` on `clock`, saying `note` once if it waits. Returns the last
/// checks.
async fn inspect_until_reachable(
    url: &str,
    key_file: Option<&Path>,
    note: &str,
    clock: &dyn Clock,
    wait: std::time::Duration,
) -> Result<Vec<diagnostics::Check>, String> {
    let end = clock.now() + chrono::Duration::from_std(wait).unwrap_or(chrono::Duration::MAX);
    let every =
        chrono::Duration::from_std(config::SETUP_SERVER_POLL).unwrap_or(chrono::Duration::MAX);
    let mut said = false;
    loop {
        let began = clock.now();
        let checks = diagnostics::inspect(url, key_file).await?;
        if !diagnostics::unreachable(&checks) || began >= end {
            return Ok(checks);
        }
        if !said {
            println!("{note}");
            said = true;
        }
        clock.sleep_until(began + every).await;
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

/// The database file `kkt serve` would use given the config file's `db`,
/// unless `db` names another. It must exist.
fn existing_db(
    env: &Env,
    db: Option<PathBuf>,
    config_db: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let path = match db.or(config_db) {
        Some(path) => path,
        None => env.default_db(Platform::CURRENT).ok_or(
            "no home directory to find the database in: set HOME or pass --db".to_string(),
        )?,
    };
    if !path.exists() {
        return Err(format!(
            "no database at {}; pass --db with the server's database file",
            path.display()
        ));
    }
    Ok(path)
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

/// The operator's commands on the local database `kkt serve` would use,
/// unless `db` names another, and the server's `max_lane_chars` from the
/// config file.
fn local_operator(
    env: &Env,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(Operator, usize), String> {
    let (_, from_file) = read_config(env, config)?;
    let max_lane_chars = from_file
        .max_lane_chars
        .unwrap_or(Limits::DEFAULT.max_lane_chars);
    let path = existing_db(env, db, from_file.db)?;
    let db = Db::open(&path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    Ok((Operator::new(db, Arc::new(SystemClock)), max_lane_chars))
}

/// Imports the lanes `read` from a folder into the local database. The
/// source recorded is `source` if given, else the folder and its git
/// commit. Returns the report to print.
async fn local_import_lanes(
    env: &Env,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
    read: lane_files::Read,
    retire_missing: bool,
    source: Option<LaneSource>,
) -> Result<String, String> {
    let (operator, max_lane_chars) = local_operator(env, db, config)?;
    let skipped_files = read.skipped_names();
    let source = source.unwrap_or_else(|| LaneSource {
        folder: read.folder.display().to_string(),
        commit: read.commit,
    });
    let lanes = lane_saves(read.lanes);
    let done = operator
        .import_lanes(LaneImport {
            lanes,
            source: source.clone(),
            retire_missing,
            skipped_files,
            max_lane_chars,
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(import_report(&done, &source))
}

/// Reads the lane folder for `kkt lanes import`, printing a warning on
/// standard error for each file it skips before anything is imported. With
/// `retire_missing`, a folder that could only retire every lane is refused.
fn read_lanes_warning(folder: &Path, retire_missing: bool) -> Result<lane_files::Read, String> {
    let read = lane_files::read_folder(&lane_files::OsDisk, folder)?;
    warn_skipped(&read.skipped);
    if retire_missing {
        kkt_rules::check_retire_missing(
            &read.folder.display().to_string(),
            read.lanes.len(),
            read.skipped.len(),
        )
        .map_err(|e| e.message)?;
    }
    Ok(read)
}

/// One warning line on standard error for each file a lane folder's read
/// skipped.
fn warn_skipped(skipped: &[lane_files::Skipped]) {
    for file in skipped {
        eprintln!("warning: {file}");
    }
}

/// Lane files as the saves an import makes.
fn lane_saves(files: Vec<lane_files::LaneFile>) -> Vec<LaneSave> {
    files
        .into_iter()
        .map(|l| LaneSave {
            name: l.name,
            description: l.description,
            text: l.text,
        })
        .collect()
}

/// The `max_lane_chars` of a server on this machine run on the config file
/// `kkt serve` finds here: its setting, else the default.
fn local_lane_limit(env: &Env) -> Result<usize, String> {
    let (_, from_file) = read_config(env, None)?;
    Ok(from_file
        .max_lane_chars
        .unwrap_or(Limits::DEFAULT.max_lane_chars))
}

/// The `max_lane_chars` of kkt in Docker set up in the folder `kkt setup`
/// uses: the container's own setting when the folder holds an install, and
/// the default for a new one, whose container starts with no config file.
/// The host's config file plays no part; the container never reads it.
fn docker_lane_limit(env: &Env) -> Result<usize, String> {
    let folder = docker_folder(env)?;
    if !folder.join(docker::ENV_FILE).exists() {
        return Ok(Limits::DEFAULT.max_lane_chars);
    }
    docker::lane_limit(&docker::OsHost, &docker::Folder(folder))
        .map_err(|e| format!("could not read the lane text limit of kkt in Docker: {e}"))
}

/// Reads the lane files in `folder` and checks them as `kkt lanes import`
/// would on a server whose limit is `max_lane_chars`, changing nothing: the
/// files' format, then each lane by the lane rules. Warns of each file it
/// skips, as the import would. Returns how many lanes there are; an error
/// names the file or lane that is wrong.
fn check_lanes_folder(folder: &Path, max_lane_chars: usize) -> Result<usize, String> {
    let read = lane_files::read_folder(&lane_files::OsDisk, folder)?;
    warn_skipped(&read.skipped);
    let count = read.lanes.len();
    kkt_rules::check_lane_import(&lane_saves(read.lanes), max_lane_chars)
        .map_err(|e| format!("{}: {}", read.folder.display(), e.message))?;
    Ok(count)
}

/// Imports the lanes `read` from a folder on the host into kkt in Docker
/// in `folder`. The files were read here first, so a mistake is named by
/// its path on the host; [`lane_files::Read::files_to_copy`] says which go
/// in. Returns the report to print.
fn docker_import_lanes(
    folder: &docker::Folder,
    read: lane_files::Read,
    retire_missing: bool,
) -> Result<String, String> {
    let import = docker::LanesImport {
        files: read.files_to_copy(retire_missing),
        source_folder: read.folder.display().to_string(),
        source_commit: read.commit,
        retire_missing,
    };
    docker::import_lanes(&docker::OsHost, folder, &import).map_err(|e| e.to_string())
}

/// One line for each lane an import touched, then one saying where from and
/// how many of each.
fn import_report(done: &[ImportedLane], source: &LaneSource) -> String {
    let mut lines = String::new();
    let (mut created, mut updated, mut unchanged, mut retired, mut skipped, mut kept) =
        (0, 0, 0, 0, 0, 0);
    for lane in done {
        let name = &lane.name;
        lines.push_str(&match &lane.outcome {
            LaneOutcome::Created => {
                created += 1;
                format!("created {name} (version 1)\n")
            }
            LaneOutcome::Updated(version) => {
                updated += 1;
                format!("updated {name} (version {version})\n")
            }
            LaneOutcome::Unchanged(version) => {
                unchanged += 1;
                format!("unchanged {name} (version {version})\n")
            }
            LaneOutcome::RetiredSkipped => {
                skipped += 1;
                format!("left {name} alone: it is retired\n")
            }
            LaneOutcome::Retired => {
                retired += 1;
                format!("retired {name}: it has no file in the folder\n")
            }
            LaneOutcome::FileSkipped => {
                kept += 1;
                format!("left {name} alone: its file was skipped, so it is not retired\n")
            }
        });
    }
    let commit = match &source.commit {
        Some(commit) => format!(" (git commit {commit})"),
        None => String::new(),
    };
    lines.push_str(&format!(
        "Imported from {}{commit}: {created} created, {updated} updated, {unchanged} unchanged",
        source.folder
    ));
    if skipped > 0 {
        lines.push_str(&format!(", {skipped} retired left alone"));
    }
    if retired > 0 {
        lines.push_str(&format!(", {retired} retired"));
    }
    if kept > 0 {
        lines.push_str(&format!(", {kept} left alone for a skipped file"));
    }
    lines.push('\n');
    lines
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

#[cfg(test)]
mod setup_and_watch_output_tests {
    use super::*;

    fn docker(tailscale: bool) -> setup::Mode {
        setup::Mode::Docker(setup::DockerSetup {
            folder: PathBuf::from("/data/kkt docker"),
            tailscale,
            hostname: None,
            tailnet: None,
        })
    }

    #[test]
    fn the_fix_for_a_silent_server_names_how_this_machine_runs_kkt() {
        for tailscale in [false, true] {
            let fix = server_fix(&docker(tailscale)).expect("a Docker fix");
            assert!(
                fix.contains("`cd '/data/kkt docker' && docker compose up -d`"),
                "{fix}"
            );
            assert!(!fix.contains("kkt service"), "{fix}");
            assert!(!fix.contains("kkt serve"), "{fix}");
        }
        let fix = server_fix(&setup::Mode::Client).expect("a client fix");
        assert!(fix.contains("another machine"), "{fix}");
        // A service machine keeps the service fix.
        for tailscale in [false, true] {
            assert_eq!(server_fix(&setup::Mode::Service { tailscale }), None);
        }
    }

    #[test]
    fn the_wait_names_tailscale_only_when_the_server_is_shared_over_it() {
        let url = "http://127.0.0.1:8080/mcp";
        for (mode, shared) in [
            (docker(false), false),
            (docker(true), true),
            (setup::Mode::Service { tailscale: false }, false),
            (setup::Mode::Service { tailscale: true }, true),
        ] {
            let note = waiting_note(url, &mode);
            assert!(
                note.starts_with(&format!("Waiting for the server to answer at {url}")),
                "{note}"
            );
            assert_eq!(note.contains("Tailscale"), shared, "{note}");
        }
    }
}
