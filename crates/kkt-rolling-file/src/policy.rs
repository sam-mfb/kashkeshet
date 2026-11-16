//! How the file is kept bounded, with no file I/O: the policy and its
//! check, write thresholds, rollover paths and file expiry.

use kkt_clock::{OutOfRange, Span, Time};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    pub(crate) fn check(&self) -> Result<(), SettingError> {
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

/// A policy the server refuses to start with: the setting's name and why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub(crate) struct SettingError {
    pub(crate) name: &'static str,
    pub(crate) rule: String,
}

impl SettingError {
    fn new(name: &'static str, rule: impl Into<String>) -> SettingError {
        SettingError {
            name,
            rule: rule.into(),
        }
    }
}

/// Prune on the first write after this interval, as well as on open/roll.
const LOG_PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Whether the next whole write needs a rollover. Empty files are never
/// rolled, even for a write larger than the limit.
pub(crate) fn should_roll(policy: LogPolicy, size: u64, next_bytes: usize) -> bool {
    size > 0 && size + next_bytes as u64 > policy.max_bytes
}

/// Whether age pruning is due on this write.
pub(crate) fn prune_due(now: Time, last_pruned: Time) -> Result<bool, OutOfRange> {
    Ok(now - last_pruned >= Span::from_std(LOG_PRUNE_INTERVAL)?)
}

/// `<file>.<n>`, the nth most recently rolled file.
fn rolled(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// The filesystem steps for one rollover, in their existing order.
pub(crate) struct Rotation {
    /// The oldest rolled file to remove before renaming.
    pub(crate) oldest: PathBuf,
    /// Source and destination pairs, oldest first.
    pub(crate) shifts: Vec<(PathBuf, PathBuf)>,
    /// Where the current file is renamed after the shifts.
    pub(crate) first: PathBuf,
}

/// The removal and reverse rename order for one rollover.
pub(crate) fn rotation(path: &Path, policy: LogPolicy) -> Rotation {
    Rotation {
        oldest: rolled(path, policy.rotated_files),
        shifts: (1..policy.rotated_files)
            .rev()
            .map(|n| (rolled(path, n), rolled(path, n + 1)))
            .collect(),
        first: rolled(path, 1),
    }
}

/// Which rolled files to inspect and when a file is too old.
pub(crate) struct Pruning {
    paths: Vec<PathBuf>,
    cutoff: Time,
}

impl Pruning {
    /// Rolled files to inspect, newest first.
    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
    /// Whether the modification time is strictly before the age cutoff.
    pub(crate) fn expired(&self, written: Time) -> bool {
        written < self.cutoff
    }
}

/// The rolled files and age cutoff to inspect at `now`.
pub(crate) fn pruning(path: &Path, policy: LogPolicy, now: Time) -> Result<Pruning, OutOfRange> {
    let max_age = Span::from_std(policy.max_age)?;
    Ok(Pruning {
        paths: (1..=policy.rotated_files)
            .map(|n| rolled(path, n))
            .collect(),
        cutoff: now - max_age,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const POLICY: LogPolicy = LogPolicy {
        max_bytes: 100,
        rotated_files: 3,
        max_age: Duration::from_secs(24 * 60 * 60),
    };

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
    fn writes_at_the_limit_fit_and_only_nonempty_files_roll() {
        assert!(!should_roll(POLICY, 0, 150));
        assert!(!should_roll(POLICY, 60, 40));
        assert!(should_roll(POLICY, 60, 41));
    }

    #[test]
    fn age_is_strict_and_pruning_runs_at_the_interval() {
        let now = Time::from_text("2026-11-11T00:00:00Z").expect("time");
        let plan = pruning(Path::new("kkt.log"), POLICY, now).expect("pruning");
        assert!(!plan.expired(now - Span::hours(24)));
        assert!(plan.expired(now - Span::hours(24) - Span::milliseconds(1)));
        assert!(!prune_due(now, now - Span::minutes(59)).expect("interval"));
        assert!(prune_due(now, now - Span::hours(1)).expect("interval"));
    }
}
