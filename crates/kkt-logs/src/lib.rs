//! Log rollover and pruning choices, with no machine work. Settings come
//! from the log's owner; the native writer applies these plans to files.

use kkt_clock::{OutOfRange, Span, Time};
use kkt_config::{LOG_PRUNE_INTERVAL, LogPolicy};
use std::path::{Path, PathBuf};

/// Whether the next whole write needs a rollover. Empty files are never
/// rolled, even for a write larger than the limit.
pub fn should_roll(policy: LogPolicy, size: u64, next_bytes: usize) -> bool {
    size > 0 && size + next_bytes as u64 > policy.max_bytes
}

/// Whether age pruning is due on this write.
pub fn prune_due(now: Time, last_pruned: Time) -> Result<bool, OutOfRange> {
    Ok(now - last_pruned >= Span::from_std(LOG_PRUNE_INTERVAL)?)
}

/// `<file>.<n>`, the nth most recently rolled file.
pub fn rolled(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// The filesystem steps for one rollover, in their existing order.
pub struct Rotation {
    pub oldest: PathBuf,
    pub shifts: Vec<(PathBuf, PathBuf)>,
    pub first: PathBuf,
}

pub fn rotation(path: &Path, policy: LogPolicy) -> Rotation {
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
pub struct Pruning {
    paths: Vec<PathBuf>,
    cutoff: Time,
}

impl Pruning {
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
    pub fn expired(&self, written: Time) -> bool {
        written < self.cutoff
    }
}

pub fn pruning(path: &Path, policy: LogPolicy, now: Time) -> Result<Pruning, OutOfRange> {
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
    fn writes_at_the_limit_fit_and_only_nonempty_files_roll() {
        assert!(!should_roll(POLICY, 0, 150));
        assert!(!should_roll(POLICY, 60, 40));
        assert!(should_roll(POLICY, 60, 41));
    }

    #[test]
    fn age_is_strict_and_pruning_runs_at_the_interval() {
        let now = Time::from_parts(2026, 11, 11, 0, 0, 0).expect("time");
        let plan = pruning(Path::new("kkt.log"), POLICY, now).expect("pruning");
        assert!(!plan.expired(now - Span::hours(24)));
        assert!(plan.expired(now - Span::hours(24) - Span::milliseconds(1)));
        assert!(!prune_due(now, now - Span::minutes(59)).expect("interval"));
        assert!(prune_due(now, now - Span::hours(1)).expect("interval"));
    }
}
