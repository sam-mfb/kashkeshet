//! The log level setting: how much is logged ([`LogLevel`]), read from
//! its name, and the levels used when nothing says.

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

/// The level logged for the program's own code when nothing says.
pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Info;
/// The level logged for the libraries a program uses when nothing says:
/// above info, since libraries log a great deal at info.
pub const DEFAULT_DEPENDENCY_LOG_LEVEL: LogLevel = LogLevel::Warn;

/// A log level setting refused: the setting's name, as the caller gave it,
/// and why.
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
    fn log_levels_parse_by_name_and_refuse_others() {
        for (level, name) in LogLevel::ALL {
            assert_eq!(LogLevel::parse("log_level", name), Ok(level));
            assert_eq!(level.name(), name);
        }
        let err = LogLevel::parse("dependency_log_level", "verbose").expect_err("unknown");
        assert_eq!(err.name, "dependency_log_level");
        assert!(LogLevel::parse("log_level", "INFO").is_err());
    }
}
