//! The settings `kkt serve` would run with: reading the config file, where
//! each setting came from, the one place they are resolved ([`resolve`]),
//! the startup checks, and each part's own settings filled from them.

use crate::machine::keys::read_server_keys;
use kkt_admission::AdminAdmission;
use kkt_admission::server_keys::ServerKeys;
use kkt_config::{self as config, InvalidSetting, LogLevel};
use kkt_hosts::AllowedHosts;
use kkt_names::AdminName;
use kkt_server::AppConfig;
use kkt_settings::{
    self as settings, AccessKey, Env, Limits, Platform, ServeOptions, ServeSettings,
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Reads the arguments a service file gives `kkt` as a `kkt serve` command:
/// the config file it names and the settings it gives. The command line is
/// `cli`'s, so `cli` supplies it ([`cli::serve_args`]).
pub type ServeArgs = fn(&[String]) -> Result<(Option<PathBuf>, ServeOptions), String>;

/// Why a command that keeps or runs a server refuses when no database is
/// given and there is no home directory for the default one.
pub(crate) const NO_HOME_TO_KEEP_DB: &str =
    "no home directory to keep the database in: set HOME or pass --db";
/// Why a command that opens an existing database refuses when none is given
/// and there is no home directory for the default one.
pub(crate) const NO_HOME_TO_FIND_DB: &str =
    "no home directory to find the database in: set HOME or pass --db";

/// What [`database`] does when no database is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DefaultDb {
    /// The per-user default, or an empty path when there is no home
    /// directory: for callers that use the other settings, not the database.
    OrEmpty,
    /// The per-user default; with no home directory, refused with this
    /// message.
    Required(&'static str),
    /// As `Required`, and the default's folder is made if missing, as
    /// `kkt serve` does. A given database's folder is never made.
    Made(&'static str),
}

/// How [`resolve`] says a setting is bad.
#[derive(Clone, Copy)]
pub(crate) enum Report<'a> {
    /// "invalid setting: …", naming the config file when it set the setting.
    Traced(&'a Sources),
    /// The setting and its rule alone, as the key finders say it.
    Bare,
}

/// The database in effect: `given`, else the per-user default as `default`
/// says.
pub(crate) fn database(
    env: &Env,
    given: Option<PathBuf>,
    default: DefaultDb,
) -> Result<PathBuf, String> {
    if let Some(given) = given {
        return Ok(given);
    }
    let found = env.default_db(Platform::CURRENT);
    match default {
        DefaultDb::OrEmpty => Ok(found.unwrap_or_default()),
        DefaultDb::Required(without_home) => found.ok_or_else(|| without_home.to_string()),
        DefaultDb::Made(without_home) => {
            let db = found.ok_or_else(|| without_home.to_string())?;
            if let Some(dir) = db.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
            }
            Ok(db)
        }
    }
}

/// The one place server settings are resolved: `options` (flags already
/// over the config file, or the file alone), the database as `db` says, the
/// default access key file, and the defaults for the rest. The database is
/// worked out first, so its error comes before any other.
pub(crate) fn resolve(
    env: &Env,
    options: ServeOptions,
    db: DefaultDb,
    report: Report,
) -> Result<ServeSettings, String> {
    let db = database(env, options.db.clone(), db)?;
    options
        .resolve(db, env.access_key_file())
        .map_err(|e| match report {
            Report::Traced(sources) => sources.invalid(e),
            Report::Bare => e.to_string(),
        })
}

/// The address `kkt serve` would listen on with the default config file.
pub fn serve_bind(env: &Env) -> Result<SocketAddr, String> {
    Ok(serve_settings(env, None, ServeOptions::NONE)?.bind)
}

/// The settings `kkt serve` would run with, given `config` and `flags`,
/// without making anything. Without a home directory the database is an
/// empty path ([`DefaultDb::OrEmpty`]).
pub(crate) fn serve_settings(
    env: &Env,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<ServeSettings, String> {
    Ok(serve_settings_and_sources(env, config, flags)?.0)
}

/// [`serve_settings`], with where each setting came from.
pub(crate) fn serve_settings_and_sources(
    env: &Env,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<(ServeSettings, Sources), String> {
    let loaded = load_serve_options(env, config, flags)?;
    let sources = loaded.sources();
    let settings = resolve(
        env,
        loaded.flags.over(loaded.from_file),
        DefaultDb::OrEmpty,
        Report::Traced(&sources),
    )?;
    Ok((settings, sources))
}

/// The settings `kkt serve` would run with, given `args` after `kkt`, as a
/// service file passes them.
pub(crate) fn serve_settings_from_args(
    env: &Env,
    serve_args: ServeArgs,
    args: &[String],
) -> Result<ServeSettings, String> {
    let (config, options) = serve_args(args)?;
    serve_settings(env, config, options)
}

/// `settings` as a running server sees them: a default access key file is
/// the same key as that file given by name.
pub(crate) fn as_running(settings: ServeSettings) -> ServeSettings {
    match settings.access_key {
        AccessKey::Default(path) => ServeSettings {
            access_key: AccessKey::File(path),
            ..settings
        },
        _ => settings,
    }
}

/// What `kkt serve` reads before applying defaults: the config file, if
/// any, what it gives, and what the flags give.
pub struct Loaded {
    pub config_file: Option<PathBuf>,
    pub from_file: ServeOptions,
    pub flags: ServeOptions,
}

impl Loaded {
    pub fn sources(&self) -> Sources {
        Sources {
            file: self
                .config_file
                .clone()
                .map(|path| (path, self.from_file.clone())),
            flags: self.flags.clone(),
        }
    }
}

/// Reads the config file `kkt serve` would read, and puts it with `flags`.
pub fn load_serve_options(
    env: &Env,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<Loaded, String> {
    let (config_file, from_file) = read_config(env, config)?;
    Ok(Loaded {
        config_file,
        from_file,
        flags,
    })
}

/// The config file to use (`config`, else the default one if it exists),
/// and what it gives.
pub fn read_config(
    env: &Env,
    config: Option<PathBuf>,
) -> Result<(Option<PathBuf>, ServeOptions), String> {
    let config_file = match config {
        Some(path) => Some(path),
        None => env.config_file().filter(|path| path.exists()),
    };
    let from_file = match &config_file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                format!(
                    "invalid setting: config could not be read from {}: {e}",
                    path.display()
                )
            })?;
            let dir = path.parent().unwrap_or(Path::new("."));
            settings::parse_file(&text, dir, env.home.as_deref())
                .map_err(|e| format!("invalid setting: {e} (in {})", path.display()))?
        }
        None => ServeOptions::NONE,
    };
    Ok((config_file, from_file))
}

