//! What client configuration needs from the machine: files and paths. The
//! program adapts the real host; unit tests use a fake.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

pub trait Host {
    /// The file's text.
    fn read(&self, path: &Path) -> io::Result<String>;

    /// Reads `candidate` if and only if it is a regular file (never a symlink)
    /// that is independent of `other` (never a hard link to it). Returns
    /// `Ok(None)` if `candidate` is missing, is a symlink/directory, or shares
    /// an underlying file with `other`.
    fn read_independent_file(&self, candidate: &Path, other: &Path) -> io::Result<Option<String>>;

    /// The entry names in `dir`, in no set order.
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<OsString>>;

    fn exists(&self, path: &Path) -> bool;

    /// Creates `path`, which must not exist yet (`AlreadyExists` otherwise),
    /// readable only by its owner, and writes `text` to it durably.
    fn create_private(&self, path: &Path, text: &str) -> io::Result<()>;

    /// Makes the file readable only by its owner.
    fn restrict(&self, path: &Path) -> io::Result<()>;

    /// The file's permission bits; `None` on a system that has none. An
    /// error when the file cannot be looked at.
    fn mode(&self, path: &Path) -> io::Result<Option<u32>>;

    /// Sets the file's permission bits; does nothing on a system that
    /// has none.
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()>;

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
}
