//! The commit a lane folder is at: [`LaneCommit`], which `kkt lanes import`
//! asks so the lane versions it saves record where they came from, and its
//! one implementation, `Git` (behind `real`), which only the program turns
//! on and injects. Consumers depend on this crate without `real`, so they
//! see the contract alone.
//!
//! `Git` runs `git -C <folder>` in the C locale and reads its answers. Only
//! an expected absence is "no commit": the folder is in no git repository,
//! the repository has no commit yet, git ignores the folder, or git is not
//! installed. Any other failure is an error carrying git's own message.
//! What the answers mean is tested over a private runner, without `real`.

use std::io;
use std::path::Path;

#[cfg(any(feature = "real", test))]
mod git;

/// The commit a lane folder is at.
pub trait LaneCommit {
    /// The commit checked out where `folder` is, with `-dirty` added when
    /// files in `folder` differ from it; `None` when `folder` is not in a
    /// checkout (or one with no commit yet), the tool ignores it, or the tool
    /// is not installed. Any other failure is an error saying what failed.
    fn commit(&self, folder: &Path) -> io::Result<Option<String>>;
}

/// The machine's git, asked where a lane folder is.
#[cfg(feature = "real")]
pub struct Git;

#[cfg(feature = "real")]
impl LaneCommit for Git {
    fn commit(&self, folder: &Path) -> io::Result<Option<String>> {
        git::commit(&git::OsGit, folder)
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
