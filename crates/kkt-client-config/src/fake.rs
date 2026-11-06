//! A [`Host`] in memory, for this crate's unit tests.

use super::Host;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

/// The directory relative paths are taken from.
pub const CURRENT_DIR: &str = "/work";

/// A stored file: its text, and whether only its owner may read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub text: String,
    pub private: bool,
}

#[derive(Default)]
pub struct FakeHost {
    pub files: RefCell<BTreeMap<PathBuf, File>>,
    dirs: RefCell<BTreeSet<PathBuf>>,
    /// Each run of `claude`: its arguments and `CLAUDE_CONFIG_DIR`.
    pub claude_runs: RefCell<Vec<(Vec<String>, Option<PathBuf>)>>,
    /// Every run of `claude` fails.
    pub claude_fails: Cell<bool>,
    /// Every run of `claude` first writes `{}` to this file.
    pub claude_writes: RefCell<Option<PathBuf>>,
    /// Every rename fails.
    pub rename_fails: Cell<bool>,
}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

impl FakeHost {
    /// Stores a file that anyone may read, and its directories.
    pub fn put(&self, path: impl Into<PathBuf>, text: &str) {
        let path = path.into();
        if let Some(parent) = path.parent() {
            self.create_dir_all(parent).expect("dirs");
        }
        self.files.borrow_mut().insert(
            path,
            File {
                text: text.into(),
                private: false,
            },
        );
    }

    pub fn file(&self, path: impl AsRef<Path>) -> Option<File> {
        self.files.borrow().get(path.as_ref()).cloned()
    }

    /// Every file's path, in order.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.borrow().keys().cloned().collect()
    }
}

impl Host for FakeHost {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.file(path).map(|f| f.text).ok_or_else(not_found)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.dirs.borrow().contains(path)
    }

    fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
        if self.exists(path) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        self.files.borrow_mut().insert(
            path.to_path_buf(),
            File {
                text: text.into(),
                private: true,
            },
        );
        Ok(())
    }

    fn restrict(&self, path: &Path) -> io::Result<()> {
        let mut files = self.files.borrow_mut();
        files.get_mut(path).ok_or_else(not_found)?.private = true;
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut dirs = self.dirs.borrow_mut();
        for dir in path.ancestors() {
            dirs.insert(dir.to_path_buf());
        }
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.rename_fails.get() {
            return Err(io::Error::other("rename refused"));
        }
        let mut files = self.files.borrow_mut();
        let file = files.remove(from).ok_or_else(not_found)?;
        files.insert(to.to_path_buf(), file);
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.files
            .borrow_mut()
            .remove(path)
            .map(|_| ())
            .ok_or_else(not_found)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(Path::new(CURRENT_DIR).join(path))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        if self.exists(path) {
            Ok(path.to_path_buf())
        } else {
            Err(not_found())
        }
    }

    fn process_id(&self) -> u32 {
        7
    }

    fn claude(&self, args: &[&str], config_dir: Option<&Path>) -> Result<(), String> {
        self.claude_runs.borrow_mut().push((
            args.iter().map(|a| a.to_string()).collect(),
            config_dir.map(Path::to_path_buf),
        ));
        if let Some(path) = self.claude_writes.borrow().clone() {
            self.put(path, "{}");
        }
        if self.claude_fails.get() {
            return Err("claude mcp failed".into());
        }
        Ok(())
    }
}
