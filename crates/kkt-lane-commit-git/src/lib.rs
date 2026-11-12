//! The commit a lane folder is at, from git: [`Git`] implements
//! kkt-lane-commit's `LaneCommit` by running `git -C <folder>` in the C
//! locale and reading its answers. Only an expected absence is "no commit":
//! the folder is in no git repository, the repository has no commit yet,
//! git ignores the folder, or git is not installed. Any other failure is an
//! error carrying git's own message. What the answers mean is tested over a
//! private runner; running git is behind `real`.

#![cfg(any(feature = "real", test))]

mod git;

/// The machine's git, asked where a lane folder is.
#[cfg(feature = "real")]
pub struct Git;

#[cfg(feature = "real")]
impl kkt_lane_commit::LaneCommit for Git {
    fn commit(&self, folder: &std::path::Path) -> std::io::Result<Option<String>> {
        git::commit(&git::OsGit, folder)
    }
}
