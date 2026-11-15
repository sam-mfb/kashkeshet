//! Adapts the machine's git to the lane folder's commit contract.

use kkt_lane_commit::Git;
use kkt_lane_files::LaneCommit;
use std::io;
use std::path::Path;

pub(crate) struct LaneGit;

impl LaneCommit for LaneGit {
    fn commit(&self, folder: &Path) -> io::Result<Option<String>> {
        Git.commit(folder)
    }
}
