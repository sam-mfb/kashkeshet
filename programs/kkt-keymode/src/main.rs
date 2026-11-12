//! `kkt-keymode`: kkt's key mode assembled a second time from the shared
//! crates, with none of the `kkt` program's code. It exists to show that
//! the crates are a core and `kkt` one product on it, and the end-to-end
//! tests run against it in CI (`KKT_TEST_PROGRAM=kkt-keymode`). It is for
//! development only and never released.
//!
//! It reads the same config file as `kkt serve` through kkt-config-file, into
//! settings of its own, checks the same key files through kkt-admission, and
//! builds the server through kkt-server with kkt-identity's `KeyMode`. Unlike `kkt serve` it makes no
//! key, no backup and no log file, adopts no database from before kkt-db,
//! and needs `db` and either `access_key_file` or `no_access_key` set.
//!
//! Usage: `kkt-keymode --config <file> [--bind <address>]`. It prints
//! `bound to <address>` once it listens, and stops on SIGINT or SIGTERM.

mod record;
use record::{RecordedSettings, new_server_run};

use kkt_admission::AccessKey;
use kkt_admission::AdminAdmission;
use kkt_admission::server_keys::{self, KeyRefusal, KeyText, KeyTexts, ServerKeys};
use kkt_api_agent::SwarmId;
use kkt_clock_system::SystemClock;
use kkt_config_file_os as config_file_os;
use kkt_db_sqlite::{Options, SqlitePlace};
use kkt_entropy_os::OsEntropy;
use kkt_events::Events;
use kkt_identity_keymode::KeyMode;
use kkt_logs::{LogLevel, LogPolicy};
use kkt_provenance::{Build, RunSettings, ServerRun};
use kkt_rules::Db;
use kkt_server::{self as app, AppConfig, Parts, ServerSettings};
use kkt_shutdown::Shutdown;
use kkt_shutdown_os::StopSignals;
use kkt_terminal_os::Terminal;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            Terminal.errors(format_args!("kkt-keymode: {e}\n"));
            ExitCode::FAILURE
        }
    }
}

/// The command line: the config file, and the address if given there.
struct Args {
    config: PathBuf,
    bind: Option<SocketAddr>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    const USAGE: &str = "usage: kkt-keymode --config <file> [--bind <address>]";
    let (mut config, mut bind) = (None, None);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or(USAGE)?;
        match arg.as_str() {
            "--config" => config = Some(PathBuf::from(value)),
            "--bind" => bind = Some(value.parse().map_err(|e| format!("--bind {value}: {e}"))?),
            _ => return Err(USAGE.into()),
        }
    }
    Ok(Args {
        config: config.ok_or(USAGE)?,
        bind,
    })
}

async fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;
    let settings = settings(&args)?;
    let config_file = args.config.display().to_string();
    let Settings {
        db,
        bind,
        allow_any_bind,
        rules,
        formatter,
        web,
        backups,
        allow_origins,
        allow_hosts,
        instructions,
        access_key,
        web_key,
        admin_key,
        log_level,
        dependency_log_level,
    } = settings;
    let server_settings = ServerSettings {
        formatter,
        web,
        mcp: kkt_mcp::Settings::DEFAULT,
        allowed_origins: allow_origins,
        allowed_hosts: allow_hosts,
    };
    let mut problems: Vec<_> = rules
        .problems()
        .into_iter()
        .map(|p| (p.name, p.check, p.rule))
        .collect();
    problems.extend(
        backups
            .problems()
            .into_iter()
            .map(|p| (p.name, p.check, p.rule)),
    );
    if let Err(p) = kkt_hosts::check_bind(bind, allow_any_bind, access_key != AccessKey::None) {
        problems.push((p.name, "check_bind", p.rule));
    }
    let server_settings = server_settings.check(problems).map_err(invalid)?;
    let instructions = instructions
        .as_deref()
        .map(|path| config_file_os::read_instructions(path).map_err(|e| e.to_string()))
        .transpose()?;
    let ServerKeys {
        admission,
        web_admission,
        admin_key,
        admin_name,
    } = server_keys::check(&mut KeyFiles(kkt_admission_os::KeyFiles::new(
        key_path(&access_key),
        key_path(&web_key),
        admin_key.as_deref(),
    )))
    .map_err(refusal)?;
    let config = AppConfig {
        admission,
        web_admission,
        admin_admission: admin_key.map(AdminAdmission::SharedKey),
        instructions,
    };

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
                config_file: Some(config_file),
                access_key_file: config.admission.key_source().map(str::to_string),
                web_key_file: config.web_admission.key_source().map(str::to_string),
                admin_key_file: config
                    .admin_admission
                    .as_ref()
                    .and_then(|admission| admission.key_source())
                    .map(str::to_string),
                admin_name: admin_name.as_ref().map(|name| name.to_string()),
                log_file: None,
                log_policy: LogPolicy::DEFAULT,
            }
            .lines(),
            instructions: config.agent_instructions().to_string(),
        },
    );
    let clock = Arc::new(SystemClock);
    let db = open(&db)?;
    let shutdown = Shutdown::new();
    let entropy = Arc::new(OsEntropy);
    let rules = kkt_rules::start(
        new_server_run(&run),
        kkt_rules::Startup {
            db,
            // Each call acts in the swarm its caller is in; key mode has one.
            swarm: SwarmId::main(),
            events: Events::new(kkt_events::Settings::DEFAULT).until_stopped(shutdown.clone()),
            clock: clock.clone(),
            entropy: entropy.clone(),
            settings: rules,
            wording: Arc::new(kkt_rules::DefaultWording),
        },
    )
    .await
    .map_err(|e| format!("could not record this server run: {e}"))?;
    let run_id = rules.run_id();
    let signals =
        StopSignals::listen().map_err(|e| format!("could not listen for stop signals: {e}"))?;
    // Key mode: each call is the avatar its token names, or the operator
    // the admin key makes, in the one swarm.
    let parts = Parts::new(
        Arc::new(rules),
        clock,
        entropy,
        Arc::new(KeyMode::new(admin_name)),
        Arc::new(kkt_logs_os::TracingLog),
        server_settings,
    );
    let server = app::build(parts, config, shutdown.clone());
    let listening = kkt_server_listen::listen(server, bind)
        .await
        .map_err(|e| format!("could not listen on {bind}: {e}"))?;
    let bound = listening
        .address()
        .map_err(|e| format!("could not read the address listened on: {e}"))?;
    Terminal.output(format_args!(
        "kkt-keymode {} (run {run_id}) bound to {bound}\n",
        run.version
    ));
    listening
        .serve_until(
            async move {
                match signals.stop_on_signal(shutdown).await {
                    Ok(signal) => Terminal.output(format_args!("received {signal}; stopping\n")),
                    Err(e) => Terminal.errors(format_args!(
                        "kkt-keymode: could not listen for stop signals: {e}\n"
                    )),
                }
            },
            |e| {
                Terminal.errors(format_args!(
                    "kkt-keymode: could not set TCP_NODELAY on a connection: {e}\n"
                ))
            },
        )
        .await
        .map_err(|e| format!("server error: {e}"))
}

/// What kkt-keymode runs with: each owner's settings, from the config file
/// with `--bind` over it, and each owner's defaults for the rest. Its own,
/// smaller surface than `kkt serve`'s: the config file is its only source,
/// and there is no default database or key file.
struct Settings {
    db: PathBuf,
    bind: SocketAddr,
    allow_any_bind: bool,
    rules: kkt_rules::Settings,
    formatter: kkt_mcp_formatter::Settings,
    web: kkt_web::Settings,
    backups: kkt_backup::Settings,
    allow_origins: Vec<String>,
    allow_hosts: Vec<String>,
    instructions: Option<PathBuf>,
    access_key: AccessKey,
    web_key: AccessKey,
    admin_key: Option<PathBuf>,
    log_level: LogLevel,
    dependency_log_level: LogLevel,
}

