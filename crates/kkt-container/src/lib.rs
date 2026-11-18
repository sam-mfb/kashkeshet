//! A Compose project as a contract, for any program: `Projects`, which
//! asks a container tool about a folder of Compose files and the services
//! they describe and acts on them, with the records it takes and gives
//! (`Project`, `ProjectFile`, `Output`, `User`, `FileState`, `Ran`,
//! `ServiceConfig`) and its error (`ProjectError`, with `FileAction`). A
//! stand-in in memory is behind `testing`.

mod projects;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use projects::{
    FileAction, FileState, Output, Project, ProjectError, ProjectFile, Projects, Ran,
    ServiceConfig, User,
};
use std::path::{Path, PathBuf};

/// A Compose project's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder(pub PathBuf);

impl Folder {
    pub fn path(&self) -> &Path {
        &self.0
    }
}
