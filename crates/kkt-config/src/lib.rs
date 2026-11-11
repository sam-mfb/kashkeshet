//! Server limits and fixed settings. Every default is written out here, in
//! one place, along with the checks a server's settings must pass at startup.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

// ---- Server defaults and where files live ----
//
// What a server uses when neither a flag nor the config file says otherwise.

/// Address to listen on.
pub const DEFAULT_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7420));
/// Whether `bind` may be a non-loopback address.
pub const DEFAULT_ALLOW_ANY_BIND: bool = false;
/// Browser origins allowed to call `/mcp` (`allow_origins`): none.
pub const DEFAULT_ALLOW_ORIGINS: &[&str] = &[];
/// Host names the server answers to besides loopback (`allow_hosts`): none.
pub const DEFAULT_ALLOW_HOSTS: &[&str] = &[];
/// Database file name in the per-user data directory, for `kkt serve`.
pub const DB_FILE_NAME: &str = "kashkeshet.db";
/// The per-user data directory on macOS, under the home directory.
pub const MACOS_DATA_DIR: &str = "Library/Application Support/kashkeshet";
/// The per-user data directory elsewhere, under `$XDG_DATA_HOME`.
pub const XDG_DATA_DIR: &str = "kashkeshet";
/// `$XDG_DATA_HOME` when it is not set, under the home directory.
pub const XDG_DATA_HOME_DEFAULT: &str = ".local/share";
/// The config directory, under `$XDG_CONFIG_HOME`, on every platform.
pub const CONFIG_DIR: &str = "kkt";
/// `$XDG_CONFIG_HOME` when it is not set, under the home directory.
pub const XDG_CONFIG_HOME_DEFAULT: &str = ".config";
/// The config file `kkt serve` reads, in the config directory, if it exists.
pub const CONFIG_FILE_NAME: &str = "config.toml";

// ---- Logging ----

/// How much is logged: the named level and everything more severe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Every level, least verbose first, as named in flags and the config file.
    pub const ALL: [(LogLevel, &'static str); 5] = [
        (LogLevel::Error, "error"),
        (LogLevel::Warn, "warn"),
        (LogLevel::Info, "info"),
        (LogLevel::Debug, "debug"),
        (LogLevel::Trace, "trace"),
    ];

    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }

    /// The level named `text`, or an error naming `setting` and the choices.
    pub fn parse(setting: &'static str, text: &str) -> Result<LogLevel, InvalidSetting> {
        LogLevel::ALL
            .iter()
            .find(|(_, name)| *name == text)
            .map(|(level, _)| *level)
            .ok_or_else(|| {
                InvalidSetting::new(
                    setting,
                    format!("must be one of error, warn, info, debug, trace, not {text:?}"),
                )
            })
    }
}

/// The level logged for kkt's own code (`log_level`).
pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Info;
/// The level logged for the libraries kkt uses (`dependency_log_level`).
/// Above info, because the MCP library logs several lines for every request.
pub const DEFAULT_DEPENDENCY_LOG_LEVEL: LogLevel = LogLevel::Warn;

/// Checks `dependency_log_level`. At debug and trace the MCP library logs
/// whole requests and replies, tokens and recovery codes included, and logs
/// must never hold those; so those levels are refused.
pub fn check_dependency_log_level(level: LogLevel) -> Result<(), InvalidSetting> {
    match level {
        LogLevel::Error | LogLevel::Warn | LogLevel::Info => Ok(()),
        LogLevel::Debug | LogLevel::Trace => Err(InvalidSetting::new(
            "dependency_log_level",
            format!(
                "must be error, warn or info, not {}: at debug and trace the MCP library \
                 logs request contents, including tokens",
                level.name()
            ),
        )),
    }
}
/// How often a log file the server writes itself looks for rolled files past
/// their age, besides when it opens and when it rolls over. It looks on the
/// first write after this much time, so a server that logs nothing deletes
/// nothing until it next logs.
pub const LOG_PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Smallest allowed [`LogPolicy::max_bytes`].
pub const MIN_LOG_FILE_BYTES: u64 = 64 * 1024;

/// How a log file the server writes itself (`--log-file`) is kept bounded.
/// When the file would grow past `max_bytes`, it is renamed `<file>.1`, the
/// older ones move up to `<file>.<rotated_files>`, and the oldest is
/// deleted. Rolled files older than `max_age` are deleted too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogPolicy {
    pub max_bytes: u64,
    pub rotated_files: usize,
    pub max_age: Duration,
}

impl LogPolicy {
    /// At most about 60 MB: the current file and five rolled ones of 10 MB,
    /// none rolled more than 30 days ago.
    pub const DEFAULT: LogPolicy = LogPolicy {
        max_bytes: 10 * 1024 * 1024,
        rotated_files: 5,
        max_age: Duration::from_secs(30 * 24 * 60 * 60),
    };

    /// Checks that the policy bounds the log. The server refuses to start
    /// with one that fails.
    pub fn check(&self) -> Result<(), InvalidSetting> {
        if self.max_bytes < MIN_LOG_FILE_BYTES {
            return Err(InvalidSetting::new(
                "log_max_bytes",
                format!("must be at least {MIN_LOG_FILE_BYTES}"),
            ));
        }
        if self.rotated_files == 0 {
            return Err(InvalidSetting::new(
                "log_rotated_files",
                "must be at least 1",
            ));
        }
        if self.max_age < Duration::from_secs(60 * 60) {
            return Err(InvalidSetting::new(
                "log_max_age",
                "must be at least an hour",
            ));
        }
        Ok(())
    }
}

/// A setting the server refuses to start with.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct InvalidSetting {
    pub name: &'static str,
    pub rule: String,
}

impl InvalidSetting {
    pub fn new(name: &'static str, rule: impl Into<String>) -> InvalidSetting {
        InvalidSetting {
            name,
            rule: rule.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_log_policy_passes_and_bad_ones_are_named() {
        assert_eq!(LogPolicy::DEFAULT.check(), Ok(()));
        let cases = [
            (
                LogPolicy {
                    max_bytes: MIN_LOG_FILE_BYTES - 1,
                    ..LogPolicy::DEFAULT
                },
                "log_max_bytes",
            ),
            (
                LogPolicy {
                    rotated_files: 0,
                    ..LogPolicy::DEFAULT
                },
                "log_rotated_files",
            ),
            (
                LogPolicy {
                    max_age: Duration::from_secs(59 * 60),
                    ..LogPolicy::DEFAULT
                },
                "log_max_age",
            ),
        ];
        for (policy, name) in cases {
            assert_eq!(policy.check().map_err(|e| e.name), Err(name));
        }
    }

    #[test]
    fn log_levels_parse_by_name_and_refuse_others() {
        for (level, name) in LogLevel::ALL {
            assert_eq!(LogLevel::parse("log_level", name), Ok(level));
            assert_eq!(level.name(), name);
        }
        let err = LogLevel::parse("dependency_log_level", "verbose").expect_err("unknown");
        assert_eq!(err.name, "dependency_log_level");
        assert!(LogLevel::parse("log_level", "INFO").is_err());
    }

    #[test]
    fn dependency_levels_that_would_log_secrets_are_refused() {
        for level in [LogLevel::Error, LogLevel::Warn, LogLevel::Info] {
            assert_eq!(check_dependency_log_level(level), Ok(()));
        }
        for level in [LogLevel::Debug, LogLevel::Trace] {
            let err = check_dependency_log_level(level).expect_err("refused");
            assert_eq!(err.name, "dependency_log_level");
        }
    }
}
