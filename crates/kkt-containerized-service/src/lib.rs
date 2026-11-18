//! kkt's server installed as a containerized service: `Containerized`
//! implements kkt-container's `Container` over the general `Projects`,
//! whatever container tool implements it. It owns what is kkt's in such an
//! install: the Compose files that come with this program, the settings
//! file's name and the variables it sets, what the managed folder's files
//! say, the service's status and the name pack its server uses, runs of the
//! server's own program in its container, reading its config home, backups
//! and the web key's comparison. It touches the machine only through
//! `Projects`, and takes the tool's name and how its commands are typed
//! from it, never naming a tool itself.
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

/// kkt's containerized service over `projects`, handing the operations it
/// does not do itself to `fallback`.
pub struct Containerized<P, F> {
    projects: P,
    fallback: F,
}

impl<P: Projects, F: Container> Containerized<P, F> {
    /// The service over `projects`, with `fallback` for the rest.
    pub fn new(projects: P, fallback: F) -> Self {
        Containerized { projects, fallback }
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
        backup::backup(&self.projects, &self.project(folder), dest)
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
        in_server::run_in_server(&self.projects, &self.project(folder), command)
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
