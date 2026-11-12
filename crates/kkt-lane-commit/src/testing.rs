//! Commits in memory, for consumers' tests: each folder given one is at it,
//! and any other folder is at none.

use crate::LaneCommit;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// Folders and the commits they are at.
#[derive(Default)]
pub struct Memory {
    commits: BTreeMap<PathBuf, String>,
}

impl Memory {
    /// No folder is at a commit.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same, with `folder` at `commit`.
    pub fn at(mut self, folder: impl Into<PathBuf>, commit: impl Into<String>) -> Self {
        self.commits.insert(folder.into(), commit.into());
        self
    }
}

impl LaneCommit for Memory {
    fn commit(&self, folder: &Path) -> io::Result<Option<String>> {
        Ok(self.commits.get(folder).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_folder_given_a_commit_is_at_one() {
        let commits = Memory::new().at("/lanes", "abc123-dirty");
        assert_eq!(
            commits.commit(Path::new("/lanes")).expect("asked"),
            Some("abc123-dirty".to_string())
        );
        assert_eq!(commits.commit(Path::new("/other")).expect("asked"), None);
    }
}
