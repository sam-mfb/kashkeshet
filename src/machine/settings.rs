//! The settings `kkt serve` would run with: reading the config file, where
//! each setting came from, the one place they are resolved ([`resolve`]),
//! the startup checks, and each part's own settings filled from them.

use super::config_disk::ConfigDisk;
use crate::machine::keys::read_server_keys;
pub use crate::machine::serve_options::{BadValue, ServeOptions, ServeSettings, server_settings};
use kkt_admission::AdminAdmission;
use kkt_admission::server_keys::ServerKeys;
use kkt_config_file::Config;
use kkt_config_file::files as config_files;
use kkt_keys::AccessKey;
use kkt_log_tracing::LogLevel;
use kkt_machine::{Env, Platform};
use kkt_names::AdminName;
use kkt_server::{AppConfig, CheckedSettings};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// The targets of kkt's own code, logged at kkt's own level: the library,
/// the `kkt` binary, and every `kkt-*` crate, whose module paths start with
/// "kkt" (check-crates.sh makes every crate name start with `kkt-`).
pub const LOG_OWN_TARGETS: &[&str] = &["kashkeshet", "kkt"];

/// Checks `dependency_log_level`. At debug and trace the MCP library logs
/// whole requests and replies, tokens and recovery codes included, and logs
/// must never hold those; so those levels are refused.
pub fn check_dependency_log_level(level: LogLevel) -> Result<(), kkt_log_tracing::SettingError> {
    match level {
        LogLevel::Error | LogLevel::Warn | LogLevel::Info => Ok(()),
        LogLevel::Debug | LogLevel::Trace => Err(kkt_log_tracing::SettingError::new(
            "dependency_log_level",
            format!(
                "must be error, warn or info, not {}: at debug and trace the MCP library \
                 logs request contents, including tokens",
                level.name()
            ),
        )),
    }
}

