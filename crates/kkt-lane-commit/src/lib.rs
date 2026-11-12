//! The commit a lane folder is at, as a contract: [`LaneCommit`], which
//! `kkt lanes import` asks so the lane versions it saves record where they
//! came from. A wrapper named for the version-control tool implements it
//! (kkt-lane-commit-git); the program injects it.

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

#[cfg(any(test, feature = "testing"))]
pub mod testing;