/// The config file's settings, with `--bind` over them, and the defaults
/// for the rest, refused as `kkt serve` refuses them. A `~/` path is
/// refused: there is no home folder.
fn settings(args: &Args) -> Result<Settings, String> {
    let text = config_file_os::read_named(&args.config).map_err(|e| e.to_string())?;
    let dir = args.config.parent().unwrap_or(Path::new("."));
    let level = |name, text: &str| LogLevel::parse(name, text).map_err(|e| e.rule);
    let config = kkt_config_file::parse_file(&text, dir, None, level).map_err(|e| {
        format!(
            "invalid setting in {}: {} {}",
            args.config.display(),
            e.name,
            e.rule
        )
    })?;
    let invalid = |name: &str, rule: &str| format!("invalid setting: {name} {rule}");
    let db = config
        .db
        .ok_or_else(|| format!("{} must set db", args.config.display()))?;
    let access_key = match (config.no_access_key == Some(true), config.access_key_file) {
        (true, Some(_)) => {
            return Err(invalid(
                "no_access_key",
                "cannot be set together with access_key_file",
            ));
        }
        (true, None) => AccessKey::None,
        (false, Some(path)) => AccessKey::File(path),
        (false, None) => {
            return Err(invalid(
                "access_key_file",
                "has no default without a home directory: set HOME, or give access_key_file or no_access_key",
            ));
        }
    };
    let (web_key, admin_key) = match &access_key {
        AccessKey::None => (AccessKey::None, None),
        AccessKey::File(path) | AccessKey::Default(path) => {
            let web = kkt_admission::web_key_path(path);
            if &web == path {
                return Err(invalid("access_key_file", "must not name the web key file"));
            }
            (AccessKey::Default(web), kkt_admission::admin_key_path(path))
        }
    };
    let defaults = kkt_rules::Settings::DEFAULT;
    let name_pack = match config.name_pack {
        None => defaults.name_pack,
        Some(name) => kkt_names::NamePack::find(&name).ok_or_else(|| {
            let packs: Vec<&str> = kkt_names::PACKS.iter().map(|p| p.name()).collect();
            invalid(
                "name_pack",
                &format!("must be one of: {}", packs.join(", ")),
            )
        })?,
    };
    Ok(Settings {
        db,
        bind: args.bind.or(config.bind).unwrap_or(kkt_hosts::DEFAULT_BIND),
        allow_any_bind: config
            .allow_any_bind
            .unwrap_or(kkt_hosts::DEFAULT_ALLOW_ANY_BIND),
        rules: kkt_rules::Settings {
            name_pack,
            default_limit: config.default_limit.unwrap_or(defaults.default_limit),
            max_limit: config.max_limit.unwrap_or(defaults.max_limit),
            sends_per_minute: config.sends_per_minute.unwrap_or(defaults.sends_per_minute),
            waiting_grace: config
                .waiting_grace_seconds
                .map(|s| std::time::Duration::from_secs(u64::from(s)))
                .unwrap_or(defaults.waiting_grace),
            max_lane_chars: config.max_lane_chars.unwrap_or(defaults.max_lane_chars),
            max_memory_entry_chars: config
                .max_memory_entry_chars
                .unwrap_or(defaults.max_memory_entry_chars),
            max_memory_chars: config.max_memory_chars.unwrap_or(defaults.max_memory_chars),
            ..defaults
        },
        formatter: kkt_mcp_formatter::Settings {
            max_response_bytes: config
                .max_response_bytes
                .unwrap_or(kkt_mcp_formatter::Settings::DEFAULT.max_response_bytes),
        },
        web: kkt_web::Settings::DEFAULT,
        backups: kkt_backup::Settings {
            upgrade_backups_kept: config
                .upgrade_backups_kept
                .unwrap_or(kkt_backup::Settings::DEFAULT.upgrade_backups_kept),
        },
        allow_origins: config.allow_origins.unwrap_or_default(),
        allow_hosts: config.allow_hosts.unwrap_or_default(),
        instructions: config.instructions,
        access_key,
        web_key,
        admin_key,
        log_level: config.log_level.unwrap_or(kkt_logs::DEFAULT_LOG_LEVEL),
        dependency_log_level: config
            .dependency_log_level
            .unwrap_or(kkt_logs::DEFAULT_DEPENDENCY_LOG_LEVEL),
    })
}

/// The database at `path`, made if it does not exist, at the core's latest
/// schema.
fn open(path: &Path) -> Result<Db, String> {
    SqlitePlace::new(path, Options::DEFAULT)
        .open(&[kkt_rules::schema()])
        .map(Db::new)
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

/// The message for a bad setting, whichever part's check refused it.
fn invalid(e: impl std::fmt::Display) -> String {
    format!("invalid setting: {e}")
}

/// Why the server's keys were refused, naming the setting. kkt-keymode
/// words the web and admin keys' refusals its own way, without the config
/// file's setting names; the rest are kkt-admission's wording.
fn refusal(refusal: KeyRefusal<String>) -> String {
    match refusal {
        KeyRefusal::WebKeyReused => {
            "invalid setting: the web key must differ from the access key".into()
        }
        KeyRefusal::WebKey(e) => format!("invalid setting: web key: {}", e.rule),
        KeyRefusal::AdminKeyReused => {
            "invalid setting: the admin key must differ from the access and web keys".into()
        }
        KeyRefusal::AdminKey(e) => format!("invalid setting: admin key: {}", e.rule),
        other => kkt_admission::key_refusal(other, None, None),
    }
}

/// The selected key path, if that key is on.
fn key_path(key: &AccessKey) -> Option<&Path> {
    match key {
        AccessKey::None => None,
        AccessKey::File(path) | AccessKey::Default(path) => Some(path),
    }
}

/// Adapts native key texts, preserving this program's refusal wording.
struct KeyFiles<'a>(kkt_admission_os::KeyFiles<'a>);
impl KeyTexts for KeyFiles<'_> {
    type Error = String;
    fn access(&mut self) -> Result<Option<KeyText>, String> {
        self.0.access().map_err(|e| e.to_string())
    }
    // This program makes no key: the selected web key must already exist.
    fn web(&mut self) -> Result<Option<KeyText>, String> {
        self.0.web().map_err(|e| e.to_string())
    }
    // A missing admin key turns the admin endpoint off.
    fn admin(&mut self) -> Result<Option<KeyText>, String> {
        self.0.admin().map_err(|e| e.to_string())
    }
    fn admin_names(&mut self) -> Result<Option<String>, String> {
        self.0.admin_names().map_err(|e| e.to_string())
    }
}
