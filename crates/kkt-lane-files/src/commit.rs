//! What this crate needs to know about a lane folder's version control: the
//! commit it is at, which an import records. The program supplies it.

use std::io;
use std::path::Path;

/// The commit a lane folder is at.
pub trait LaneCommit {
    /// The commit checked out where `folder` is, with `-dirty` added when
    /// files in `folder` differ from it; `None` when `folder` is not in a
    /// checkout (or one with no commit yet), the tool ignores it, or the tool
    /// is not installed. Any other failure is an error saying what failed.
    fn commit(&self, folder: &Path) -> io::Result<Option<String>>;
}
