//! The commit a lane folder is at, as the machine's git says: `Git`
//! (behind `real`), which `kkt lanes import` asks so the lane versions it
//! saves record where they came from. It declares no contract: the program
//! adapts `Git` to the one kkt-lane-files declares.
//!
//! `Git` runs `git -C <folder>` in the C locale and reads its answers. Only
//! an expected absence is "no commit": the folder is in no git repository,
//! the repository has no commit yet, git ignores the folder, or git is not
//! installed. Any other failure is an error carrying git's own message.
//! What the answers mean is tested over a private runner, without `real`.

use std::io;
use std::path::Path;

mod git;

/// The machine's git, asked where a lane folder is.
pub struct Git;

impl Git {
    /// The commit checked out where `folder` is, with `-dirty` added when
    /// files in `folder` differ from it; `None` when `folder` is not in a
    /// checkout (or one with no commit yet), git ignores it, or git is not
    /// installed. Any other failure is an error carrying git's own message.
    pub fn commit(&self, folder: &Path) -> io::Result<Option<String>> {
        git::commit(&git::OsGit, folder)
    }
}
