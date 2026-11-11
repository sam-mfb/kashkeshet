//! `kkt-keymode`: kkt's key mode assembled a second time from the shared
//! crates, with none of the `kkt` program's code. It exists to show that
//! the crates are a core and `kkt` one product on it, and the end-to-end
//! tests run against it in CI (`KKT_TEST_PROGRAM=kkt-keymode`). It is for
//! development only and never released.
//!
//! It reads the same config file as `kkt serve` through kkt-config-file and
//! kkt-settings, checks the same key files through kkt-admission, and
//! builds the server through kkt-server with kkt-identity's `KeyMode`. Unlike `kkt serve` it makes no
//! key, no backup and no log file, adopts no database from before kkt-db,
//! and needs `db` and either `access_key_file` or `no_access_key` set.
//!
//! Usage: `kkt-keymode --config <file> [--bind <address>]`. It prints
//! `bound to <address>` once it listens, and stops on SIGINT or SIGTERM.

use kkt_admission::AccessKey;
use kkt_admission::AdminAdmission;
use kkt_admission::server_keys::{self, KeyRefusal, KeyText, KeyTexts, ServerKeys};
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_config_file_os as config_file_os;
use kkt_db_sqlite::{Options, SqlitePlace};
use kkt_entropy_os::OsEntropy;
use kkt_hosts::AllowedHosts;
use kkt_identity_keymode::KeyMode;
use kkt_logs::{LogLevel, LogPolicy};
use kkt_provenance::{Build, RunSettings, ServerRun};
use kkt_rules::Db;
use kkt_server::{self as app, AppConfig, Parts};
use kkt_settings::{InvalidSetting, Limits, RecordedSettings, ServeOptions, ServeSettings};
use kkt_shutdown::Shutdown;
use kkt_shutdown_os::StopSignals;
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
            eprintln!("kkt-keymode: {e}");
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
    let ServeSettings {
        db,
        bind,
        allow_any_bind,
        limits,
        allow_origins,
        allow_hosts,
        instructions,
        access_key,
        web_key,
        admin_key,
        log_file: _,
        log_level,
        dependency_log_level,
    } = settings;
    kkt_hosts::check_bind(bind, allow_any_bind, access_key != AccessKey::None).map_err(invalid)?;
    limits.check().map_err(invalid)?;
    let allowed_hosts = AllowedHosts::new(&allow_hosts).map_err(invalid)?;
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
        rules: rules_settings(&limits),
        web: web_settings(&limits),
        mcp: kkt_mcp::Settings::DEFAULT,
        sessions: kkt_admission::Settings {
            web_session_lifetime: limits.web_session_lifetime,
        },
        allowed_origins: allow_origins,
        allowed_hosts,
        instructions,
    };

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
    let run_id = app::record_run(&db, &run, clock.now())
        .await
        .map_err(|e| format!("could not record this server run: {e}"))?;
    let shutdown = Shutdown::new();
    let signals =
        StopSignals::listen().map_err(|e| format!("could not listen for stop signals: {e}"))?;
    // Key mode: each call is the avatar its token names, or the operator
    // the admin key makes, in the one swarm.
    let parts = Parts::new(
        db,
        clock,
        Arc::new(OsEntropy),
        Arc::new(KeyMode::new(admin_name)),
        Arc::new(kkt_logs_os::TracingLog),
        &kkt_mcp_formatter::Settings {
            max_response_bytes: limits.max_response_bytes,
        },
        &kkt_events::Settings::DEFAULT,
        run_id,
    );
    let server = app::build(parts, config, shutdown.clone());
    let listening = kkt_server_listen::listen(server, bind)
        .await
        .map_err(|e| format!("could not listen on {bind}: {e}"))?;
    let bound = listening
        .address()
        .map_err(|e| format!("could not read the address listened on: {e}"))?;
    println!(
        "kkt-keymode {} (run {run_id}) bound to {bound}",
        run.version
    );
    listening
        .serve_until(
            async move {
                match signals.stop_on_signal(shutdown).await {
                    Ok(signal) => println!("received {signal}; stopping"),
                    Err(e) => eprintln!("kkt-keymode: could not listen for stop signals: {e}"),
                }
            },
            |e| eprintln!("kkt-keymode: could not set TCP_NODELAY on a connection: {e}"),
        )
        .await
        .map_err(|e| format!("server error: {e}"))
}

