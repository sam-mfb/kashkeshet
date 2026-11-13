//! The `kkt serve` surface: the settings a server may be given, from one
//! source (the command line or the config file), how a flag wins over the
//! file and the file over the defaults, and the settings to run with. Each
//! default is its owner's (kkt-rules, kkt-mcp-formatter, kkt-web,
//! kkt-backup, kkt-hosts, kkt-logs); settings are not a domain of their own
//! (Sam, 2026-11-12). This part reads no files and no environment: callers
//! pass in what the file gives and the defaults that depend on the machine.
//! Its items are public as [`crate::machine::settings`]'s.

use kkt_hosts::{DEFAULT_ALLOW_ANY_BIND, DEFAULT_BIND};
use kkt_logs::{DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL, LogLevel};
use kkt_server::ServerSettings;
use kkt_server_keys::{AccessKey, admin_key_path, web_key_path};
use std::net::SocketAddr;
use std::path::PathBuf;

/// Browser origins allowed to call `/mcp` (`allow_origins`): none.
const DEFAULT_ALLOW_ORIGINS: &[&str] = &[];
/// Host names the server answers to besides loopback (`allow_hosts`): none.
const DEFAULT_ALLOW_HOSTS: &[&str] = &[];

/// A setting given that cannot be used, before any owner checks it: its
/// name and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadValue {
    pub name: &'static str,
    pub rule: String,
}

impl BadValue {
    pub fn new(name: &'static str, rule: impl Into<String>) -> BadValue {
        BadValue {
            name,
            rule: rule.into(),
        }
    }
}

impl std::fmt::Display for BadValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.rule)
    }
}

/// Server settings that may or may not be given, from one source: the
/// command line or the config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeOptions {
    pub name_pack: Option<String>,
    pub db: Option<PathBuf>,
    pub bind: Option<SocketAddr>,
    /// Allow `bind` to be a non-loopback address. Needs an access key.
    pub allow_any_bind: Option<bool>,
    pub max_response_bytes: Option<usize>,
    pub default_limit: Option<usize>,
    pub max_limit: Option<usize>,
    pub sends_per_minute: Option<u32>,
    pub waiting_grace_seconds: Option<u32>,
    /// How many backups made before upgrading the database to keep.
    pub upgrade_backups_kept: Option<usize>,
    /// Longest lane instructions, in characters.
    pub max_lane_chars: Option<usize>,
    /// Longest memory entry, in characters.
    pub max_memory_entry_chars: Option<usize>,
    /// Most characters in one avatar's memory, all entries together.
    pub max_memory_chars: Option<usize>,
    pub allow_origins: Option<Vec<String>>,
    pub allow_hosts: Option<Vec<String>>,
    pub instructions: Option<PathBuf>,
    pub access_key_file: Option<PathBuf>,
    /// Run with no access key. Refused together with `access_key_file`.
    pub no_access_key: Option<bool>,
    /// Write the log to this file, kept bounded, instead of to stdout.
    pub log_file: Option<PathBuf>,
    /// How much kkt's own code logs.
    pub log_level: Option<LogLevel>,
    /// How much the libraries kkt uses log.
    pub dependency_log_level: Option<LogLevel>,
}

impl ServeOptions {
    /// Nothing given.
    pub const NONE: ServeOptions = ServeOptions {
        name_pack: None,
        db: None,
        bind: None,
        allow_any_bind: None,
        max_response_bytes: None,
        default_limit: None,
        max_limit: None,
        sends_per_minute: None,
        waiting_grace_seconds: None,
        upgrade_backups_kept: None,
        max_lane_chars: None,
        max_memory_entry_chars: None,
        max_memory_chars: None,
        allow_origins: None,
        allow_hosts: None,
        instructions: None,
        access_key_file: None,
        no_access_key: None,
        log_file: None,
        log_level: None,
        dependency_log_level: None,
    };

