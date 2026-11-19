//! [`Memory`], backups in memory, for tests.

use crate::{BackupError, Backups, Layout, old_names};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Backups in memory: what each folder holds, by name, and a failure on
/// cue. It keeps the contract's rules a caller can see: a name already
/// there is refused, and pruning removes what [`old_names`] picks.
#[derive(Default)]
pub struct Memory {
    folders: RefCell<BTreeMap<PathBuf, BTreeMap<String, Written>>>,
    failing: RefCell<Option<Box<dyn Fn() -> BackupError>>>,
}

/// A backup [`Memory`] holds: what its layout said to copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The database copied.
    pub database: PathBuf,
    /// Each other file copied: its name in the backup, its source, and
    /// whether it is private.
    pub copies: Vec<(&'static str, PathBuf, bool)>,
    /// The about file's text.
    pub about: String,
}

impl Memory {
    pub fn new() -> Memory {
        Memory::default()
    }

    /// Puts a finished or unfinished backup named `name` in `folder`, as
    /// if it were already there.
    pub fn with_backup(self, folder: impl Into<PathBuf>, name: &str) -> Memory {
        self.folders
            .borrow_mut()
            .entry(folder.into())
            .or_default()
            .insert(
                name.to_owned(),
                Written {
                    database: PathBuf::new(),
                    copies: Vec::new(),
                    about: String::new(),
                },
            );
        self
    }

    /// Makes every write and prune fail with the error `error` makes.
    pub fn failing(self, error: impl Fn() -> BackupError + 'static) -> Memory {
        *self.failing.borrow_mut() = Some(Box::new(error));
        self
    }

    /// The names of the backups in `folder`, in order.
    pub fn names(&self, folder: &Path) -> Vec<String> {
        self.folders
            .borrow()
            .get(folder)
            .map(|backups| backups.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The backup named `name` in `folder`, if there is one.
    pub fn backup(&self, folder: &Path, name: &str) -> Option<Written> {
        self.folders.borrow().get(folder)?.get(name).cloned()
    }

    fn check(&self) -> Result<(), BackupError> {
        match self.failing.borrow().as_ref() {
            Some(error) => Err(error()),
            None => Ok(()),
        }
    }
}

impl Backups for Memory {
    fn write(&self, folder: &Path, layout: &Layout<'_>) -> Result<PathBuf, BackupError> {
        self.check()?;
        let done = folder.join(layout.name());
        let mut folders = self.folders.borrow_mut();
        let backups = folders.entry(folder.to_owned()).or_default();
        if backups.contains_key(layout.name()) {
            return Err(BackupError::Exists { path: done });
        }
        backups.insert(
            layout.name().to_owned(),
            Written {
                database: layout.database().to_owned(),
                copies: layout
                    .copies()
                    .iter()
                    .map(|copy| (copy.name, copy.source.to_owned(), copy.private))
                    .collect(),
                about: layout.about().to_owned(),
            },
        );
        Ok(done)
    }

    fn record_highest(&self, backup: &Path, highest: i64) -> Result<(), BackupError> {
        self.check()?;
        let mut folders = self.folders.borrow_mut();
        for (folder, backups) in folders.iter_mut() {
            for (name, written) in backups.iter_mut() {
                if folder.join(name) == backup {
                    written.about.push_str(&crate::highest_line(highest));
                    return Ok(());
                }
            }
        }
        Err(BackupError::file(
            crate::FileAction::Write,
            backup,
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ))
    }

    fn prune(&self, folder: &Path, keep: usize) -> Result<Vec<PathBuf>, BackupError> {
        self.check()?;
        let mut folders = self.folders.borrow_mut();
        let Some(backups) = folders.get_mut(folder) else {
            return Ok(Vec::new());
        };
        let old = old_names(backups.keys().cloned().collect(), keep);
        Ok(old
            .into_iter()
            .map(|name| {
                backups.remove(&name);
                folder.join(name)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Contents, layout, name};
    use kkt_clock::Time;

    fn at(text: &str) -> Time {
        Time::from_text(text).expect("time")
    }

    #[test]
    fn a_written_backup_is_kept_by_name_and_a_second_of_that_name_is_refused() {
        let backups = Memory::new();
        let folder = Path::new("/b");
        let contents = Contents {
            database: Path::new("/d/kashkeshet.db"),
            config: None,
            access_key: Some(Path::new("/k/access-key")),
            web_key: None,
            admin_key: None,
            admin_name: None,
        };
        let layout = layout(at("2026-11-16T10:00:00Z"), "1.0.0", &contents);
        let written = backups.write(folder, &layout).expect("written");
        assert_eq!(written, folder.join(layout.name()));
        let kept = backups.backup(folder, layout.name()).expect("kept");
        assert_eq!(kept.database, PathBuf::from("/d/kashkeshet.db"));
        assert_eq!(
            kept.copies,
            [("access-key", PathBuf::from("/k/access-key"), true)]
        );
        assert_eq!(
            backups.write(folder, &layout).map_err(|e| e.to_string()),
            Err(format!("{} already exists", written.display()))
        );
    }

    #[test]
    fn pruning_removes_the_oldest_finished_backups_only() {
        let folder = Path::new("/b");
        let day = |d: u32| name(at(&format!("2026-11-{d}T10:00:00Z")), "1.0.0");
        let backups = Memory::new()
            .with_backup(folder, &day(14))
            .with_backup(folder, &day(15))
            .with_backup(folder, &day(16))
            .with_backup(folder, &format!("{}.partial", day(13)))
            .with_backup(folder, "notes");
        let removed = backups.prune(folder, 2).expect("pruned");
        assert_eq!(removed, [folder.join(day(14))]);
        assert_eq!(backups.names(folder).len(), 4);
        assert_eq!(
            backups
                .prune(Path::new("/none"), 1)
                .map_err(|e| e.to_string()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn a_failure_on_cue_fails_both_operations() {
        let backups = Memory::new().failing(|| {
            BackupError::file(
                crate::FileAction::Write,
                "/b",
                std::io::Error::other("disk full"),
            )
        });
        let contents = Contents {
            database: Path::new("/d/kashkeshet.db"),
            config: None,
            access_key: None,
            web_key: None,
            admin_key: None,
            admin_name: None,
        };
        let layout = layout(at("2026-11-16T10:00:00Z"), "1.0.0", &contents);
        assert_eq!(
            backups
                .write(Path::new("/b"), &layout)
                .map_err(|e| e.to_string()),
            Err("could not write /b: disk full".to_owned())
        );
        assert!(backups.names(Path::new("/b")).is_empty());
        assert_eq!(
            backups.prune(Path::new("/b"), 1).map_err(|e| e.to_string()),
            Err("could not write /b: disk full".to_owned())
        );
    }
}
