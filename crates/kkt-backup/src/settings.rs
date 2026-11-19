//! The backup retention policy's settings, checks and run-record wording.

/// How many completed backups to keep when upgrading a database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub upgrade_backups_kept: usize,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        upgrade_backups_kept: 5,
    };

    /// Every invalid setting, with the check that found it. The caller
    /// chooses which problem to report alongside other owners' problems.
    pub fn problems(&self) -> Vec<Problem> {
        if self.upgrade_backups_kept == 0 {
            vec![Problem {
                name: "upgrade_backups_kept",
                check: "floor",
                rule: "must be at least 1".into(),
            }]
        } else {
            Vec::new()
        }
    }

    /// The retention setting as the run record words it.
    pub fn record_lines(&self) -> Vec<(String, String)> {
        let Self {
            upgrade_backups_kept,
        } = self;
        vec![(
            "upgrade_backups_kept".into(),
            upgrade_backups_kept.to_string(),
        )]
    }
}

/// An invalid backup setting and the owner's check that found it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct Problem {
    pub name: &'static str,
    pub check: &'static str,
    pub rule: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_keeps_the_existing_floor_and_refusal() {
        let problems = Settings {
            upgrade_backups_kept: 0,
        }
        .problems();
        assert_eq!(
            problems,
            vec![Problem {
                name: "upgrade_backups_kept",
                check: "floor",
                rule: "must be at least 1".into(),
            }]
        );
        assert_eq!(
            problems[0].to_string(),
            "upgrade_backups_kept must be at least 1"
        );
        for settings in [
            Settings::DEFAULT,
            Settings {
                upgrade_backups_kept: 1,
            },
        ] {
            assert!(settings.problems().is_empty());
        }
    }

    #[test]
    fn retention_keeps_the_run_records_name_and_value() {
        assert_eq!(
            Settings::DEFAULT.record_lines(),
            vec![("upgrade_backups_kept".into(), "5".into())]
        );
        assert_eq!(
            Settings {
                upgrade_backups_kept: 9
            }
            .record_lines(),
            vec![("upgrade_backups_kept".into(), "9".into())]
        );
    }
}