/// The config file's settings, with `--bind` over them, and the defaults
/// for the rest. There is no default database or key file.
fn settings(args: &Args) -> Result<ServeSettings, String> {
    let text = config_file_os::read_named(&args.config).map_err(|e| e.to_string())?;
    let dir = args.config.parent().unwrap_or(Path::new("."));
    let from_file = parse_config(&text, dir)
        .map_err(|e| format!("invalid setting in {}: {e}", args.config.display()))?;
    let flags = ServeOptions {
        bind: args.bind,
        ..ServeOptions::NONE
    };
    let options = flags.over(from_file);
    let db = options
        .db
        .clone()
        .ok_or_else(|| format!("{} must set db", args.config.display()))?;
    options
        .resolve(db, None)
        .map_err(|e| format!("invalid setting: {e}"))
}

/// The settings in the config file's `text`, as kkt-settings takes them:
/// the file parsed by kkt-config-file, with kkt-logs' log levels.
/// There is no home folder, so a `~/` path is refused.
fn parse_config(text: &str, dir: &Path) -> Result<ServeOptions, InvalidSetting> {
    let level = |name, text: &str| LogLevel::parse(name, text).map_err(|e| e.rule);
    let config = kkt_config_file::parse_file(text, dir, None, level)
        .map_err(|e| InvalidSetting::new(e.name, e.rule))?;
    Ok(ServeOptions {
        log_level: config.log_level,
        dependency_log_level: config.dependency_log_level,
        name_pack: config.name_pack,
        db: config.db,
        bind: config.bind,
        allow_any_bind: config.allow_any_bind,
        max_response_bytes: config.max_response_bytes,
        default_limit: config.default_limit,
        max_limit: config.max_limit,
        sends_per_minute: config.sends_per_minute,
        waiting_grace_seconds: config.waiting_grace_seconds,
        upgrade_backups_kept: config.upgrade_backups_kept,
        max_lane_chars: config.max_lane_chars,
        max_memory_entry_chars: config.max_memory_entry_chars,
        max_memory_chars: config.max_memory_chars,
        allow_origins: config.allow_origins,
        allow_hosts: config.allow_hosts,
        instructions: config.instructions,
        access_key_file: config.access_key_file,
        no_access_key: config.no_access_key,
        log_file: config.log_file,
    })
}

/// The database at `path`, made if it does not exist, at the core's latest
/// schema.
/// The rules' share of `limits`. The longest lane description is not an
/// operator setting, so it keeps the rules' default.
fn rules_settings(limits: &Limits) -> kkt_rules::Settings {
    kkt_rules::Settings {
        name_pack: limits.name_pack,
        default_limit: limits.default_limit,
        max_limit: limits.max_limit,
        max_body_bytes: limits.max_body_bytes,
        max_channel_name_chars: limits.max_channel_name_chars,
        max_purpose_chars: limits.max_purpose_chars,
        max_description_chars: limits.max_description_chars,
        max_note_chars: limits.max_note_chars,
        max_server_announcement_chars: limits.max_server_announcement_chars,
        max_memory_entry_chars: limits.max_memory_entry_chars,
        max_memory_chars: limits.max_memory_chars,
        max_lane_chars: limits.max_lane_chars,
        max_lane_description_chars: kkt_rules::Settings::DEFAULT.max_lane_description_chars,
        sends_per_minute: limits.sends_per_minute,
        registrations_per_minute: limits.registrations_per_minute,
        max_agents: limits.max_agents,
        default_wait: limits.default_wait,
        max_wait: limits.max_wait,
        waiting_grace: limits.waiting_grace,
        member_preview: limits.member_preview,
        purpose_preview_chars: limits.purpose_preview_chars,
    }
}

/// The web page's share of `limits`.
fn web_settings(limits: &Limits) -> kkt_web::Settings {
    kkt_web::Settings {
        page_messages: limits.web_page_messages,
        member_excerpt_chars: limits.web_member_excerpt_chars,
        keep_alive: kkt_web::Settings::DEFAULT.keep_alive,
    }
}

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
    fn web(&mut self) -> Result<Option<KeyText>, String> {
        self.0.web().map_err(|e| e.to_string())
    }
    fn admin(&mut self) -> Result<Option<KeyText>, String> {
        self.0.admin().map_err(|e| e.to_string())
    }
    fn admin_names(&mut self) -> Result<Option<String>, String> {
        self.0.admin_names().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_parts_defaults_are_the_gathered_defaults() {
        assert_eq!(
            rules_settings(&Limits::DEFAULT),
            kkt_rules::Settings::DEFAULT
        );
        assert_eq!(web_settings(&Limits::DEFAULT), kkt_web::Settings::DEFAULT);
    }
}