/// Where a server's settings came from, so a bad one can be traced to the
/// config file when that is where it was set.
pub struct Sources {
    pub(crate) flags: ServeOptions,
    /// The config file read, and what it gave.
    pub(crate) file: Option<(PathBuf, ServeOptions)>,
}

impl Sources {
    /// The message for a bad setting, naming the config file if the setting
    /// came from there.
    pub fn invalid(&self, e: InvalidSetting) -> String {
        match &self.file {
            Some((path, file)) if file.gives(e.name) && !self.flags.gives(e.name) => {
                format!("invalid setting: {e} (set in {})", path.display())
            }
            _ => format!("invalid setting: {e}"),
        }
    }
}

/// Settings that passed every startup check, with the files they name read.
pub struct Checked {
    pub db_path: PathBuf,
    pub bind: SocketAddr,
    pub config: AppConfig,
    /// Every limit, as gathered: for the formatter, the backups before an
    /// upgrade and the record of the server run. `config` holds the rest's
    /// share.
    pub limits: Limits,
    /// The name tied to the admin key, under which the admin agent's changes
    /// are made and announced. `None`: the admin key may only read.
    pub admin_name: Option<AdminName>,
    // The rest only the record of the server run needs.
    pub allow_any_bind: bool,
    /// The config file the settings were read from, if any.
    pub config_file: Option<String>,
    pub log_file: Option<String>,
    pub log_level: LogLevel,
    pub dependency_log_level: LogLevel,
}

/// Runs the startup checks on `settings` and reads the files they name.
/// The errors name the bad setting, and the config file if it set it.
pub fn check_settings(settings: ServeSettings, sources: &Sources) -> Result<Checked, String> {
    let ServeSettings {
        db: db_path,
        bind,
        allow_any_bind,
        limits,
        allow_origins,
        allow_hosts,
        instructions,
        access_key,
        web_key,
        admin_key,
        log_file,
        log_level,
        dependency_log_level,
    } = settings;
    config::check_bind(bind, allow_any_bind, access_key != AccessKey::None)
        .map_err(|e| sources.invalid(e))?;
    config::check_dependency_log_level(dependency_log_level).map_err(|e| sources.invalid(e))?;
    limits.check().map_err(|e| sources.invalid(e))?;
    let allowed_hosts = AllowedHosts::new(&allow_hosts).map_err(|e| sources.invalid(e))?;
    // None configured: the server sends its built-in instructions.
    let instructions = instructions
        .as_ref()
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))
        })
        .transpose()?;
    let ServerKeys {
        admission,
        web_admission,
        admin_key,
        admin_name,
    } = read_server_keys(&access_key, &web_key, admin_key.as_deref(), sources)?;
    let config = AppConfig {
        admission,
        web_admission,
        // kkt's admin endpoint takes its admin key, as before.
        admin_admission: admin_key.map(AdminAdmission::SharedKey),
        rules: rules_settings(&limits),
        web: web_settings(&limits),
        sessions: session_settings(&limits),
        allowed_origins: allow_origins,
        allowed_hosts,
        instructions,
    };
    Ok(Checked {
        db_path,
        bind,
        config,
        limits,
        admin_name,
        allow_any_bind,
        config_file: sources.file.as_ref().map(|(p, _)| p.display().to_string()),
        log_file: log_file.map(|p| p.display().to_string()),
        log_level,
        dependency_log_level,
    })
}