    /// Each setting from `self` if given there, else from `fallback`. A list
    /// given in `self` replaces the one in `fallback`; the two are not joined.
    pub fn over(self, fallback: ServeOptions) -> ServeOptions {
        ServeOptions {
            name_pack: self.name_pack.or(fallback.name_pack),
            db: self.db.or(fallback.db),
            bind: self.bind.or(fallback.bind),
            allow_any_bind: self.allow_any_bind.or(fallback.allow_any_bind),
            max_response_bytes: self.max_response_bytes.or(fallback.max_response_bytes),
            default_limit: self.default_limit.or(fallback.default_limit),
            max_limit: self.max_limit.or(fallback.max_limit),
            sends_per_minute: self.sends_per_minute.or(fallback.sends_per_minute),
            waiting_grace_seconds: self
                .waiting_grace_seconds
                .or(fallback.waiting_grace_seconds),
            upgrade_backups_kept: self.upgrade_backups_kept.or(fallback.upgrade_backups_kept),
            max_lane_chars: self.max_lane_chars.or(fallback.max_lane_chars),
            max_memory_entry_chars: self
                .max_memory_entry_chars
                .or(fallback.max_memory_entry_chars),
            max_memory_chars: self.max_memory_chars.or(fallback.max_memory_chars),
            allow_origins: self.allow_origins.or(fallback.allow_origins),
            allow_hosts: self.allow_hosts.or(fallback.allow_hosts),
            instructions: self.instructions.or(fallback.instructions),
            access_key_file: self.access_key_file.or(fallback.access_key_file),
            no_access_key: self.no_access_key.or(fallback.no_access_key),
            log_file: self.log_file.or(fallback.log_file),
            log_level: self.log_level.or(fallback.log_level),
            dependency_log_level: self.dependency_log_level.or(fallback.dependency_log_level),
        }
    }

    /// Whether the setting `name`, as its owner's check names it, is given.
    pub fn gives(&self, name: &str) -> bool {
        match name {
            "name_pack" => self.name_pack.is_some(),
            "db" => self.db.is_some(),
            "bind" => self.bind.is_some(),
            "allow_any_bind" => self.allow_any_bind.is_some(),
            "max_response_bytes" => self.max_response_bytes.is_some(),
            "default_limit" => self.default_limit.is_some(),
            "max_limit" => self.max_limit.is_some(),
            "sends_per_minute" => self.sends_per_minute.is_some(),
            "waiting_grace" => self.waiting_grace_seconds.is_some(),
            "upgrade_backups_kept" => self.upgrade_backups_kept.is_some(),
            "max_lane_chars" => self.max_lane_chars.is_some(),
            "max_memory_entry_chars" => self.max_memory_entry_chars.is_some(),
            "max_memory_chars" => self.max_memory_chars.is_some(),
            "allow_origin" | "allow_origins" => self.allow_origins.is_some(),
            "allow_host" | "allow_hosts" => self.allow_hosts.is_some(),
            "instructions" => self.instructions.is_some(),
            "access_key_file" => self.access_key_file.is_some(),
            "no_access_key" => self.no_access_key.is_some(),
            "log_file" => self.log_file.is_some(),
            "log_level" => self.log_level.is_some(),
            "dependency_log_level" => self.dependency_log_level.is_some(),
            _ => false,
        }
    }

    /// The `max_lane_chars` given, else the default; unchecked, so commands
    /// that read the config file without starting a server can use it.
    /// [`ServeOptions::resolve`] uses it too.
    pub fn lane_limit(&self) -> usize {
        self.max_lane_chars
            .unwrap_or(kkt_rules::Settings::DEFAULT.max_lane_chars)
    }

    /// The name pack named, else the default's name; unchecked, as written,
    /// for `kkt status`. [`ServeOptions::resolve`] instead finds the pack
    /// and refuses an unknown name, so it does not use this.
    pub fn name_pack_name(&self) -> &str {
        self.name_pack
            .as_deref()
            .unwrap_or(kkt_rules::Settings::DEFAULT.name_pack.name())
    }

