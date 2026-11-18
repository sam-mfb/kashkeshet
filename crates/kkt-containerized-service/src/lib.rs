//! kkt's server installed as a containerized service: `Containerized`
//! implements kkt-container's `Container` over the general `Projects`,
//! whatever container tool implements it. It owns what is kkt's in such an
//! install: the Compose files that come with this program, the settings
//! file's name and the variables it sets, what the managed folder's files
//! say, the service's status and the name pack its server uses, runs of the
//! server's own program in its container, reading its config home, backups
//! and the web key's comparison. It touches the machine only through
//! `Projects`, and takes the tool's name and how its commands are typed
//! from it, never naming a tool itself. What the sharing node's status
//! answer means comes from the `SharingNode` it is given.
//!
//! The operations it does not do itself it hands to the fallback
//! `Container` it is given.

mod backup;
mod files;
mod in_server;
mod keys;
mod reads;
mod services;
mod status;
#[cfg(test)]
mod tests;

use futures::future::LocalBoxFuture;
use kkt_clock::Clock;
use kkt_container::{
    ConfigReader, Container, Existing, Folder, InServer, Inputs, KeyCopies, KeyCopy, LoginLink,
    Project, Projects, Provisioned, State,
};
use std::path::{Path, PathBuf};

/// No folder: paths in it are taken as they are, relative ones from where
/// the person ran kkt.
fn here() -> Folder {
    Folder(PathBuf::new())
}

/// Tells the person running kkt one line.
pub trait Tell {
    fn say(&self, line: &str);
}

/// Reads what the node that shares the server says of itself, whatever
/// shares it.
pub trait SharingNode {
    /// What the node's status answer, `answer`, says of its login, or
    /// `None` when the answer cannot be read.
    fn login(&self, answer: &str) -> Option<NodeLogin>;
}

/// Where a sharing node's login stands, as its status answer says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeLogin {
    LoggedIn,
    NeedsLogin,
    /// Another state, in the node's own word for it.
    Other(String),
}

/// kkt's containerized service over `projects`, handing the operations it
/// does not do itself to `fallback`.
pub struct Containerized<P, F> {
    projects: P,
    fallback: F,
    tell: Box<dyn Tell>,
    sharing: Box<dyn SharingNode>,
    /// The folder the person ran kkt in, which paths they type are
    /// relative to, or why it could not be found.
    working_dir: Result<PathBuf, String>,
}

impl<P: Projects, F: Container> Containerized<P, F> {
    /// The service over `projects`, with `fallback` for the rest, telling
    /// the person what they need to know through `tell`, and reading what
    /// the sharing node says of itself through `sharing`. `working_dir` is
    /// the folder the person ran kkt in, read once by the caller, or why
    /// it could not be found.
    pub fn new(
        projects: P,
        fallback: F,
        tell: Box<dyn Tell>,
        sharing: Box<dyn SharingNode>,
        working_dir: Result<PathBuf, String>,
    ) -> Self {
        Containerized {
            projects,
            fallback,
            tell,
            sharing,
            working_dir,
        }
    }

    /// The managed folder as a Compose project, whose commands run without
    /// the variables its settings file sets.
    fn project<'a>(&self, folder: &'a Folder) -> Project<'a> {
        Project {
            folder,
            shielded: files::SHIELDED,
        }
    }
}

impl<P: Projects, F: Container> Container for Containerized<P, F> {
    fn provision<'a>(
        &'a self,
        clock: &'a dyn Clock,
        login: &'a dyn LoginLink,
        folder: &'a Folder,
        inputs: &'a Inputs,
        keys: &'a KeyCopies,
    ) -> LocalBoxFuture<'a, Result<Provisioned, String>> {
        self.fallback.provision(clock, login, folder, inputs, keys)
    }
    fn add_admin_key<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        keys: &'a KeyCopies,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<PathBuf, String>> {
        self.fallback.add_admin_key(clock, folder, keys, name)
    }
    fn status(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default_name_pack: &str,
        config_file: &Path,
    ) -> Result<State, String> {
        status::status(
            &self.projects,
            &self.project(folder),
            &*self.sharing,
            reader,
            default_name_pack,
            config_file,
        )
    }
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, String> {
        reads::existing(&self.projects, &self.project(folder))
    }
    fn compare_web_key(&self, folder: &Folder, web_key_file: &Path) -> Result<KeyCopy, String> {
        keys::compare_web_key(&self.projects, &self.project(folder), web_key_file)
    }
    fn up(&self, folder: &Folder) -> Result<(), String> {
        services::up(&self.projects, &self.project(folder))
    }
    fn down(&self, folder: &Folder, remove_data: bool) -> Result<(), String> {
        services::down(&self.projects, &self.project(folder), remove_data)
    }
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, String> {
        backup::backup(
            &self.projects,
            &self.project(folder),
            self.working_dir.as_deref(),
            dest,
        )
    }
    fn upgrade<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        new_version: &'a str,
        keys: Option<&'a KeyCopies>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        self.fallback.upgrade(clock, folder, new_version, keys)
    }
    fn run_in_server(&self, folder: &Folder, command: &InServer<'_>) -> Result<String, String> {
        in_server::run_in_server(&self.projects, &*self.tell, &self.project(folder), command)
    }
    fn config_home_file(&self, folder: &Folder, relative: &Path) -> Result<String, String> {
        in_server::config_home_file(&self.projects, &self.project(folder), relative)
    }
    fn has_settings(&self, folder: &Folder) -> bool {
        reads::has_settings(&self.projects, &self.project(folder))
    }
    fn settings_file(&self, folder: &Folder) -> bool {
        reads::settings_file(&self.projects, &self.project(folder))
    }
    fn version(&self, folder: &Folder) -> Result<String, String> {
        reads::version(&self.projects, &self.project(folder))
    }
    fn sharing_key(&self, path: &Path) -> Result<String, String> {
        reads::sharing_key(&self.projects, path)
    }
}