/// The rules' share of `limits`. The longest lane description is not an
/// operator setting, so it keeps the rules' default.
pub fn rules_settings(limits: &Limits) -> kkt_rules::Settings {
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

/// The formatter's share of `limits`.
pub fn format_settings(limits: &Limits) -> kkt_format::Settings {
    kkt_format::Settings {
        max_response_bytes: limits.max_response_bytes,
    }
}

/// The web page's share of `limits`.
pub fn web_settings(limits: &Limits) -> kkt_web::Settings {
    kkt_web::Settings {
        page_messages: limits.web_page_messages,
        member_excerpt_chars: limits.web_member_excerpt_chars,
    }
}

/// The web sessions' share of `limits`.
pub fn session_settings(limits: &Limits) -> kkt_admission::Settings {
    kkt_admission::Settings {
        web_session_lifetime: limits.web_session_lifetime,
    }
}

/// An environment with only `home` set, for tests.
#[cfg(test)]
pub(crate) fn test_env(home: Option<&Path>) -> Env {
    Env {
        home: home.map(Path::to_path_buf),
        claude_config_dir: None,
        codex_home: None,
        xdg_config_home: None,
        xdg_data_home: None,
    }
}

/// Writes `text` as the default config file under `home`, and returns its
/// path, for tests.
#[cfg(test)]
pub(crate) fn write_default_config(home: &Path, text: &str) -> PathBuf {
    let path = test_env(Some(home)).config_file().expect("a config path");
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("config folder");
    std::fs::write(&path, text).expect("config file");
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch;

    #[test]
    fn each_parts_defaults_are_the_gathered_defaults() {
        // Two sets of defaults: kkt-settings' and each part's own. Filling
        // each part's settings from the gathered defaults must give exactly
        // that part's defaults, so the two cannot drift apart.
        assert_eq!(
            rules_settings(&Limits::DEFAULT),
            kkt_rules::Settings::DEFAULT
        );
        assert_eq!(
            format_settings(&Limits::DEFAULT),
            kkt_format::Settings::DEFAULT
        );
        assert_eq!(web_settings(&Limits::DEFAULT), kkt_web::Settings::DEFAULT);
        assert_eq!(
            session_settings(&Limits::DEFAULT),
            kkt_admission::Settings::DEFAULT
        );
    }

    const NO_HOME_KEY: &str = "invalid setting: access_key_file has no default without a home \
         directory: set HOME, or give access_key_file or no_access_key";

    #[test]
    fn without_a_home_the_database_is_an_empty_path() {
        let dir = scratch::Dir::new("settings-no-home-db");
        let config = dir.join("serve.toml");
        std::fs::write(&config, "no_access_key = true").expect("config");
        let settings =
            serve_settings(&test_env(None), Some(config), ServeOptions::NONE).expect("valid");
        assert_eq!(settings.db, PathBuf::new());
        assert_eq!(settings.access_key, AccessKey::None);
        assert_eq!(settings.bind, config::DEFAULT_BIND);
    }

    #[test]
    fn without_a_home_or_a_key_setting_the_key_has_no_default() {
        let env = test_env(None);
        assert_eq!(
            serve_settings(&env, None, ServeOptions::NONE).map(|s| s.db),
            Err(NO_HOME_KEY.to_string())
        );
        assert_eq!(serve_bind(&env), Err(NO_HOME_KEY.to_string()));
    }

    #[test]
    fn with_a_home_the_defaults_apply_and_flags_win_over_the_file() {
        let home = scratch::Dir::new("settings-home-defaults");
        let env = test_env(Some(&home));
        let settings = serve_settings(&env, None, ServeOptions::NONE).expect("valid");
        assert_eq!(Some(settings.db), env.default_db(Platform::CURRENT));
        assert_eq!(
            settings.access_key,
            AccessKey::Default(env.access_key_file().expect("key path"))
        );
        write_default_config(
            &home,
            "db = \"/data/file.db\"\nbind = \"127.0.0.1:8000\"\nmax_limit = 50",
        );
        let flags = ServeOptions {
            bind: Some("127.0.0.1:9000".parse().expect("addr")),
            ..ServeOptions::NONE
        };
        let settings = serve_settings(&env, None, flags).expect("valid");
        assert_eq!(settings.db, PathBuf::from("/data/file.db"));
        assert_eq!(settings.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(settings.limits.max_limit, 50);
        assert_eq!(
            serve_bind(&env).map(|b| b.to_string()),
            Ok("127.0.0.1:8000".into())
        );
    }

    #[test]
    fn a_bad_setting_names_the_config_file_only_when_it_set_it() {
        let home = scratch::Dir::new("settings-bad-setting");
        let env = test_env(Some(&home));
        let path = write_default_config(&home, "name_pack = \"missing\"");
        let err = serve_settings(&env, None, ServeOptions::NONE).expect_err("bad pack");
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(
            err.ends_with(&format!(" (set in {})", path.display())),
            "{err}"
        );
        let flags = ServeOptions {
            name_pack: Some("also-missing".into()),
            ..ServeOptions::NONE
        };
        let err = serve_settings(&env, None, flags).expect_err("bad pack");
        assert!(!err.contains("(set in"), "{err}");
    }
}