    /// The settings to run with: what was given, and the defaults for the
    /// rest. `default_db` is used only when no database was given.
    /// `default_key` is the key file used, and created if missing, when
    /// neither `access_key_file` nor `no_access_key` is given; `None` when
    /// there is no home directory to put it in.
    pub fn resolve(
        self,
        default_db: PathBuf,
        default_key: Option<PathBuf>,
    ) -> Result<ServeSettings, BadValue> {
        let max_lane_chars = self.lane_limit();
        let server = server_settings(&self);
        let access_key = match (self.no_access_key == Some(true), self.access_key_file) {
            (true, Some(_)) => {
                return Err(BadValue::new(
                    "no_access_key",
                    "cannot be set together with access_key_file",
                ));
            }
            (true, None) => AccessKey::None,
            (false, Some(path)) => AccessKey::File(path),
            (false, None) => match default_key {
                Some(path) => AccessKey::Default(path),
                None => {
                    return Err(BadValue::new(
                        "access_key_file",
                        "has no default without a home directory: set HOME, \
                         or give access_key_file or no_access_key",
                    ));
                }
            },
        };
        let (web_key, admin_key) = match &access_key {
            AccessKey::None => (AccessKey::None, None),
            AccessKey::File(path) | AccessKey::Default(path) => {
                let web = web_key_path(path);
                if &web == path {
                    return Err(BadValue::new(
                        "access_key_file",
                        "must not name the web key file",
                    ));
                }
                (AccessKey::Default(web), admin_key_path(path))
            }
        };
        let rules = kkt_rules::Settings::DEFAULT;
        Ok(ServeSettings {
            db: self.db.unwrap_or(default_db),
            bind: self.bind.unwrap_or(DEFAULT_BIND),
            allow_any_bind: self.allow_any_bind.unwrap_or(DEFAULT_ALLOW_ANY_BIND),
            rules: kkt_rules::Settings {
                name_pack: match self.name_pack {
                    None => rules.name_pack,
                    Some(name) => kkt_names::NamePack::find(&name).ok_or_else(|| {
                        BadValue::new(
                            "name_pack",
                            format!(
                                "must be one of: {}",
                                kkt_names::PACKS
                                    .iter()
                                    .map(|p| p.name())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        )
                    })?,
                },
                default_limit: self.default_limit.unwrap_or(rules.default_limit),
                max_limit: self.max_limit.unwrap_or(rules.max_limit),
                sends_per_minute: self.sends_per_minute.unwrap_or(rules.sends_per_minute),
                waiting_grace: self
                    .waiting_grace_seconds
                    .map(|s| std::time::Duration::from_secs(u64::from(s)))
                    .unwrap_or(rules.waiting_grace),
                max_lane_chars,
                max_memory_entry_chars: self
                    .max_memory_entry_chars
                    .unwrap_or(rules.max_memory_entry_chars),
                max_memory_chars: self.max_memory_chars.unwrap_or(rules.max_memory_chars),
                ..rules
            },
            server,
            backups: kkt_backup::Settings {
                upgrade_backups_kept: self
                    .upgrade_backups_kept
                    .unwrap_or(kkt_backup::Settings::DEFAULT.upgrade_backups_kept),
            },
            instructions: self.instructions,
            access_key,
            web_key,
            admin_key,
            log_file: self.log_file,
            log_level: self.log_level.unwrap_or(DEFAULT_LOG_LEVEL),
            dependency_log_level: self
                .dependency_log_level
                .unwrap_or(DEFAULT_DEPENDENCY_LOG_LEVEL),
        })
    }
}

/// What kkt-server builds with, from `options`: each config key and flag
/// onto one [`ServerSettings`] field, and its default where none is given.
/// The web page's and the MCP endpoints' settings have no key: they are
/// their owners' defaults.
pub fn server_settings(options: &ServeOptions) -> ServerSettings {
    ServerSettings {
        formatter: kkt_mcp_formatter::Settings {
            max_response_bytes: options
                .max_response_bytes
                .unwrap_or(kkt_mcp_formatter::Settings::DEFAULT.max_response_bytes),
        },
        web: kkt_web::Settings::DEFAULT,
        mcp: kkt_mcp::Settings::DEFAULT,
        allowed_origins: options
            .allow_origins
            .clone()
            .unwrap_or_else(|| strings(DEFAULT_ALLOW_ORIGINS)),
        allowed_hosts: options
            .allow_hosts
            .clone()
            .unwrap_or_else(|| strings(DEFAULT_ALLOW_HOSTS)),
    }
}

/// A default list as the owned list a setting holds.
fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// The settings a server runs with, each owner's in its own terms. They are
/// not yet checked: [`crate::machine::settings::check_settings`] asks each
/// owner for its problems and kkt-server names the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeSettings {
    pub db: PathBuf,
    pub bind: SocketAddr,
    /// Whether `bind` may be a non-loopback address.
    pub allow_any_bind: bool,
    /// What the rules enforce and fill in.
    pub rules: kkt_rules::Settings,
    /// What kkt-server builds with: the formatter's, the web page's and the
    /// MCP endpoints' settings, and the origins and hosts allowed.
    pub server: ServerSettings,
    /// How many backups made before an upgrade to keep.
    pub backups: kkt_backup::Settings,
    /// A file of instructions for agents, in place of the built-in ones.
    pub instructions: Option<PathBuf>,
    pub access_key: AccessKey,
    pub web_key: AccessKey,
    /// Where the admin key is, if the server has an access key. The admin
    /// endpoint is on only when this file exists; it is never made here.
    pub admin_key: Option<PathBuf>,
    /// The log file, if the log goes to one instead of stdout.
    pub log_file: Option<PathBuf>,
    pub log_level: LogLevel,
    pub dependency_log_level: LogLevel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_given_means_the_defaults() {
        let settings = ServeOptions::NONE
            .resolve(
                PathBuf::from("default.db"),
                Some(PathBuf::from("default-key")),
            )
            .expect("valid");
        assert_eq!(
            settings,
            ServeSettings {
                db: PathBuf::from("default.db"),
                bind: DEFAULT_BIND,
                allow_any_bind: false,
                rules: kkt_rules::Settings::DEFAULT,
                server: ServerSettings::DEFAULT,
                backups: kkt_backup::Settings::DEFAULT,
                instructions: None,
                access_key: AccessKey::Default(PathBuf::from("default-key")),
                web_key: AccessKey::Default(PathBuf::from("web-key")),
                admin_key: Some(PathBuf::from("admin-key")),
                log_file: None,
                log_level: LogLevel::Info,
                dependency_log_level: LogLevel::Warn,
            }
        );
        assert_eq!(settings.bind.to_string(), "127.0.0.1:7420");
    }

