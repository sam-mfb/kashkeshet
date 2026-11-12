//! The config file's format: TOML with one key per server setting, named as
//! the flag is with `_` for `-`. The lists are `allow_origins` and
//! `allow_hosts`.
//!
//! ```toml
//! db = "~/kkt/kashkeshet.db"
//! bind = "127.0.0.1:7420"
//! allow_hosts = ["host.docker.internal"]
//! access_key_file = "access-key"
//! # or, to run with no key at all:
//! # no_access_key = true
//! ```
//!
//! A path starting with `~/` is under the home directory; any other relative
//! path is relative to the config file's directory. An unknown key or a
//! value of the wrong kind is refused, naming the key.
//!
//! This crate owns the file's format and where it is by default
//! ([`config_file`], under the supplied config-home folder): parsing its text into
//! [`Config`], what the file says in its own terms, and adding an allowed
//! host to its text. It reads and writes no file (kkt-config-file-os
//! does) and knows nothing of the settings' defaults or checks beyond the
//! kind of each value: the log levels are kept as written, for the reader to
//! check.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// The config directory, under `$XDG_CONFIG_HOME`, on every platform.
pub const CONFIG_DIR: &str = "kkt";
/// The config file `kkt serve` reads, in the config directory, if it exists.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// kkt's config directory: `kkt` under the supplied config-home folder.
/// With no config home there is no default config directory.
pub fn config_dir(config_home: Option<&Path>) -> Option<PathBuf> {
    Some(config_home?.join(CONFIG_DIR))
}

/// The config file `kkt serve` reads when it exists and no `--config` is
/// given.
pub fn config_file(config_home: Option<&Path>) -> Option<PathBuf> {
    Some(config_dir(config_home)?.join(CONFIG_FILE_NAME))
}

/// Why a config file's text could not be read as a config file: the setting
/// (`config` for the file as a whole) and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct ConfigError {
    pub name: &'static str,
    pub rule: String,
}

impl ConfigError {
    fn new(name: &'static str, rule: impl Into<String>) -> ConfigError {
        ConfigError {
            name,
            rule: rule.into(),
        }
    }
}

/// What a config file says, in the file's own terms: each setting it gives,
/// or `None`. Paths are resolved as the file says to. The log levels are
/// `L`, whatever the caller's level check in [`parse_file`] makes of them:
/// which levels there are is not this crate's to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config<L> {
    pub name_pack: Option<String>,
    pub db: Option<PathBuf>,
    pub bind: Option<SocketAddr>,
    pub allow_any_bind: Option<bool>,
    pub max_response_bytes: Option<usize>,
    pub default_limit: Option<usize>,
    pub max_limit: Option<usize>,
    pub sends_per_minute: Option<u32>,
    pub waiting_grace_seconds: Option<u32>,
    pub upgrade_backups_kept: Option<usize>,
    pub max_lane_chars: Option<usize>,
    pub max_memory_entry_chars: Option<usize>,
    pub max_memory_chars: Option<usize>,
    pub allow_origins: Option<Vec<String>>,
    pub allow_hosts: Option<Vec<String>>,
    pub instructions: Option<PathBuf>,
    pub access_key_file: Option<PathBuf>,
    pub no_access_key: Option<bool>,
    pub log_file: Option<PathBuf>,
    pub log_level: Option<L>,
    pub dependency_log_level: Option<L>,
}

