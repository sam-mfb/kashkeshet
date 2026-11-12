//! Log levels, their checks, and pure rollover and age-pruning rules.
//! The native writer applies these plans to the files on disk.

mod policy;
pub use policy::{Pruning, Rotation, prune_due, pruning, rotation, should_roll};

use std::time::Duration;

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
    pub fn parse(setting: &'static str, text: &str) -> Result<LogLevel, SettingError> {
        LogLevel::ALL
            .iter()
            .find(|(_, name)| *name == text)
            .map(|(level, _)| *level)
            .ok_or_else(|| {
                SettingError::new(
                    setting,
                    format!("must be one of error, warn, info, debug, trace, not {text:?}"),
                )
            })
    }
}

/// The level logged for kkt's own code (`log_level`) when nothing says.
pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Info;
/// The level logged for the libraries kkt uses (`dependency_log_level`)
/// when nothing says. Above info, because the MCP library logs several
/// lines for every request.
pub const DEFAULT_DEPENDENCY_LOG_LEVEL: LogLevel = LogLevel::Warn;

/// Checks `dependency_log_level`. At debug and trace the MCP library logs
/// whole requests and replies, tokens and recovery codes included, and logs
/// must never hold those; so those levels are refused.
pub fn check_dependency_log_level(level: LogLevel) -> Result<(), SettingError> {
    match level {
        LogLevel::Error | LogLevel::Warn | LogLevel::Info => Ok(()),
        LogLevel::Debug | LogLevel::Trace => Err(SettingError::new(
            "dependency_log_level",
            format!(
                "must be error, warn or info, not {}: at debug and trace the MCP library \
                 logs request contents, including tokens",
                level.name()
            ),
        )),
    }
}

/// Smallest allowed [`LogPolicy::max_bytes`].
const MIN_LOG_FILE_BYTES: u64 = 64 * 1024;

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
    pub fn check(&self) -> Result<(), SettingError> {
        if self.max_bytes < MIN_LOG_FILE_BYTES {
            return Err(SettingError::new(
                "log_max_bytes",
                format!("must be at least {MIN_LOG_FILE_BYTES}"),
            ));
        }
        if self.rotated_files == 0 {
            return Err(SettingError::new("log_rotated_files", "must be at least 1"));
        }
        if self.max_age < Duration::from_secs(60 * 60) {
            return Err(SettingError::new("log_max_age", "must be at least an hour"));
        }
        Ok(())
    }
}

/// A log setting the server refuses to start with: the setting's name and
/// why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct SettingError {
    pub name: &'static str,
    pub rule: String,
}

impl SettingError {
    pub fn new(name: &'static str, rule: impl Into<String>) -> SettingError {
        SettingError {
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