    #[test]
    fn an_access_key_named_admin_key_keeps_admin_access_disabled() {
        let options = ServeOptions {
            access_key_file: Some(PathBuf::from("/keys/admin-key")),
            ..ServeOptions::NONE
        };
        let settings = options.resolve(PathBuf::from("test.db"), None).unwrap();
        assert_eq!(
            settings.access_key,
            AccessKey::File(PathBuf::from("/keys/admin-key"))
        );
        assert_eq!(settings.admin_key, None);
        assert_eq!(
            settings.web_key,
            AccessKey::Default(PathBuf::from("/keys/web-key"))
        );
    }

    #[test]
    fn name_pack_comes_from_file_or_flag_and_unknown_names_are_refused() {
        let resolve = |options: ServeOptions| {
            options.resolve(PathBuf::from("test.db"), Some(PathBuf::from("key")))
        };
        let file = ServeOptions {
            name_pack: Some("garden".into()),
            ..ServeOptions::NONE
        };
        assert_eq!(
            resolve(file.clone()).unwrap().rules.name_pack.name(),
            "garden"
        );
        let flags = ServeOptions {
            name_pack: Some("animals".into()),
            ..ServeOptions::NONE
        };
        assert_eq!(
            resolve(flags.over(file)).unwrap().rules.name_pack.name(),
            "animals"
        );
        let bad = ServeOptions {
            name_pack: Some("missing".into()),
            ..ServeOptions::NONE
        };
        assert_eq!(resolve(bad).unwrap_err().name, "name_pack");
    }

    #[test]
    fn gives_answers_by_the_names_checks_use() {
        let options = ServeOptions {
            max_limit: Some(1),
            allow_hosts: Some(vec![]),
            ..ServeOptions::NONE
        };
        assert!(options.gives("max_limit"));
        assert!(options.gives("allow_host"));
        assert!(!options.gives("default_limit"));
        assert!(!options.gives("max_wait"));
        assert!(!ServeOptions::NONE.gives("bind"));
    }

