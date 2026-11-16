//! What tests outside kkt-keys need: the shortest key, and
//! [`Memory`], key files in memory.

use crate::{ACCESS_KEY_FILE_MODE, KeyFiles, admin_name_path};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

/// Shortest access key, in bytes: a key this long, and otherwise valid, is
/// one [`check_access_key`](crate::check_access_key) accepts.
pub const MIN_ACCESS_KEY_BYTES: usize = crate::MIN_ACCESS_KEY_BYTES;

/// Key files in memory, for tests: each file's text and mode, the folders
/// made, and a failure on cue. A path is a file only once written; a
/// folder exists once made or once a file is written in it.
#[derive(Default)]
pub struct Memory {
    files: RefCell<BTreeMap<PathBuf, File>>,
    dirs: RefCell<BTreeSet<PathBuf>>,
    failing: RefCell<BTreeSet<PathBuf>>,
}

/// A file [`Memory`] holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub text: String,
    /// The mode it was written with; `None` for a file not written as a key.
    pub mode: Option<u32>,
}

impl Memory {
    pub fn new() -> Memory {
        Memory::default()
    }

    /// Puts a file at `path`, as if it were already there.
    pub fn with_file(self, path: impl Into<PathBuf>, text: &str) -> Memory {
        self.files.borrow_mut().insert(
            path.into(),
            File {
                text: text.to_owned(),
                mode: None,
            },
        );
        self
    }

    /// Makes every step on `path` fail, as a file the system cannot read or
    /// write would.
    pub fn failing(self, path: impl Into<PathBuf>) -> Memory {
        self.failing.borrow_mut().insert(path.into());
        self
    }

    /// The file at `path`, if there is one.
    pub fn file(&self, path: &Path) -> Option<File> {
        self.files.borrow().get(path).cloned()
    }

    /// Whether the folder `dir` was made.
    pub fn has_dir(&self, dir: &Path) -> bool {
        self.dirs.borrow().contains(dir)
    }

    fn check(&self, path: &Path) -> io::Result<()> {
        if self.failing.borrow().contains(path) {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "refused"))
        } else {
            Ok(())
        }
    }

    fn put(&self, path: &Path, text: &str, mode: Option<u32>) {
        self.files.borrow_mut().insert(
            path.to_owned(),
            File {
                text: text.to_owned(),
                mode,
            },
        );
    }
}

impl KeyFiles for Memory {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.check(path)?;
        self.files
            .borrow()
            .get(path)
            .map(|file| file.text.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn exists(&self, path: &Path) -> bool {
        self.try_exists(path).unwrap_or(false)
    }

    fn try_exists(&self, path: &Path) -> io::Result<bool> {
        self.check(path)?;
        Ok(self.files.borrow().contains_key(path) || self.dirs.borrow().contains(path))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn create_dir_all(&self, dir: &Path) -> Result<(), String> {
        self.check(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let mut dirs = self.dirs.borrow_mut();
        for ancestor in dir.ancestors().filter(|d| !d.as_os_str().is_empty()) {
            dirs.insert(ancestor.to_owned());
        }
        Ok(())
    }

    fn keep_staged(&self, staged: &Path, path: &Path) -> Result<(), String> {
        let moved = self.check(staged).and_then(|()| {
            self.files
                .borrow_mut()
                .remove(staged)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
        });
        let file =
            moved.map_err(|e| format!("could not move the key to {}: {e}", path.display()))?;
        self.files.borrow_mut().insert(path.to_owned(), file);
        Ok(())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.check(path)?;
        self.files
            .borrow_mut()
            .remove(path)
            .map(drop)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String> {
        self.check(path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        if !force && self.files.borrow().contains_key(path) {
            return Err(format!(
                "{} already exists; pass --force to replace it",
                path.display()
            ));
        }
        self.put(path, &format!("{key}\n"), Some(ACCESS_KEY_FILE_MODE));
        Ok(())
    }

    fn write_admin_name(&self, admin_key: &Path, bound: Option<&str>) -> io::Result<()> {
        let names = admin_name_path(admin_key);
        self.check(&names)?;
        match bound {
            Some(text) => {
                self.put(&names, text, None);
                Ok(())
            }
            None => match self.remove(&names) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
        }
    }
}
