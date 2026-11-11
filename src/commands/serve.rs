//! `kkt serve`: resolves the server's settings, starts its log, and runs
//! the server, through `kkt-server`, until it is told to stop.

use crate::commands::lifecycle::backup_before_upgrade;
use crate::machine::keys::make_default_key;
use crate::machine::settings::{
    Checked, DefaultDb, NO_HOME_TO_KEEP_DB, Report, Sources, check_settings, format_settings,
    load_serve_options, resolve,
};
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_entropy_os::OsEntropy;
use kkt_identity_keymode::KeyMode;
use kkt_logs::{LogPolicy, check_dependency_log_level};
use kkt_logs_os::init_logging;
use kkt_machine::{Env, Platform};
use kkt_provenance::{Build, RunSettings, ServerRun};
use kkt_server::{self as app, Parts};
use kkt_settings::{RecordedSettings, ServeOptions, ServeSettings};
use kkt_shutdown::Shutdown;
use kkt_shutdown_os::StopSignals;
use kkt_terminal_os::Terminal;
use std::path::PathBuf;
use std::sync::Arc;

/// `kkt serve`: flags over the config file over the defaults, with the
/// database in the per-user data directory unless one is given.
pub async fn run(
    env: &Env,
    platform: Platform,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<(), String> {
    let loaded = load_serve_options(env, config, flags)?;
    let sources = loaded.sources();
    let settings = resolve(
        env,
        platform,
        loaded.flags.over(loaded.from_file),
        DefaultDb::Made(NO_HOME_TO_KEEP_DB),
        Report::Traced(&sources),
    )?;
    // Before logging starts at that level, not after.
    check_dependency_log_level(settings.dependency_log_level).map_err(|e| sources.invalid(e))?;
    let log_file = settings.log_file.clone();
    // No setting changes it: every log file is kept within the default.
    let log_policy = LogPolicy::DEFAULT;
    init_logging(
        log_file.as_deref().map(|path| (path, log_policy)),
        settings.log_level,
        settings.dependency_log_level,
        Arc::new(SystemClock),
    )?;
    let served = serve(settings, &sources, log_policy).await;
    if let (Some(file), Err(e)) = (&log_file, &served) {
        // Someone at a terminal reads stderr, not the log file.
        Terminal.errors(format_args!("error: {e} (logged in {})\n", file.display()));
    }
    served
}

/// `log_policy` is the policy the log file, if any, was opened with.
async fn serve(
    settings: ServeSettings,
    sources: &Sources,
    log_policy: LogPolicy,
) -> Result<(), String> {
    make_default_key(&settings.access_key, "access")?;
    make_default_key(&settings.web_key, "web")?;
    // Listing every field, with no `..`, makes a new one fail to compile
    // here until it is used or passed on.
    let Checked {
        db_path,
        bind,
        config,
        limits,
        admin_name,
        allow_any_bind,
        config_file,
        log_file,
        log_level,
        dependency_log_level,
    } = check_settings(settings, sources)?;
    let clock = Arc::new(SystemClock);
    backup_before_upgrade(&db_path, limits.upgrade_backups_kept, clock.as_ref())?;
    let db = crate::machine::database::open(&db_path)?;
    let run = ServerRun::new(
        Build::CURRENT,
        &RunSettings {
            settings: RecordedSettings {
                allow_any_bind,
                limits: limits.clone(),
                log_level,
                dependency_log_level,
                allowed_origins: config.allowed_origins.clone(),
                allowed_hosts: config.allowed_hosts.extra().to_vec(),
                config_file: config_file.clone(),
                access_key_file: config.admission.key_source().map(str::to_string),
                web_key_file: config.web_admission.key_source().map(str::to_string),
                admin_key_file: config
                    .admin_admission
                    .as_ref()
                    .and_then(|admission| admission.key_source())
                    .map(str::to_string),
                admin_name: admin_name.as_ref().map(|name| name.to_string()),
                log_file,
                log_policy,
            }
            .lines(),
            // The run record hashes the base instructions: the server's
            // instructions file, or kkt-mcp's built-in text when there is
            // none. kkt-mcp adds the current server announcement when an
            // agent connects; the announcement is stored with the time it
            // was saved, so the two together show what an agent got.
            instructions: config.agent_instructions().to_string(),
        },
    );
    let access = match config.admission.key_source() {
        Some(file) => format!("access key from {file}"),
        None => "no access key".to_string(),
    };
    let admin = match config
        .admin_admission
        .as_ref()
        .and_then(|admission| admission.key_source())
    {
        Some(source) => format!(
            ", admin MCP http://{bind}{} (admin key from {}, {})",
            kkt_mcp::MCP_ADMIN_PATH,
            source,
            match &admin_name {
                Some(name) => format!("named {name}"),
                None => "no name, so read-only".to_string(),
            }
        ),
        None => String::new(),
    };
    let settings_from = match &config_file {
        Some(file) => format!(", config {file}"),
        None => String::new(),
    };
    let run_id = app::record_run(&db, &run, clock.now())
        .await
        .map_err(|e| format!("could not record this server run: {e}"))?;
    let shutdown = Shutdown::new();
    let signals =
        StopSignals::listen().map_err(|e| format!("could not listen for shutdown signals: {e}"))?;
    // Key mode: each call is the avatar its token names, or the operator
    // the admin key makes, in the one swarm.
    let parts = Parts::new(
        db,
        clock,
        Arc::new(OsEntropy),
        Arc::new(KeyMode::new(admin_name)),
        Arc::new(kkt_logs_os::TracingLog),
        &format_settings(&limits),
        &kkt_events::Settings::DEFAULT,
        run_id,
    );
    let server = app::build(parts, config, shutdown.clone());
    let listening = kkt_server_listen::listen(server, bind)
        .await
        .map_err(|e| format!("could not listen on {bind}: {e}"))?;
    tracing::info!(
        "kashkeshet {} (commit {}, run {run_id}) listening: web page http://{bind}/ , MCP http://{bind}/mcp ({access}){admin}, database {}{settings_from}",
        run.version,
        run.commit,
        db_path.display()
    );
    // The address as bound, which names the port the system chose when the
    // port asked for was 0. A line of its own, so the line above stays as
    // it was.
    let bound = listening
        .address()
        .map_err(|e| format!("could not read the address listened on: {e}"))?;
    tracing::info!("bound to {bound}");
    listening
        .serve_until(
            async move {
                match signals.stop_on_signal(shutdown).await {
                    Ok(signal) => tracing::info!("received {signal}; shutting down"),
                    Err(e) => tracing::error!("could not listen for shutdown signals: {e}"),
                }
            },
            |e| {
                tracing::warn!(
                    "could not turn off Nagle's algorithm (TCP_NODELAY) on a connection, \
                     so its MCP replies may each take about 40 ms longer: {e}"
                )
            },
        )
        .await
        .map_err(|e| format!("server error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{TEST_PLATFORM, test_env};
    use crate::scratch;

    fn bad_pack() -> ServeOptions {
        ServeOptions {
            name_pack: Some("missing".into()),
            ..ServeOptions::NONE
        }
    }

    #[tokio::test]
    async fn without_a_home_or_a_db_serve_refuses_before_other_settings() {
        let err = run(&test_env(None), TEST_PLATFORM, None, bad_pack()).await;
        assert_eq!(
            err,
            Err("no home directory to keep the database in: set HOME or pass --db".to_string())
        );
    }

    #[tokio::test]
    async fn the_default_database_folder_is_made_before_the_settings_are_checked() {
        let home = scratch::Dir::new("serve-makes-db-folder");
        let env = test_env(Some(&home));
        let folder = kkt_setup::default_db(&env, TEST_PLATFORM)
            .and_then(|db| db.parent().map(PathBuf::from))
            .expect("a folder");
        let err = run(&env, TEST_PLATFORM, None, bad_pack())
            .await
            .expect_err("bad pack");
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(folder.is_dir());
    }

    #[tokio::test]
    async fn a_given_database_needs_no_home_and_its_folder_is_not_made() {
        let dir = scratch::Dir::new("serve-given-db");
        let db = dir.join("sub").join("given.db");
        let flags = ServeOptions {
            db: Some(db),
            no_access_key: Some(true),
            ..bad_pack()
        };
        let err = run(&test_env(None), TEST_PLATFORM, None, flags)
            .await
            .expect_err("bad pack");
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(!dir.join("sub").exists());
    }
}