    #[test]
    fn a_given_setting_wins_over_the_fallback() {
        let flags = ServeOptions {
            bind: Some("127.0.0.1:9000".parse().expect("addr")),
            allow_hosts: Some(vec!["flag.example".into()]),
            ..ServeOptions::NONE
        };
        let file = ServeOptions {
            db: Some(PathBuf::from("/data/file.db")),
            bind: Some("127.0.0.1:8000".parse().expect("addr")),
            max_limit: Some(50),
            allow_hosts: Some(vec!["file.example".into(), "other.example".into()]),
            no_access_key: Some(true),
            ..ServeOptions::NONE
        };
        let settings = flags
            .over(file)
            .resolve(PathBuf::from("default.db"), None)
            .expect("valid");
        assert_eq!(settings.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(settings.db, PathBuf::from("/data/file.db"));
        assert_eq!(settings.rules.max_limit, 50);
        assert_eq!(
            settings.rules.default_limit,
            kkt_rules::Settings::DEFAULT.default_limit
        );
        assert_eq!(settings.server.allowed_hosts, ["flag.example"]);
    }

    #[test]
    fn the_access_key_is_on_unless_turned_off() {
        let default_key = || Some(PathBuf::from("/home/sam/.config/kkt/access-key"));
        let resolve = |options: ServeOptions| options.resolve(PathBuf::from("db"), default_key());
        assert_eq!(
            resolve(ServeOptions::NONE).expect("valid").access_key,
            AccessKey::Default(PathBuf::from("/home/sam/.config/kkt/access-key"))
        );
        let file = ServeOptions {
            access_key_file: Some(PathBuf::from("/keys/k")),
            ..ServeOptions::NONE
        };
        assert_eq!(
            resolve(file.clone()).expect("valid").access_key,
            AccessKey::File(PathBuf::from("/keys/k"))
        );
        let off = ServeOptions {
            no_access_key: Some(true),
            ..ServeOptions::NONE
        };
        assert_eq!(
            resolve(off.clone()).expect("valid").access_key,
            AccessKey::None
        );
        // `no_access_key = false` is the same as not saying.
        let on = ServeOptions {
            no_access_key: Some(false),
            ..ServeOptions::NONE
        };
        assert!(matches!(
            resolve(on).expect("valid").access_key,
            AccessKey::Default(_)
        ));
        let both = ServeOptions {
            no_access_key: Some(true),
            ..file
        };
        assert_eq!(resolve(both).map_err(|e| e.name), Err("no_access_key"));
        // A flag's key file and the file's opt-out together are refused too.
        let file_off = off.over(ServeOptions {
            access_key_file: Some(PathBuf::from("/keys/k")),
            ..ServeOptions::NONE
        });
        assert_eq!(resolve(file_off).map_err(|e| e.name), Err("no_access_key"));
    }

    #[test]
    fn max_lane_chars_comes_from_file_or_flag_and_zero_is_refused() {
        let resolve = |options: ServeOptions| {
            options
                .resolve(PathBuf::from("db"), Some(PathBuf::from("key")))
                .expect("valid")
        };
        let file = ServeOptions {
            max_lane_chars: Some(12000),
            ..ServeOptions::NONE
        };
        let settings = resolve(file.clone());
        assert_eq!(settings.rules.max_lane_chars, 12000);
        assert!(settings.rules.problems().is_empty(), "12000 is allowed");
        let flags = ServeOptions {
            max_lane_chars: Some(300),
            ..ServeOptions::NONE
        };
        assert!(flags.gives("max_lane_chars"));
        assert_eq!(resolve(flags.over(file)).rules.max_lane_chars, 300);
        assert_eq!(
            resolve(ServeOptions::NONE).rules.max_lane_chars,
            kkt_rules::Settings::DEFAULT.max_lane_chars
        );
        let zero = ServeOptions {
            max_lane_chars: Some(0),
            ..ServeOptions::NONE
        };
        assert_eq!(resolve(zero).rules.problems()[0].name, "max_lane_chars");
    }

    #[test]
    fn memory_limits_come_from_file_or_flag_and_are_checked() {
        let resolve = |options: ServeOptions| {
            options
                .resolve(PathBuf::from("db"), Some(PathBuf::from("key")))
                .expect("valid")
        };
        let file = ServeOptions {
            max_memory_entry_chars: Some(500),
            max_memory_chars: Some(2000),
            ..ServeOptions::NONE
        };
        let settings = resolve(file.clone());
        assert_eq!(settings.rules.max_memory_entry_chars, 500);
        assert_eq!(settings.rules.max_memory_chars, 2000);
        assert!(settings.rules.problems().is_empty(), "allowed");
        let flags = ServeOptions {
            max_memory_chars: Some(700),
            ..ServeOptions::NONE
        };
        assert!(flags.gives("max_memory_chars"));
        assert!(!flags.gives("max_memory_entry_chars"));
        let both = resolve(flags.over(file));
        assert_eq!(
            (
                both.rules.max_memory_entry_chars,
                both.rules.max_memory_chars
            ),
            (500, 700)
        );
        let defaults = resolve(ServeOptions::NONE).rules;
        assert_eq!(
            (defaults.max_memory_entry_chars, defaults.max_memory_chars),
            (
                kkt_rules::Settings::DEFAULT.max_memory_entry_chars,
                kkt_rules::Settings::DEFAULT.max_memory_chars
            )
        );
        let small = ServeOptions {
            max_memory_entry_chars: Some(500),
            max_memory_chars: Some(400),
            ..ServeOptions::NONE
        };
        assert_eq!(resolve(small).rules.problems()[0].name, "max_memory_chars");
        let zero = ServeOptions {
            max_memory_entry_chars: Some(0),
            ..ServeOptions::NONE
        };
        assert_eq!(
            resolve(zero).rules.problems()[0].name,
            "max_memory_entry_chars"
        );
    }

    #[test]
    fn lane_limit_is_the_given_value_else_the_default() {
        let given = ServeOptions {
            max_lane_chars: Some(12000),
            ..ServeOptions::NONE
        };
        assert_eq!(given.lane_limit(), 12000);
        assert_eq!(
            ServeOptions::NONE.lane_limit(),
            kkt_rules::Settings::DEFAULT.max_lane_chars
        );
    }

    #[test]
    fn name_pack_name_is_the_given_name_unchecked_else_the_default() {
        let given = ServeOptions {
            name_pack: Some("missing".into()),
            ..ServeOptions::NONE
        };
        assert_eq!(given.name_pack_name(), "missing");
        assert_eq!(
            ServeOptions::NONE.name_pack_name(),
            kkt_rules::Settings::DEFAULT.name_pack.name()
        );
    }

    #[test]
    fn without_a_home_the_key_must_be_given_or_turned_off() {
        let err = ServeOptions::NONE
            .resolve(PathBuf::from("db"), None)
            .expect_err("no default key");
        assert_eq!(err.name, "access_key_file");
    }
}

#[cfg(test)]
mod waiting_tests {
    use super::*;
    #[test]
    fn grace_is_configurable_with_flag_precedence_and_checked() {
        let file = ServeOptions {
            waiting_grace_seconds: Some(4),
            ..ServeOptions::NONE
        };
        let flags = ServeOptions {
            waiting_grace_seconds: Some(2),
            ..ServeOptions::NONE
        };
        let settings = flags
            .over(file)
            .resolve(PathBuf::from("db"), Some(PathBuf::from("key")))
            .unwrap();
        assert_eq!(
            settings.rules.waiting_grace,
            std::time::Duration::from_secs(2)
        );
        assert!(settings.rules.problems().is_empty());
        let bad = kkt_rules::Settings {
            waiting_grace: std::time::Duration::from_secs(51),
            ..kkt_rules::Settings::DEFAULT
        };
        assert_eq!(bad.problems()[0].name, "waiting_grace");
        assert!(
            kkt_rules::Settings {
                waiting_grace: std::time::Duration::ZERO,
                ..kkt_rules::Settings::DEFAULT
            }
            .problems()
            .is_empty()
        );
    }
}
