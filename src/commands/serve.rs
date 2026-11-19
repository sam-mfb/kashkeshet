//! `kkt serve`: resolves the server's settings, starts its log, and runs
//! the server, through `kkt-server`, until it is told to stop.

use crate::machine::database::{OpenError, PendingError};
use crate::machine::keys::make_default_key;
use crate::machine::server_run::{RecordedSettings, new_server_run};
use crate::machine::settings::{
    Checked, DbHome, DefaultDb, LogFileError, Report, SettingsError, Sources, check_settings,
    load_serve_options, resolve,
};
use crate::machine::settings::{ServeOptions, ServeSettings};
use kkt_api_agent::SwarmId;
use kkt_backup::{self as backup, Backups};
use kkt_backup_os::OsBackups;
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_entropy_os::OsEntropy;
use kkt_identity_keymode::KeyMode;
use kkt_log::{Level, Log};
use kkt_log_tracing::TracingLog;
use kkt_log_tracing::init_logging;
use kkt_machine::{Env, Platform};
use kkt_provenance::{Build, RunSettings, ServerRun};
use kkt_rolling_file::{LogPolicy, open_log_file};
use kkt_server::{self as app, Parts};
use kkt_shutdown::Shutdown;
use kkt_shutdown::Signals;
use kkt_shutdown_os::StopSignals;
use kkt_terminal::Terminal;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Why `kkt serve` stopped or did not start: a source's own error, or the
/// step here that failed.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// A default key file could not be made.
    #[error(transparent)]
    Key(#[from] kkt_keys::KeyFileError),
    #[error(transparent)]
    Database(#[from] OpenError),
    #[error(transparent)]
    Pending(#[from] PendingError),
    #[error(transparent)]
    Log(#[from] LogFileError),
    #[error(transparent)]
    Logging(kkt_log_tracing::LoggingError),
    #[error("could not record this server run: {0}")]
    RecordRun(#[source] kkt_rules::StoreError),
    #[error("could not listen for shutdown signals: {0}")]
    Signals(#[source] std::io::Error),
    #[error("could not listen on {bind}: {source}")]
    Listen {
        bind: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read the address listened on: {0}")]
    Address(#[source] std::io::Error),
    #[error("server error: {0}")]
    Server(#[source] std::io::Error),
    /// Where the backups before an upgrade go could not be worked out.
    #[error(transparent)]
    BackupFolder(#[from] kkt_backup::FolderError),
    /// The backup before an upgrade failed, so nothing was changed.
    #[error(
        "the database needs upgrading (schema {from} to {to}), and kkt backs it up first, but \
         the backup failed, so the server did not start and the database is unchanged: {source}"
    )]
    BackupFirst {
        from: String,
        to: String,
        #[source]
        source: kkt_backup::BackupError,
    },
}

/// `kkt serve`: flags over the config file over the defaults, with the
/// database in the per-user data directory unless one is given.
pub async fn run(
    env: &Env,
    platform: Platform,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<(), ServeError> {
    let loaded = load_serve_options(env, config, flags)?;
    let sources = loaded.sources();
    let settings = resolve(
        env,
        platform,
        loaded.flags.over(loaded.from_file),
        DefaultDb::Made(DbHome::Keep),
        Report::Traced(&sources),
    )?;
    // Before logging starts at that level, not after.
    crate::machine::settings::check_dependency_log_level(settings.dependency_log_level)
        .map_err(|e| SettingsError::Invalid(sources.invalid(e)))?;
    let log_file = settings.log_file.clone();
    // No setting changes it: every log file is kept within the default.
    let log_policy = LogPolicy::DEFAULT;
    let writer = match log_file.as_deref() {
        Some(path) => Some(Box::new(
            open_log_file(path, log_policy, Arc::new(SystemClock)).map_err(LogFileError)?,
        ) as Box<dyn std::io::Write + Send>),
        None => None,
    };
    init_logging(
        writer,
        crate::machine::settings::LOG_OWN_TARGETS,
        settings.log_level,
        settings.dependency_log_level,
    )
    .map_err(ServeError::Logging)?;
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
) -> Result<(), ServeError> {
    make_default_key(&settings.access_key, "access")?;
    make_default_key(&settings.web_key, "web")?;
    // Listing every field, with no `..`, makes a new one fail to compile
    // here until it is used or passed on.
    let Checked {
        db_path,
        bind,
        config,
        server_settings,
        rules,
        backups,
        admin_name,
        allow_any_bind,
        config_file,
        log_file,
        log_level,
        dependency_log_level,
    } = check_settings(settings, sources)?;
    let clock = Arc::new(SystemClock);
    backup_before_upgrade(&db_path, backups.upgrade_backups_kept, clock.as_ref())?;
    let db = crate::machine::database::open(&db_path)?;
    let run = ServerRun::new(
        Build::CURRENT,
        &RunSettings {
            settings: RecordedSettings {
                allow_any_bind,
                server: server_settings.settings(),
                rules: rules.clone(),
                backup: backups,
                log_level,
                dependency_log_level,
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
    let shutdown = Shutdown::new();
    let entropy = Arc::new(OsEntropy);
    let rules = kkt_rules::start(
        new_server_run(&run),
        kkt_rules::Startup {
            db,
            // Each call acts in the swarm its caller is in; key mode has one.
            swarm: SwarmId::main(),
            shutdown: shutdown.clone(),
            clock: clock.clone(),
            entropy: entropy.clone(),
            settings: rules,
            wording: Arc::new(kkt_rules::DefaultWording),
        },
    )
    .await
    .map_err(ServeError::RecordRun)?;
    let run_id = rules.run_id();
    let signals = StopSignals::listen().map_err(ServeError::Signals)?;
    // Key mode: each call is the avatar its token names, or the operator
    // the admin key makes, in the one swarm.
    let parts = Parts::new(
        Arc::new(rules),
        clock,
        entropy,
        Arc::new(KeyMode::new(admin_name)),
        Arc::new(kkt_log_tracing::TracingLog),
        server_settings,
    );
    let server = app::build(parts, config, shutdown.clone());
    let listening = kkt_listen::listen(server.into_router(), bind)
        .await
        .map_err(|source| ServeError::Listen { bind, source })?;
    TracingLog.record(Level::Info, module_path!(), format_args!(
        "kashkeshet {} (commit {}, run {run_id}) listening: web page http://{bind}/ , MCP http://{bind}/mcp ({access}){admin}, database {}{settings_from}",
        run.version,
        run.commit,
        db_path.display()), &[]);
    // The address as bound, which names the port the system chose when the
    // port asked for was 0, on a line of its own.
    let bound = listening.address().map_err(ServeError::Address)?;
    TracingLog.record(
        Level::Info,
        module_path!(),
        format_args!("bound to {bound}"),
        &[],
    );
    listening
        .serve_until(
            async move {
                match signals.stop_on_signal(shutdown).await {
                    Ok(signal) => TracingLog.record(
                        Level::Info,
                        module_path!(),
                        format_args!("received {signal}; shutting down"),
                        &[],
                    ),
                    Err(e) => TracingLog.record(
                        Level::Error,
                        module_path!(),
                        format_args!("could not listen for shutdown signals: {e}"),
                        &[],
                    ),
                }
            },
            |e| {
                TracingLog.record(
                    Level::Warn,
                    module_path!(),
                    format_args!(
                        "could not turn off Nagle's algorithm (TCP_NODELAY) on a connection, \
                     so its MCP replies may each take about 40 ms longer: {e}"
                    ),
                    &[],
                )
            },
        )
        .await
        .map_err(ServeError::Server)
}

/// Writes a backup of `contents` into `folder`, copying the database with
/// SQLite's online backup.
fn write_backup(
    folder: &Path,
    clock: &dyn Clock,
    contents: &backup::Contents,
) -> Result<PathBuf, kkt_backup::BackupError> {
    let layout = backup::layout(clock.now(), Build::CURRENT.version, contents);
    OsBackups::new(crate::machine::database::copy).write(folder, &layout)
}

/// Before the server upgrades the tables in `database`, backs it up beside
/// it and prunes those backups to the newest `keep`. Only the database: an
/// upgrade changes nothing else, and a copy of the key in the data folder
/// would be one more place it could leak from. Any failure to back up stops
/// the server before the database is changed.
fn backup_before_upgrade(
    database: &Path,
    keep: usize,
    clock: &dyn Clock,
) -> Result<(), ServeError> {
    let Some(upgrade) = crate::machine::database::pending_upgrade(database)? else {
        return Ok(());
    };
    let folder = backup::upgrade_backup_folder(database)?;
    let contents = backup::Contents {
        database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let written =
        write_backup(&folder, clock, &contents).map_err(|source| ServeError::BackupFirst {
            from: upgrade.from.clone(),
            to: upgrade.to.clone(),
            source,
        })?;
    TracingLog.record(
        Level::Info,
        module_path!(),
        format_args!(
            "backed up the database to {} before upgrading it from schema {} to {}",
            written.display(),
            upgrade.from,
            upgrade.to
        ),
        &[],
    );
    // The backup is made, so the upgrade is safe; an old backup left behind
    // only takes space.
    match OsBackups::new(crate::machine::database::copy).prune(&folder, keep) {
        Ok(removed) => {
            for path in removed {
                TracingLog.record(
                    Level::Info,
                    module_path!(),
                    format_args!("removed the old backup {}", path.display()),
                    &[],
                );
            }
        }
        Err(e) => TracingLog.record(
            Level::Warn,
            module_path!(),
            format_args!("could not remove old backups: {e}"),
            &[],
        ),
    }
    Ok(())
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
        let err = run(&test_env(None), TEST_PLATFORM, None, bad_pack())
            .await
            .map_err(|e| e.to_string());
        assert_eq!(
            err,
            Err("no home directory to keep the database in: set HOME or pass --db".to_string())
        );
    }

    #[tokio::test]
    async fn the_default_database_folder_is_made_before_the_settings_are_checked() {
        let home = scratch::Dir::new("serve-makes-db-folder");
        let env = test_env(Some(&home));
        let folder = kkt_deployment::default_db(&env, TEST_PLATFORM)
            .and_then(|db| db.parent().map(PathBuf::from))
            .expect("a folder");
        let err = run(&env, TEST_PLATFORM, None, bad_pack())
            .await
            .expect_err("bad pack")
            .to_string();
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
            .expect_err("bad pack")
            .to_string();
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(!dir.join("sub").exists());
    }

    /// The startup failures `kkt serve` words itself, word for word, on the
    /// kinds that word them. The failures themselves (no signals, a port in
    /// use, a backup that fails before an upgrade) cannot be made to happen
    /// here; the sentences are the ones the old code formatted.
    #[test]
    fn the_startup_failures_are_pinned() {
        let io = || std::io::Error::other("refused");
        assert_eq!(
            ServeError::RecordRun(kkt_rules::StoreError::Poisoned).to_string(),
            "could not record this server run: database lock poisoned"
        );
        assert_eq!(
            ServeError::Signals(io()).to_string(),
            "could not listen for shutdown signals: refused"
        );
        assert_eq!(
            ServeError::Listen {
                bind: "127.0.0.1:7420".parse().expect("an address"),
                source: io(),
            }
            .to_string(),
            "could not listen on 127.0.0.1:7420: refused"
        );
        assert_eq!(
            ServeError::Address(io()).to_string(),
            "could not read the address listened on: refused"
        );
        assert_eq!(
            ServeError::Server(io()).to_string(),
            "server error: refused"
        );
        assert_eq!(
            ServeError::BackupFirst {
                from: "core 7".into(),
                to: "core 8".into(),
                source: kkt_backup::BackupError::Exists {
                    path: PathBuf::from("/b/kkt-backup"),
                },
            }
            .to_string(),
            "the database needs upgrading (schema core 7 to core 8), and kkt backs it up first, \
             but the backup failed, so the server did not start and the database is unchanged: \
             /b/kkt-backup already exists"
        );
        assert_eq!(
            crate::process::KktPathError(io()).to_string(),
            "could not find kkt's path: refused"
        );
    }
}