impl<L> Config<L> {
    /// A file that says nothing.
    pub const NONE: Config<L> = Config {
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
}

/// The settings in a config file's `text`. `dir` is the directory the file
/// is in, for relative paths; `home` is the home folder, for `~/` paths,
/// if there is one. `level` checks a log level as written, given the
/// setting's name, and says what is wrong with it if it is not one. Settings
/// are checked in the order of their names, so the first bad one is named.
pub fn parse_file<L>(
    text: &str,
    dir: &Path,
    home: Option<&Path>,
    level: impl Fn(&'static str, &str) -> Result<L, String>,
) -> Result<Config<L>, ConfigError> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        ConfigError::new(
            "config",
            format!("is not valid TOML: {}", e.message().trim()),
        )
    })?;
    let mut config = Config::NONE;
    for (key, value) in table {
        let value = Value(value);
        match key.as_str() {
            "name_pack" => {
                config.name_pack = Some(
                    value
                        .string("name_pack", "a pack name in quotes")?
                        .to_owned(),
                )
            }
            "db" => config.db = Some(value.path("db", dir, home)?),
            "bind" => config.bind = Some(value.address("bind")?),
            "allow_any_bind" => config.allow_any_bind = Some(value.boolean("allow_any_bind")?),
            "max_response_bytes" => {
                config.max_response_bytes = Some(value.count("max_response_bytes")?)
            }
            "default_limit" => config.default_limit = Some(value.count("default_limit")?),
            "max_limit" => config.max_limit = Some(value.count("max_limit")?),
            "waiting_grace_seconds" => {
                config.waiting_grace_seconds = Some(value.count("waiting_grace_seconds")?)
            }
            "sends_per_minute" => config.sends_per_minute = Some(value.count("sends_per_minute")?),
            "upgrade_backups_kept" => {
                config.upgrade_backups_kept = Some(value.count("upgrade_backups_kept")?)
            }
            "max_lane_chars" => config.max_lane_chars = Some(value.count("max_lane_chars")?),
            "max_memory_entry_chars" => {
                config.max_memory_entry_chars = Some(value.count("max_memory_entry_chars")?)
            }
            "max_memory_chars" => config.max_memory_chars = Some(value.count("max_memory_chars")?),
            "allow_origins" => config.allow_origins = Some(value.strings("allow_origins")?),
            "allow_hosts" => config.allow_hosts = Some(value.strings("allow_hosts")?),
            "instructions" => config.instructions = Some(value.path("instructions", dir, home)?),
            "access_key_file" => {
                config.access_key_file = Some(value.path("access_key_file", dir, home)?)
            }
            "no_access_key" => config.no_access_key = Some(value.boolean("no_access_key")?),
            "log_file" => config.log_file = Some(value.path("log_file", dir, home)?),
            "log_level" => config.log_level = Some(value.log_level("log_level", &level)?),
            "dependency_log_level" => {
                config.dependency_log_level = Some(value.log_level("dependency_log_level", &level)?)
            }
            _ => {
                return Err(ConfigError::new(
                    "config",
                    format!("has an unknown setting {key:?}"),
                ));
            }
        }
    }
    Ok(config)
}

/// The config file `text` with `host` added to `allow_hosts`, keeping the
/// rest of the file, comments included, as it was. Unchanged if the host is
/// already there. Empty `text` makes a file with only that line.
pub fn with_allowed_host(text: &str, host: &str) -> Result<String, ConfigError> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        ConfigError::new(
            "config",
            format!("is not valid TOML: {}", e.message().trim()),
        )
    })?;
    let hosts = doc
        .entry("allow_hosts")
        .or_insert_with(|| toml_edit::value(toml_edit::Array::new()))
        .as_array_mut()
        .ok_or_else(|| ConfigError::new("allow_hosts", "in the config file must be a list"))?;
    if !hosts.iter().any(|h| h.as_str() == Some(host)) {
        hosts.push(host);
    }
    Ok(doc.to_string())
}

/// One value from the file.
struct Value(toml::Value);

impl Value {
    fn wrong(&self, name: &'static str, expected: &str) -> ConfigError {
        ConfigError::new(
            name,
            format!("in the config file must be {expected}, not {}", self.0),
        )
    }

    fn string(&self, name: &'static str, expected: &str) -> Result<&str, ConfigError> {
        self.0.as_str().ok_or_else(|| self.wrong(name, expected))
    }

