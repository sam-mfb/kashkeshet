//! A [`Host`] in memory, for this crate's unit tests, which is also the
//! client's own command: a call can write a file, as Claude Code's does.

use super::Host;
use crate::ClientCommand;
use crate::testing::Call;
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
    /// Each call of the client's command.
    pub claude_runs: RefCell<Vec<Call>>,
    /// Every call of the client's command fails.
    pub claude_fails: Cell<bool>,
    /// Every call of the client's command first writes `{}` to this file.
    pub claude_writes: RefCell<Option<PathBuf>>,
    /// Every rename fails.
    pub rename_fails: Cell<bool>,
    /// Each operation named here fails, as `refused`: `read`,
    /// `create_private`, `restrict`, `create_dir_all`, `remove_file`,
    /// `absolute`, `canonicalize`.
    pub refused: RefCell<BTreeSet<&'static str>>,
}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

impl FakeHost {
    /// `Err(refused)` when `operation` is set to fail.
    fn check(&self, operation: &str) -> io::Result<()> {
        if self.refused.borrow().contains(operation) {
            return Err(io::Error::other("refused"));
        }
        Ok(())
    }

    /// Makes `operation` fail from now on.
    pub fn refuse(&self, operation: &'static str) {
        self.refused.borrow_mut().insert(operation);
    }
}

impl FakeHost {
    /// Stores a file that anyone may read, and its directories.
    pub fn put(&self, path: impl Into<PathBuf>, text: &str) {
        let path = path.into();
        if let Some(parent) = path.parent() {
            let mut dirs = self.dirs.borrow_mut();
            for dir in parent.ancestors() {
                dirs.insert(dir.to_path_buf());
            }
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
        self.check("read")?;
        self.file(path).map(|f| f.text).ok_or_else(not_found)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.dirs.borrow().contains(path)
    }

    fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
        self.check("create_private")?;
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
        self.check("restrict")?;
        let mut files = self.files.borrow_mut();
        files.get_mut(path).ok_or_else(not_found)?.private = true;
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.check("create_dir_all")?;
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
        self.check("remove_file")?;
        self.files
            .borrow_mut()
            .remove(path)
            .map(|_| ())
            .ok_or_else(not_found)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        self.check("absolute")?;
        Ok(Path::new(CURRENT_DIR).join(path))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.check("canonicalize")?;
        if self.exists(path) {
            Ok(path.to_path_buf())
        } else {
            Err(not_found())
        }
    }

    fn process_id(&self) -> u32 {
        7
    }
}

impl FakeHost {
    fn command(&self, call: Call) -> Result<(), crate::CommandError> {
        self.claude_runs.borrow_mut().push(call);
        if let Some(path) = self.claude_writes.borrow().clone() {
            self.put(path, "{}");
        }
        if self.claude_fails.get() {
            return Err(crate::CommandError {
                kind: crate::CommandFailure::Failed,
                cause: "claude mcp failed".into(),
            });
        }
        Ok(())
    }
}

impl ClientCommand for FakeHost {
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), crate::CommandError> {
        self.command(Call::Remove {
            config_dir: config_dir.map(Path::to_path_buf),
        })
    }
    fn add_server(
        &self,
        entry: &str,
        config_dir: Option<&Path>,
    ) -> Result<(), crate::CommandError> {
        self.command(Call::Add {
            entry: entry.to_string(),
            config_dir: config_dir.map(Path::to_path_buf),
        })
    }
}
