//! The log level setting: a [`Level`] read from its name
//! ([`parse_level`]), and the levels used when nothing says.

use kkt_log::Level;

/// The level named `text`, as a setting such as `log_level` gives it, or an
/// error naming `setting` and the choices.
pub fn parse_level(setting: &'static str, text: &str) -> Result<Level, SettingError> {
    Level::ALL
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

/// The level logged for the program's own code when nothing says.
pub const DEFAULT_LOG_LEVEL: Level = Level::Info;
/// The level logged for the libraries a program uses when nothing says:
/// above info, since libraries log a great deal at info.
pub const DEFAULT_DEPENDENCY_LOG_LEVEL: Level = Level::Warn;

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
        for (level, name) in Level::ALL {
            assert_eq!(parse_level("log_level", name), Ok(level));
            assert_eq!(level.name(), name);
        }
        let err = parse_level("dependency_log_level", "verbose").expect_err("unknown");
        assert_eq!(err.name, "dependency_log_level");
        assert!(parse_level("log_level", "INFO").is_err());
    }
}
