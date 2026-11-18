//! A file on disk that rolls over at a size and prunes old copies by age
//! ([`RotatingFile`], opened by [`open_log_file`] after checking its
//! [`LogPolicy`]), such as a log. The rules that keep it bounded are pure
//! plans, in `policy`, carried out here. The caller supplies the wall clock,
//! and words the refusals ([`OpenError`]) in its own terms.

mod policy;
pub use policy::{LogPolicy, PolicyProblem, PolicyValue};

use kkt_clock::{Clock, Time};
use policy::{prune_due, pruning, rotation, should_roll};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Why [`open_log_file`] refused. Its own text is plain; a caller words
/// the refusal in its own terms, such as the setting each value comes from.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The policy does not bound the file.
    #[error("the policy's {:?} {}", .0.value, .0.rule)]
    Policy(PolicyProblem),
    /// The file at `path` could not be opened, for the system's reason.
    #[error("could not open {}: {error}", path.display())]
    File { path: PathBuf, error: io::Error },
}

/// Opens the file at `path` for [`RotatingFile::open`], after checking
/// `policy`.
pub fn open_log_file(
    path: &Path,
    policy: LogPolicy,
    clock: Arc<dyn Clock>,
) -> Result<RotatingFile, OpenError> {
    policy.check().map_err(OpenError::Policy)?;
    RotatingFile::open(path, policy, clock).map_err(|error| OpenError::File {
        path: path.to_owned(),
        error,
    })
}

/// The log file at `path`, appended to, and rolled over by its policy.
pub struct RotatingFile {
    path: PathBuf,
    policy: LogPolicy,
    clock: Arc<dyn Clock>,
    file: File,
    size: u64,
    /// When rolled files were last checked for age.
    pruned_at: Time,
}

impl RotatingFile {
    /// Opens `path` for appending, creating it and its folder if needed, and
    /// deletes rolled files that are past the policy's age.
    pub(crate) fn open(
        path: &Path,
        policy: LogPolicy,
        clock: Arc<dyn Clock>,
    ) -> io::Result<RotatingFile> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let file = append(path)?;
        let size = file.metadata()?.len();
        let pruned_at = clock.now();
        let mut log = RotatingFile {
            path: path.to_path_buf(),
            policy,
            clock,
            file,
            size,
            pruned_at,
        };
        log.prune()?;
        Ok(log)
    }

    /// `<file>.<n>`, the `n`th most recently rolled file.
    #[cfg(test)]
    fn rolled(&self, n: usize) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    /// Renames the file to `<file>.1`, moving older ones up and dropping the
    /// oldest, then starts a new, empty file.
    fn roll(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let plan = rotation(&self.path, self.policy);
        remove_if_present(&plan.oldest)?;
        for (from, to) in plan.shifts {
            rename_if_present(&from, &to)?;
        }
        std::fs::rename(&self.path, plan.first)?;
        self.file = append(&self.path)?;
        self.size = 0;
        self.prune()
    }

    /// Deletes rolled files last written more than the policy's age ago.
    fn prune(&mut self) -> io::Result<()> {
        let now = self.clock.now();
        let plan = pruning(&self.path, self.policy, now).map_err(invalid_policy)?;
        self.pruned_at = now;
        for rolled in plan.paths() {
            let written = match std::fs::metadata(rolled) {
                Ok(meta) => Time::from(meta.modified()?),
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if plan.expired(written) {
                remove_if_present(rolled)?;
            }
        }
        Ok(())
    }
}

impl Write for RotatingFile {
    /// Writes `buf` to the file, rolling it over first if `buf` would take
    /// it past the policy's size. A file is never rolled while empty, so one
    /// write larger than the limit still lands whole. Otherwise, at most
    /// periodically, as [`prune_due`] decides, deletes rolled
    /// files past their age.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if should_roll(self.policy, self.size, buf.len()) {
            self.roll()?;
        } else if prune_due(self.clock.now(), self.pruned_at).map_err(invalid_policy)? {
            self.prune()?;
        }
        let written = self.file.write(buf)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn rename_if_present(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn invalid_policy(error: kkt_clock::OutOfRange) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

/// Where unit tests put their scratch files: shared with the integration tests.
#[cfg(test)]
use kkt_scratch as scratch;

#[cfg(test)]
mod tests;
