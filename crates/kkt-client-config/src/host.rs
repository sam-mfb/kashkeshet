//! What client configuration needs from the machine: files, paths and the
//! `claude` command. The program adapts the real host; unit tests use a fake.

use std::io;
use std::path::{Path, PathBuf};

pub trait Host {
    /// The file's text.
    fn read(&self, path: &Path) -> io::Result<String>;

    fn exists(&self, path: &Path) -> bool;

    /// Creates `path`, which must not exist yet (`AlreadyExists` otherwise),
    /// readable only by its owner, and writes `text` to it durably.
    fn create_private(&self, path: &Path, text: &str) -> io::Result<()>;

    /// Makes the file readable only by its owner.
    fn restrict(&self, path: &Path) -> io::Result<()>;

    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Replaces `to` with `from` in one step.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// `path` made absolute against the current directory.
    fn absolute(&self, path: &Path) -> io::Result<PathBuf>;

    /// `path` with every symbolic link resolved; `NotFound` if it does not
    /// exist.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;

    /// A number that no other process running now has, for temporary names.
    fn process_id(&self) -> u32;

    /// Runs `claude` with `args`, setting `CLAUDE_CONFIG_DIR` to
    /// `config_dir`, or unsetting it for `None`. The error is for people and
    /// never holds the command's output, which may hold credentials.
    fn claude(&self, args: &[&str], config_dir: Option<&Path>) -> Result<(), String>;
}