/// Why the log file `kkt serve` writes to could not be opened, worded with
/// the settings it comes from.
pub fn log_file_refusal(refusal: kkt_rolling_file::OpenError) -> String {
    use kkt_rolling_file::{OpenError, PolicyValue};
    match refusal {
        OpenError::Policy(problem) => {
            let setting = match problem.value {
                PolicyValue::MaxBytes => "log_max_bytes",
                PolicyValue::RotatedFiles => "log_rotated_files",
                PolicyValue::MaxAge => "log_max_age",
            };
            format!("invalid setting: {setting} {}", problem.rule)
        }
        OpenError::File { path, error } => format!(
            "invalid setting: log_file could not be opened at {}: {error}",
            path.display()
        ),
    }
}

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
    platform: Platform,
    given: Option<PathBuf>,
    default: DefaultDb,
) -> Result<PathBuf, String> {
    if let Some(given) = given {
        return Ok(given);
    }
    let found = kkt_deployment::default_db(env, platform);
    match default {
        DefaultDb::OrEmpty => Ok(found.unwrap_or_default()),
        DefaultDb::Required(without_home) => found.ok_or_else(|| without_home.to_string()),
        DefaultDb::Made(without_home) => {
            let db = found.ok_or_else(|| without_home.to_string())?;
            if let Some(dir) = db.parent() {
                kkt_files::Files
                    .create_dir_all(dir)
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
    platform: Platform,
    options: ServeOptions,
    db: DefaultDb,
    report: Report,
) -> Result<ServeSettings, String> {
    let db = database(env, platform, options.db.clone(), db)?;
    options
        .resolve(db, default_access_key(env))
        .map_err(|e| match report {
            Report::Traced(sources) => sources.invalid(e),
            Report::Bare => e.to_string(),
        })
}

/// The access key file `kkt serve` uses when none is given: kkt-admission's
/// default, in kkt's config directory. `None` without a home.
pub(crate) fn default_access_key(env: &Env) -> Option<PathBuf> {
    kkt_config_file::config_dir(env.config_home().as_deref())
        .map(|dir| kkt_keys::access_key_file(&dir))
}

/// The address `kkt serve` would listen on with the default config file.
pub fn serve_bind(env: &Env, platform: Platform) -> Result<SocketAddr, String> {
    Ok(serve_settings(env, platform, None, ServeOptions::NONE)?.bind)
}

/// The settings `kkt serve` would run with, given `config` and `flags`,
/// without making anything. Without a home directory the database is an
/// empty path ([`DefaultDb::OrEmpty`]).
pub(crate) fn serve_settings(
    env: &Env,
    platform: Platform,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<ServeSettings, String> {
    Ok(serve_settings_and_sources(env, platform, config, flags)?.0)
}

/// [`serve_settings`], with where each setting came from.
pub(crate) fn serve_settings_and_sources(
    env: &Env,
    platform: Platform,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<(ServeSettings, Sources), String> {
    let loaded = load_serve_options(env, config, flags)?;
    let sources = loaded.sources();
    let settings = resolve(
        env,
        platform,
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
    platform: Platform,
    serve_args: ServeArgs,
    args: &[String],
) -> Result<ServeSettings, String> {
    let (config, options) = serve_args(args)?;
    serve_settings(env, platform, config, options)
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
    let unreadable = |e: config_files::ReadError| {
        format!(
            "invalid setting: config could not be read from {}: {}",
            e.path.display(),
            e.cause
        )
    };
    // A file named must be there; the default one is read only if it is.
    let found = match config {
        Some(path) => {
            let text = config_files::read_named(&ConfigDisk, &path).map_err(unreadable)?;
            Some((path, text))
        }
        None => match kkt_config_file::config_file(env.config_home().as_deref()) {
            Some(path) => config_files::read_default(&ConfigDisk, &path)
                .map_err(unreadable)?
                .map(|text| (path, text)),
            None => None,
        },
    };
    match found {
        Some((path, text)) => {
            let dir = path.parent().unwrap_or(Path::new("."));
            let from_file = parse_config(&text, dir, env.home.as_deref())
                .map_err(|e| format!("invalid setting: {e} (in {})", path.display()))?;
            Ok((Some(path), from_file))
        }
        None => Ok((None, ServeOptions::NONE)),
    }
}

/// The settings in a config file's `text`, as [`ServeOptions`]: the
/// file parsed by kkt-config-file, with kkt-log-tracing's log levels.
pub(crate) fn parse_config(
    text: &str,
    dir: &Path,
    home: Option<&Path>,
) -> Result<ServeOptions, BadValue> {
    let level = |name, text: &str| LogLevel::parse(name, text).map_err(|e| e.rule);
    let config = kkt_config_file::parse_file(text, dir, home, level)
        .map_err(|e| BadValue::new(e.name, e.rule))?;
    Ok(serve_options(config))
}

/// What a parsed config file says, as [`ServeOptions`].
fn serve_options(config: Config<LogLevel>) -> ServeOptions {
    ServeOptions {
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
    }
}

/// Parses the kkt container's config file for kkt-containerized-service, as `kkt serve`
/// in the container would: a setting `kkt serve` would refuse is refused.
pub(crate) struct ContainerConfigs;

impl kkt_containerized_service::ConfigReader for ContainerConfigs {
    fn settings(
        &self,
        text: &str,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<kkt_containerized_service::ContainerConfig, String> {
        let options = parse_config(text, dir, home).map_err(|e| e.to_string())?;
        Ok(kkt_containerized_service::ContainerConfig {
            name_pack: options.name_pack,
            max_lane_chars: options.max_lane_chars,
        })
    }
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
    pub fn invalid(&self, e: impl BadSetting) -> String {
        match self.set_in(e.setting()) {
            Some(path) => format!("invalid setting: {e} (set in {})", path.display()),
            None => format!("invalid setting: {e}"),
        }
    }

    /// The config file that set the setting `name`, if it was set there and
    /// not by a flag.
    pub fn set_in(&self, name: &str) -> Option<&Path> {
        match &self.file {
            Some((path, file)) if file.gives(name) && !self.flags.gives(name) => Some(path),
            _ => None,
        }
    }
}

/// A bad setting, as each part that checks settings reports it in its own
/// error type: the setting's name, and a message that starts with it.
pub trait BadSetting: std::fmt::Display {
    /// The setting's name, as the config file and the flags name it.
    fn setting(&self) -> &'static str;
}

impl BadSetting for BadValue {
    fn setting(&self) -> &'static str {
        self.name
    }
}

impl BadSetting for kkt_server::SettingError {
    fn setting(&self) -> &'static str {
        self.name
    }
}

impl BadSetting for kkt_log_tracing::SettingError {
    fn setting(&self) -> &'static str {
        self.name
    }
}

impl BadSetting for kkt_hosts::SettingError {
    fn setting(&self) -> &'static str {
        self.name
    }
}

impl BadSetting for kkt_keys::SettingError {
    fn setting(&self) -> &'static str {
        self.name
    }
}

/// Settings that passed every startup check, with the files they name read.
pub struct Checked {
    pub db_path: PathBuf,
    pub bind: SocketAddr,
    pub config: AppConfig,
    pub server_settings: CheckedSettings,
    /// The rules' settings, which the program starts the rules with.
    pub rules: kkt_rules::Settings,
    /// How many backups to keep before an upgrade.
    pub backups: kkt_backup::Settings,
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

/// The order the startup checks report bad settings in: the first problem
/// in this list wins, whichever owner found it. A problem the list does not
/// name comes after every one it does, in the order given.
const CHECK_ORDER: &[(&str, &str)] = &[
    ("bind", "check_bind"),
    ("allow_any_bind", "check_bind"),
    ("dependency_log_level", "check_dependency_log_level"),
    ("default_limit", "floor"),
    ("max_body_bytes", "floor"),
    ("max_channel_name_chars", "floor"),
    ("max_purpose_chars", "floor"),
    ("max_description_chars", "floor"),
    ("max_note_chars", "floor"),
    ("max_server_announcement_chars", "floor"),
    ("max_memory_entry_chars", "floor"),
    ("max_memory_chars", "floor"),
    ("max_lane_chars", "floor"),
    ("sends_per_minute", "floor"),
    ("registrations_per_minute", "floor"),
    ("max_agents", "floor"),
    ("web_page_messages", "floor"),
    ("web_member_excerpt_chars", "floor"),
    ("upgrade_backups_kept", "floor"),
    ("max_response_bytes", "floor"),
    ("max_limit", "default_limit"),
    ("max_memory_chars", "max_memory_entry_chars"),
    ("web_page_messages", "ceiling"),
    ("web_session_lifetime", "floor"),
    ("waiting_grace", "max_wait"),
    ("max_wait", "default_wait"),
    ("allow_host", "host"),
];

/// The first of `problems`, each (setting, owner's check, rule), in
/// [`CHECK_ORDER`], or none.
fn first_problem(
    problems: Vec<(&'static str, &'static str, String)>,
) -> Option<kkt_server::SettingError> {
    let index = CHECK_ORDER
        .iter()
        .find_map(|key| {
            problems
                .iter()
                .position(|(name, check, _)| (*name, *check) == *key)
        })
        .or_else(|| (!problems.is_empty()).then_some(0))?;
    let (name, _, rule) = problems.into_iter().nth(index)?;
    Some(kkt_server::SettingError { name, rule })
}

/// Runs the startup checks on `settings` and reads the files they name.
/// The errors name the bad setting, and the config file if it set it.
pub fn check_settings(settings: ServeSettings, sources: &Sources) -> Result<Checked, String> {
    let ServeSettings {
        db: db_path,
        bind,
        allow_any_bind,
        server: server_settings,
        rules,
        backups,
        instructions,
        access_key,
        web_key,
        admin_key,
        log_file,
        log_level,
        dependency_log_level,
    } = settings;
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
    if let Err(p) = check_dependency_log_level(dependency_log_level) {
        problems.push((p.name, "check_dependency_log_level", p.rule));
    }
    problems.extend(server_settings.problems());
    if let Some(problem) = first_problem(problems) {
        return Err(sources.invalid(problem));
    }
    let server_settings = server_settings.check([]).map_err(|e| sources.invalid(e))?;
    // None configured: the server sends its built-in instructions.
    let instructions = instructions
        .as_ref()
        .map(|path| config_files::read_instructions(&ConfigDisk, path).map_err(|e| e.to_string()))
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
        // kkt's admin endpoint takes its admin key.
        admin_admission: admin_key.map(AdminAdmission::SharedKey),
        instructions,
    };
    Ok(Checked {
        db_path,
        bind,
        config,
        server_settings,
        rules,
        backups,
        admin_name,
        allow_any_bind,
        config_file: sources.file.as_ref().map(|(p, _)| p.display().to_string()),
        log_file: log_file.map(|p| p.display().to_string()),
        log_level,
        dependency_log_level,
    })
}

#[cfg(test)]
pub(crate) use test_support::{TEST_PLATFORM, test_env, write_default_config};

#[cfg(test)]
mod test_support {
    use super::*;
    /// An environment with only `home` set, for tests.
    pub(crate) fn test_env(home: Option<&Path>) -> Env {
        Env {
            home: home.map(Path::to_path_buf),
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        }
    }

    /// The platform the tests place files for. Every place a test looks is
    /// worked out for it too, so it need not be this machine's.
    pub(crate) const TEST_PLATFORM: Platform = Platform::Xdg;

    /// Writes `text` as the default config file under `home`, and returns its
    /// path, for tests.
    pub(crate) fn write_default_config(home: &Path, text: &str) -> PathBuf {
        let path = kkt_config_file::config_file(test_env(Some(home)).config_home().as_deref())
            .expect("a config path");
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("config folder");
        std::fs::write(&path, text).expect("config file");
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch;
    use kkt_server::ServerSettings;

    /// Which bad setting the startup checks report, of all those given, as
    /// `check_settings` asks it.
    fn first_reported(problems: Vec<(&'static str, &'static str, String)>) -> String {
        first_problem(problems).expect("a problem").rule
    }

    /// The order bad settings are reported in, pinned: given every kind of
    /// problem at once, last first, each report names the next in this list.
    #[test]
    fn bad_settings_are_reported_in_a_fixed_order() {
        const ORDER: &[(&str, &str)] = &[
            ("bind", "check_bind"),
            ("allow_any_bind", "check_bind"),
            ("dependency_log_level", "check_dependency_log_level"),
            ("default_limit", "floor"),
            ("max_body_bytes", "floor"),
            ("max_channel_name_chars", "floor"),
            ("max_purpose_chars", "floor"),
            ("max_description_chars", "floor"),
            ("max_note_chars", "floor"),
            ("max_server_announcement_chars", "floor"),
            ("max_memory_entry_chars", "floor"),
            ("max_memory_chars", "floor"),
            ("max_lane_chars", "floor"),
            ("sends_per_minute", "floor"),
            ("registrations_per_minute", "floor"),
            ("max_agents", "floor"),
            ("web_page_messages", "floor"),
            ("web_member_excerpt_chars", "floor"),
            ("upgrade_backups_kept", "floor"),
            ("max_response_bytes", "floor"),
            ("max_limit", "default_limit"),
            ("max_memory_chars", "max_memory_entry_chars"),
            ("web_page_messages", "ceiling"),
            ("web_session_lifetime", "floor"),
            ("waiting_grace", "max_wait"),
            ("max_wait", "default_wait"),
            ("allow_host", "host"),
        ];
        let mut left: Vec<_> = ORDER
            .iter()
            .rev()
            .map(|(name, check)| (*name, *check, format!("{name}/{check}")))
            .collect();
        // One the order does not know comes after every known one.
        left.insert(
            0,
            ("product_setting", "floor", "product_setting/floor".into()),
        );
        let mut reported = Vec::new();
        while !left.is_empty() {
            let rule = first_reported(left.clone());
            left.retain(|(_, _, r)| *r != rule);
            reported.push(rule);
        }
        let mut expected: Vec<_> = ORDER
            .iter()
            .map(|(name, check)| format!("{name}/{check}"))
            .collect();
        expected.push("product_setting/floor".into());
        assert_eq!(reported, expected);
    }

    #[test]
    fn dependency_levels_that_would_log_secrets_are_refused() {
        for level in [LogLevel::Error, LogLevel::Warn, LogLevel::Info] {
            assert_eq!(check_dependency_log_level(level), Ok(()));
        }
        let refused = check_dependency_log_level(LogLevel::Debug).expect_err("refused");
        assert_eq!(
            refused.to_string(),
            "dependency_log_level must be error, warn or info, not debug: at debug and trace \
             the MCP library logs request contents, including tokens"
        );
        let refused = check_dependency_log_level(LogLevel::Trace).expect_err("refused");
        assert_eq!(refused.name, "dependency_log_level");
    }

    /// The log file's refusals read as they did when kkt-rolling-file
    /// worded them itself, naming kkt's settings.
    #[test]
    fn a_log_file_refusal_names_the_setting_it_comes_from() {
        use kkt_rolling_file::{LogPolicy, OpenError, open_log_file};
        let home = scratch::Dir::new("log-file-refusal");
        let clock = std::sync::Arc::new(kkt_clock::testing::FixedClock::new(
            kkt_clock::Time::from_text("2026-11-16T00:00:00Z").expect("time"),
        ));
        let small = LogPolicy {
            max_bytes: 1,
            ..LogPolicy::DEFAULT
        };
        let refused = open_log_file(&home.join("kkt.log"), small, clock.clone())
            .err()
            .expect("refused");
        assert_eq!(
            log_file_refusal(refused),
            "invalid setting: log_max_bytes must be at least 65536"
        );
        for (policy, said) in [
            (
                LogPolicy {
                    rotated_files: 0,
                    ..LogPolicy::DEFAULT
                },
                "invalid setting: log_rotated_files must be at least 1",
            ),
            (
                LogPolicy {
                    max_age: std::time::Duration::from_secs(60),
                    ..LogPolicy::DEFAULT
                },
                "invalid setting: log_max_age must be at least an hour",
            ),
        ] {
            let refused = open_log_file(&home.join("kkt.log"), policy, clock.clone())
                .err()
                .expect("refused");
            assert_eq!(log_file_refusal(refused), said);
        }
        let blocker = home.join("not-a-folder");
        std::fs::write(&blocker, "").expect("write");
        let path = blocker.join("kkt.log");
        let refused = open_log_file(&path, LogPolicy::DEFAULT, clock)
            .err()
            .expect("refused");
        assert!(matches!(refused, OpenError::File { .. }));
        let said = log_file_refusal(refused);
        let start = format!(
            "invalid setting: log_file could not be opened at {}: ",
            path.display()
        );
        assert!(said.starts_with(&start), "{said}");
    }

    fn config_path_env(home: Option<&str>, config: Option<&str>) -> Env {
        Env {
            home: home.map(PathBuf::from),
            named: Default::default(),
            xdg_config_home: config.map(PathBuf::from),
            xdg_data_home: None,
        }
    }

    #[test]
    fn the_config_file_goes_under_dot_config_on_every_platform() {
        let e = config_path_env(Some("/home/sam"), None);
        assert_eq!(
            kkt_config_file::config_file(e.config_home().as_deref()),
            Some(PathBuf::from("/home/sam/.config/kkt/config.toml"))
        );
        let e = config_path_env(Some("/home/sam"), Some("/xdg/config"));
        assert_eq!(
            kkt_config_file::config_file(e.config_home().as_deref()),
            Some(PathBuf::from("/xdg/config/kkt/config.toml"))
        );
    }

    #[test]
    fn a_relative_xdg_config_home_is_ignored() {
        let e = config_path_env(Some("/home/sam"), Some("relative/config"));
        assert_eq!(
            kkt_config_file::config_dir(e.config_home().as_deref()),
            Some(PathBuf::from("/home/sam/.config/kkt"))
        );
    }

    #[test]
    fn without_a_home_there_is_no_config_file_unless_xdg_says() {
        assert_eq!(
            kkt_config_file::config_file(config_path_env(None, None).config_home().as_deref()),
            None
        );
        assert!(
            kkt_config_file::config_file(
                config_path_env(None, Some("/xdg/config"))
                    .config_home()
                    .as_deref()
            )
            .is_some()
        );
    }

    #[test]
    fn a_parsed_config_file_gives_each_setting_it_says() {
        let options = parse_config(
            r#"
            db = "~/data/kkt.db"
            name_pack = "garden"
            bind = "127.0.0.1:9000"
            allow_any_bind = true
            max_response_bytes = 4096
            default_limit = 10
            max_limit = 40
            sends_per_minute = 5
            waiting_grace_seconds = 4
            upgrade_backups_kept = 2
            max_lane_chars = 12000
            max_memory_entry_chars = 3000
            max_memory_chars = 30000
            allow_origins = ["http://localhost:6274"]
            allow_hosts = ["host.docker.internal", "box.ts.net"]
            instructions = "instructions.md"
            access_key_file = "/keys/kkt"
            no_access_key = false
            log_file = "~/logs/kkt.log"
            log_level = "debug"
            dependency_log_level = "info"
            "#,
            Path::new("/etc/kkt"),
            Some(Path::new("/home/sam")),
        )
        .expect("valid file");
        assert_eq!(
            options,
            ServeOptions {
                name_pack: Some("garden".into()),
                waiting_grace_seconds: Some(4),
                upgrade_backups_kept: Some(2),
                max_lane_chars: Some(12000),
                max_memory_entry_chars: Some(3000),
                max_memory_chars: Some(30000),
                db: Some(PathBuf::from("/home/sam/data/kkt.db")),
                bind: Some("127.0.0.1:9000".parse().expect("addr")),
                allow_any_bind: Some(true),
                max_response_bytes: Some(4096),
                default_limit: Some(10),
                max_limit: Some(40),
                sends_per_minute: Some(5),
                allow_origins: Some(vec!["http://localhost:6274".into()]),
                allow_hosts: Some(vec!["host.docker.internal".into(), "box.ts.net".into()]),
                instructions: Some(PathBuf::from("/etc/kkt/instructions.md")),
                access_key_file: Some(PathBuf::from("/keys/kkt")),
                no_access_key: Some(false),
                log_file: Some(PathBuf::from("/home/sam/logs/kkt.log")),
                log_level: Some(LogLevel::Debug),
                dependency_log_level: Some(LogLevel::Info),
            }
        );
        assert_eq!(
            parse_config("", Path::new("/"), None),
            Ok(ServeOptions::NONE)
        );
    }

    #[test]
    fn a_config_files_bad_value_is_refused_naming_the_setting() {
        let parse = |text| parse_config(text, Path::new("/etc/kkt"), None);
        for (text, name) in [
            ("log_level = \"loud\"", "log_level"),
            ("dependency_log_level = \"loud\"", "dependency_log_level"),
            ("dependency_log_level = 3", "dependency_log_level"),
            ("max_limit = -1", "max_limit"),
            ("bnid = 1", "config"),
        ] {
            assert_eq!(parse(text).map_err(|e| e.name), Err(name), "{text}");
        }
        // The file's own wording, as kkt-config-file gives it.
        assert_eq!(
            parse("max_limit = \"5\"").unwrap_err().to_string(),
            "max_limit in the config file must be a whole number of zero or more, not \"5\""
        );
        // A level that is not one, worded as kkt-log-tracing words it.
        let loud = LogLevel::parse("log_level", "loud").unwrap_err();
        assert_eq!(
            parse("log_level = \"loud\"").unwrap_err().to_string(),
            format!("{} {}", loud.name, loud.rule)
        );
        // Of two bad settings, the first by name is named, as before
        // kkt-config-file: a log level is checked in its turn.
        for (text, name) in [
            ("max_limit = -1\nlog_level = \"loud\"", "log_level"),
            ("dependency_log_level = \"loud\"\nbind = 1", "bind"),
            (
                "log_level = \"loud\"\ndependency_log_level = \"quiet\"",
                "dependency_log_level",
            ),
        ] {
            assert_eq!(parse(text).map_err(|e| e.name), Err(name), "{text}");
        }
    }

    #[test]
    fn a_container_config_gives_its_name_pack_and_lane_limit() {
        use kkt_containerized_service::ConfigReader;
        let said = ContainerConfigs
            .settings(
                "name_pack = \"garden\"\nmax_lane_chars = 300\ndb = \"~/kkt.db\"",
                Path::new("/"),
                Some(Path::new("/data")),
            )
            .expect("valid");
        assert_eq!(said.name_pack.as_deref(), Some("garden"));
        assert_eq!(said.max_lane_chars, Some(300));
        // A level kkt serve would refuse is refused here too.
        let err = ContainerConfigs
            .settings("log_level = \"loud\"", Path::new("/"), None)
            .unwrap_err();
        assert!(err.starts_with("log_level "), "{err}");
    }

    const NO_HOME_KEY: &str = "invalid setting: access_key_file has no default without a home \
         directory: set HOME, or give access_key_file or no_access_key";

    #[test]
    fn without_a_home_the_database_is_an_empty_path() {
        let dir = scratch::Dir::new("settings-no-home-db");
        let config = dir.join("serve.toml");
        std::fs::write(&config, "no_access_key = true").expect("config");
        let settings = serve_settings(
            &test_env(None),
            TEST_PLATFORM,
            Some(config),
            ServeOptions::NONE,
        )
        .expect("valid");
        assert_eq!(settings.db, PathBuf::new());
        assert_eq!(settings.access_key, AccessKey::None);
        assert_eq!(settings.bind, kkt_hosts::DEFAULT_BIND);
    }

    #[test]
    fn without_a_home_or_a_key_setting_the_key_has_no_default() {
        let env = test_env(None);
        assert_eq!(
            serve_settings(&env, TEST_PLATFORM, None, ServeOptions::NONE).map(|s| s.db),
            Err(NO_HOME_KEY.to_string())
        );
        assert_eq!(
            serve_bind(&env, TEST_PLATFORM),
            Err(NO_HOME_KEY.to_string())
        );
    }

    #[test]
    fn with_a_home_the_defaults_apply_and_flags_win_over_the_file() {
        let home = scratch::Dir::new("settings-home-defaults");
        let env = test_env(Some(&home));
        let settings =
            serve_settings(&env, TEST_PLATFORM, None, ServeOptions::NONE).expect("valid");
        assert_eq!(
            Some(settings.db),
            kkt_deployment::default_db(&env, TEST_PLATFORM)
        );
        assert_eq!(
            settings.access_key,
            AccessKey::Default(default_access_key(&env).expect("key path"))
        );
        write_default_config(
            &home,
            "db = \"/data/file.db\"\nbind = \"127.0.0.1:8000\"\nmax_limit = 50",
        );
        let flags = ServeOptions {
            bind: Some("127.0.0.1:9000".parse().expect("addr")),
            ..ServeOptions::NONE
        };
        let settings = serve_settings(&env, TEST_PLATFORM, None, flags).expect("valid");
        assert_eq!(settings.db, PathBuf::from("/data/file.db"));
        assert_eq!(settings.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(settings.rules.max_limit, 50);
        assert_eq!(
            serve_bind(&env, TEST_PLATFORM).map(|b| b.to_string()),
            Ok("127.0.0.1:8000".into())
        );
    }

    #[test]
    fn a_bad_setting_names_the_config_file_only_when_it_set_it() {
        let home = scratch::Dir::new("settings-bad-setting");
        let env = test_env(Some(&home));
        let path = write_default_config(&home, "name_pack = \"missing\"");
        let err =
            serve_settings(&env, TEST_PLATFORM, None, ServeOptions::NONE).expect_err("bad pack");
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
        let err = serve_settings(&env, TEST_PLATFORM, None, flags).expect_err("bad pack");
        assert!(!err.contains("(set in"), "{err}");
    }

    /// Of several bad settings, the check names one, always the same, in an
    /// order nothing else pins: refusals have always followed it, and it
    /// must stay.
    /// Every setting starts bad; each run names the next, which is then
    /// fixed. `max_memory_chars` and `web_page_messages` are named twice:
    /// first for their floor, then for their bound against another setting.
    #[test]
    fn of_several_bad_settings_the_same_one_is_named_first() {
        use std::time::Duration;
        type Fix = fn(&mut ServeSettings);
        let mut settings = ServeSettings {
            db: PathBuf::from("kkt.db"),
            bind: "0.0.0.0:7420".parse().expect("an address"),
            allow_any_bind: false,
            rules: kkt_rules::Settings {
                default_limit: 0,
                max_limit: 0,
                max_body_bytes: 0,
                max_channel_name_chars: 0,
                max_purpose_chars: 0,
                max_description_chars: 0,
                max_note_chars: 0,
                max_server_announcement_chars: 0,
                max_memory_entry_chars: 0,
                max_memory_chars: 0,
                max_lane_chars: 0,
                sends_per_minute: 0,
                registrations_per_minute: 0,
                max_agents: 0,
                max_wait: Duration::from_secs(10),
                waiting_grace: Duration::from_secs(20),
                ..kkt_rules::Settings::DEFAULT
            },
            server: ServerSettings {
                formatter: kkt_mcp::result_format::Settings {
                    max_response_bytes: 0,
                },
                web: kkt_web::Settings {
                    page: kkt_web::PageLimits {
                        page_messages: 0,
                        member_excerpt_chars: 0,
                    },
                    session_lifetime: Duration::ZERO,
                    ..kkt_web::Settings::DEFAULT
                },
                allowed_hosts: vec!["not a host!".into()],
                ..ServerSettings::DEFAULT
            },
            backups: kkt_backup::Settings {
                upgrade_backups_kept: 0,
            },
            instructions: None,
            access_key: AccessKey::None,
            web_key: AccessKey::None,
            admin_key: None,
            log_file: None,
            log_level: LogLevel::Info,
            dependency_log_level: LogLevel::Debug,
        };
        let d = kkt_rules::Settings::DEFAULT;
        let fixes: [(&str, Fix); 26] = [
            ("bind", |s| {
                s.bind = "127.0.0.1:7420".parse().expect("an address")
            }),
            ("dependency_log_level", |s| {
                s.dependency_log_level = LogLevel::Warn
            }),
            ("default_limit", |s| {
                s.rules.default_limit = kkt_rules::Settings::DEFAULT.default_limit
            }),
            ("max_body_bytes", |s| {
                s.rules.max_body_bytes = kkt_rules::Settings::DEFAULT.max_body_bytes
            }),
            ("max_channel_name_chars", |s| {
                s.rules.max_channel_name_chars = kkt_rules::Settings::DEFAULT.max_channel_name_chars
            }),
            ("max_purpose_chars", |s| {
                s.rules.max_purpose_chars = kkt_rules::Settings::DEFAULT.max_purpose_chars
            }),
            ("max_description_chars", |s| {
                s.rules.max_description_chars = kkt_rules::Settings::DEFAULT.max_description_chars
            }),
            ("max_note_chars", |s| {
                s.rules.max_note_chars = kkt_rules::Settings::DEFAULT.max_note_chars
            }),
            ("max_server_announcement_chars", |s| {
                s.rules.max_server_announcement_chars =
                    kkt_rules::Settings::DEFAULT.max_server_announcement_chars
            }),
            ("max_memory_entry_chars", |s| {
                s.rules.max_memory_entry_chars = kkt_rules::Settings::DEFAULT.max_memory_entry_chars
            }),
            // Past its floor, but still below max_memory_entry_chars.
            ("max_memory_chars", |s| s.rules.max_memory_chars = 1),
            ("max_lane_chars", |s| {
                s.rules.max_lane_chars = kkt_rules::Settings::DEFAULT.max_lane_chars
            }),
            ("sends_per_minute", |s| {
                s.rules.sends_per_minute = kkt_rules::Settings::DEFAULT.sends_per_minute
            }),
            ("registrations_per_minute", |s| {
                s.rules.registrations_per_minute =
                    kkt_rules::Settings::DEFAULT.registrations_per_minute
            }),
            ("max_agents", |s| {
                s.rules.max_agents = kkt_rules::Settings::DEFAULT.max_agents
            }),
            // Past its floor, but over the page ceiling.
            ("web_page_messages", |s| {
                s.server.web.page.page_messages = kkt_web::MAX_WEB_PAGE_MESSAGES + 1
            }),
            ("web_member_excerpt_chars", |s| {
                s.server.web.page.member_excerpt_chars =
                    kkt_web::Settings::DEFAULT.page.member_excerpt_chars
            }),
            ("upgrade_backups_kept", |s| {
                s.backups.upgrade_backups_kept = kkt_backup::Settings::DEFAULT.upgrade_backups_kept
            }),
            ("max_response_bytes", |s| {
                s.server.formatter.max_response_bytes =
                    kkt_mcp::result_format::Settings::DEFAULT.max_response_bytes
            }),
            ("max_limit", |s| {
                s.rules.max_limit = kkt_rules::Settings::DEFAULT.max_limit
            }),
            ("max_memory_chars", |s| {
                s.rules.max_memory_chars = kkt_rules::Settings::DEFAULT.max_memory_chars
            }),
            ("web_page_messages", |s| {
                s.server.web.page.page_messages = kkt_web::Settings::DEFAULT.page.page_messages
            }),
            ("web_session_lifetime", |s| {
                s.server.web.session_lifetime = kkt_web::Settings::DEFAULT.session_lifetime
            }),
            ("waiting_grace", |s| {
                s.rules.waiting_grace = kkt_rules::Settings::DEFAULT.waiting_grace
            }),
            ("max_wait", |s| {
                s.rules.max_wait = kkt_rules::Settings::DEFAULT.max_wait
            }),
            ("allow_host", |s| s.server.allowed_hosts.clear()),
        ];
        assert!(
            d.max_wait > Duration::from_secs(10),
            "the start is bad for max_wait"
        );
        let sources = Sources {
            flags: ServeOptions::NONE,
            file: None,
        };
        let mut named = Vec::new();
        for (expected, fix) in fixes {
            let err = match check_settings(settings.clone(), &sources) {
                Ok(_) => panic!("{expected} is still bad, but the check passed"),
                Err(e) => e,
            };
            let rest = err
                .strip_prefix("invalid setting: ")
                .unwrap_or_else(|| panic!("an invalid setting: {err}"));
            named.push(rest.split(' ').next().unwrap_or_default().to_string());
            fix(&mut settings);
        }
        assert_eq!(
            named,
            [
                "bind",
                "dependency_log_level",
                "default_limit",
                "max_body_bytes",
                "max_channel_name_chars",
                "max_purpose_chars",
                "max_description_chars",
                "max_note_chars",
                "max_server_announcement_chars",
                "max_memory_entry_chars",
                "max_memory_chars",
                "max_lane_chars",
                "sends_per_minute",
                "registrations_per_minute",
                "max_agents",
                "web_page_messages",
                "web_member_excerpt_chars",
                "upgrade_backups_kept",
                "max_response_bytes",
                "max_limit",
                "max_memory_chars",
                "web_page_messages",
                "web_session_lifetime",
                "waiting_grace",
                "max_wait",
                "allow_host",
            ]
        );
        assert!(
            check_settings(settings, &sources).is_ok(),
            "every setting fixed"
        );
    }
}