    fn path(
        &self,
        name: &'static str,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<PathBuf, ConfigError> {
        let text = self.string(name, "a path in quotes")?;
        if text.is_empty() {
            return Err(self.wrong(name, "a path in quotes"));
        }
        if let Some(rest) = text.strip_prefix("~/") {
            return match home {
                Some(home) => Ok(home.join(rest)),
                None => Err(ConfigError::new(
                    name,
                    "in the config file starts with ~/, but no home directory is set",
                )),
            };
        }
        Ok(dir.join(text))
    }

    /// A log level, as the caller's `level` reads it.
    fn log_level<L>(
        &self,
        name: &'static str,
        level: impl Fn(&'static str, &str) -> Result<L, String>,
    ) -> Result<L, ConfigError> {
        level(
            name,
            self.string(name, "a level in quotes, such as \"info\"")?,
        )
        .map_err(|rule| ConfigError::new(name, rule))
    }

    fn boolean(&self, name: &'static str) -> Result<bool, ConfigError> {
        self.0
            .as_bool()
            .ok_or_else(|| self.wrong(name, "true or false"))
    }

    fn address(&self, name: &'static str) -> Result<SocketAddr, ConfigError> {
        let expected = "an address in quotes, such as \"127.0.0.1:7420\"";
        self.string(name, expected)?
            .parse()
            .map_err(|_| self.wrong(name, expected))
    }

    fn count<T: TryFrom<i64>>(&self, name: &'static str) -> Result<T, ConfigError> {
        let expected = "a whole number of zero or more";
        let n = self
            .0
            .as_integer()
            .ok_or_else(|| self.wrong(name, expected))?;
        T::try_from(n).map_err(|_| self.wrong(name, expected))
    }

    fn strings(&self, name: &'static str) -> Result<Vec<String>, ConfigError> {
        let expected = "a list of strings, such as [\"a\", \"b\"]";
        let items = self
            .0
            .as_array()
            .ok_or_else(|| self.wrong(name, expected))?;
        items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| self.wrong(name, expected))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any level is taken as written.
    fn parse(text: &str) -> Result<Config<String>, ConfigError> {
        parse_file(
            text,
            Path::new("/etc/kkt"),
            Some(Path::new("/home/sam")),
            |_, text| Ok(text.to_owned()),
        )
    }

    #[test]
    fn an_empty_file_gives_nothing() {
        assert_eq!(parse(""), Ok(Config::NONE));
    }

    #[test]
    fn every_setting_can_be_given() {
        let options = parse(
            r#"
            db = "~/data/kkt.db"
            name_pack = "garden"
            bind = "127.0.0.1:9000"
            allow_any_bind = true
            max_response_bytes = 4096
            default_limit = 10
            max_limit = 40
            sends_per_minute = 5
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
        )
        .expect("valid file");
        assert_eq!(
            options,
            Config {
                name_pack: Some("garden".into()),
                waiting_grace_seconds: None,
                upgrade_backups_kept: None,
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
                log_level: Some("debug".into()),
                dependency_log_level: Some("info".into()),
            }
        );
    }

    #[test]
    fn unknown_keys_are_refused_naming_them() {
        let err = parse("bnid = \"127.0.0.1:1\"").expect_err("unknown key");
        assert_eq!(err.name, "config");
        assert!(err.rule.contains("\"bnid\""), "{}", err.rule);
        // Flag spellings with dashes are not keys.
        let err = parse("max-limit = 5").expect_err("dashed key");
        assert!(err.rule.contains("\"max-limit\""), "{}", err.rule);
    }

    #[test]
    fn bad_values_are_refused_naming_the_setting() {
        let cases = [
            ("name_pack = 1", "name_pack"),
            ("bind = \"not an address\"", "bind"),
            ("bind = 7420", "bind"),
            ("max_limit = -1", "max_limit"),
            ("max_limit = \"5\"", "max_limit"),
            ("sends_per_minute = 5000000000", "sends_per_minute"),
            ("max_lane_chars = -1", "max_lane_chars"),
            ("max_lane_chars = \"12000\"", "max_lane_chars"),
            ("max_memory_entry_chars = -1", "max_memory_entry_chars"),
            ("max_memory_chars = \"9\"", "max_memory_chars"),
            ("allow_hosts = \"one.example\"", "allow_hosts"),
            ("allow_origins = [1]", "allow_origins"),
            ("no_access_key = \"yes\"", "no_access_key"),
            ("allow_any_bind = \"yes\"", "allow_any_bind"),
            ("dependency_log_level = 3", "dependency_log_level"),
            ("db = \"\"", "db"),
            ("access_key_file = 3", "access_key_file"),
        ];
        for (text, name) in cases {
            let err = parse(text).expect_err(text);
            assert_eq!(err.name, name, "{text}");
        }
    }

    #[test]
    fn log_levels_are_what_the_callers_check_makes_of_them() {
        let config = parse("log_level = \"loud\"").expect("a string");
        assert_eq!(config.log_level, Some("loud".into()));
        let refuse = |_, text: &str| Err::<(), _>(format!("is not a level: {text}"));
        let err =
            parse_file("log_level = \"loud\"", Path::new("/"), None, refuse).expect_err("refused");
        assert_eq!(err.name, "log_level");
        assert_eq!(err.to_string(), "log_level is not a level: loud");
    }

    #[test]
    fn of_two_bad_settings_the_first_by_name_is_named() {
        let refuse = |_, _: &str| Err::<(), _>("is not a level".to_string());
        let parse = |text| parse_file(text, Path::new("/"), None, refuse).expect_err(text);
        // Whichever comes first in the file, the first by name is named,
        // log levels included.
        assert_eq!(parse("max_limit = -1\nlog_level = \"x\"").name, "log_level");
        assert_eq!(parse("log_level = \"x\"\nmax_limit = -1").name, "log_level");
        assert_eq!(parse("bind = 1\nlog_level = \"x\"").name, "bind");
        assert_eq!(parse("dependency_log_level = \"x\"\nbind = 1").name, "bind");
        assert_eq!(
            parse("log_level = \"x\"\ndependency_log_level = \"y\"").name,
            "dependency_log_level"
        );
    }

    #[test]
    fn broken_toml_is_refused() {
        let err = parse("bind = ").expect_err("broken");
        assert_eq!(err.name, "config");
    }

    #[test]
    fn home_paths_need_a_home() {
        let err = parse_file(
            "db = \"~/x.db\"",
            Path::new("/etc/kkt"),
            None,
            |_, _| Ok(()),
        )
        .expect_err("no home");
        assert_eq!(err.name, "db");
    }

    #[test]
    fn an_allowed_host_is_added_keeping_the_rest_of_the_file() {
        let text = "# my settings\nbind = \"127.0.0.1:7420\" # local\n";
        let added = with_allowed_host(text, "box.tail1234.ts.net").expect("added");
        assert_eq!(
            added,
            "# my settings\nbind = \"127.0.0.1:7420\" # local\nallow_hosts = [\"box.tail1234.ts.net\"]\n"
        );
        let again = with_allowed_host(&added, "box.tail1234.ts.net").expect("again");
        assert_eq!(again, added);
        let more = with_allowed_host("allow_hosts = [\"a\"]\n", "b").expect("more");
        assert_eq!(more, "allow_hosts = [\"a\", \"b\"]\n");
        assert_eq!(
            with_allowed_host("", "b").expect("new"),
            "allow_hosts = [\"b\"]\n"
        );
        let bad = with_allowed_host("allow_hosts = \"a\"\n", "b").expect_err("not a list");
        assert_eq!(bad.name, "allow_hosts");
    }

    #[test]
    fn the_config_paths_are_under_the_supplied_folder() {
        let folder = Some(Path::new("/home/sam/.config"));
        assert_eq!(
            config_dir(folder),
            Some(PathBuf::from("/home/sam/.config/kkt"))
        );
        assert_eq!(
            config_file(folder),
            Some(PathBuf::from("/home/sam/.config/kkt/config.toml"))
        );
        assert_eq!(config_dir(None), None);
        assert_eq!(config_file(None), None);
    }
}
