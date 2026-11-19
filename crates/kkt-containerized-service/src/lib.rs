//! kkt's server installed as a containerized service. `ServiceContainer` is the
//! contract its install commands use, with its records, what it is given
//! (`ConfigReader`, `LoginLink`) and its error (`ContainerError`), and a
//! stand-in in memory behind `testing`. `Containerized` implements it over
//! kkt-container's general `Projects`,
//! whatever container tool implements it. It owns what is kkt's in such an
//! install: the Compose files that come with this program, the settings
//! file's name and the variables it sets, what the managed folder's files
//! say, the service's status and the name pack its server uses, runs of the
//! server's own program in its container, reading its config home,
//! backups, the web key's comparison, and setting it up: its files, the
//! sharing node's login, and the server's key copies on this machine. It
//! touches the machine only through `Projects` and kkt-keys' `KeyFiles`,
//! for the key copies, and takes the tool's name and how its commands are typed
//! from it. The command that asks the sharing node for its status, and
//! what the answer means, come from the `SharingNode` it is given.

mod admin_key;
mod backup;
mod config;
mod container;
mod error;
mod files;
mod in_server;
mod keys;
mod login;
mod reads;
mod services;
mod setup;
mod status;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
mod upgrade;

pub use config::{ConfigReader, ContainerConfig};
pub use container::{
    Arg, Carry, Existing, InServer, Inputs, KeyCopies, KeyCopy, Login, Provisioned,
    ServiceContainer, Sharing, State,
};
pub use error::{ConfigUnread, ContainerError, NoWorkingDir, NodeUnread};
use futures::future::LocalBoxFuture;
use kkt_clock::Clock;
use kkt_container::{Folder, Project, Projects};
use kkt_keys::KeyFiles;
pub use login::LoginLink;
use std::path::{Path, PathBuf};

/// The login link of the operations that never log a sharing node in
/// (the admin key and upgrade): it shows nothing and is never asked to
/// open.
struct NoLogin;

impl LoginLink for NoLogin {
    fn show(&self, _: &str) {}
    fn open_requested(&self) -> bool {
        false
    }
}

/// No folder: paths in it are taken as they are, relative ones from where
/// the person ran kkt.
fn here() -> Folder {
    Folder(PathBuf::new())
}

/// Tells the person running kkt one line.
pub trait Tell {
    fn say(&self, line: &str);
}

/// The person's browser, which a sharing setup's login link opens in.
pub trait Browser {
    /// Starts opening `url`. Fails if it cannot be started; whether it
    /// then worked is told by `open_failure`.
    fn open(&self, url: &str) -> std::io::Result<()>;
    /// Why opening ended without success, if it did since the last time
    /// this was asked. Never repeats the link, which may hold a secret.
    fn open_failure(&self) -> Option<String>;
}

/// Asks and reads what the node that shares the server says of itself,
/// whatever shares it.
pub trait SharingNode {
    /// The command, run in the sharing service, whose answer the two
    /// methods below read.
    fn status_command(&self) -> Vec<String>;
    /// What the node's status answer, `answer`, says of its login, or
    /// `None` when the answer cannot be read.
    fn login(&self, answer: &str) -> Option<NodeLogin>;
    /// What the node's status answer, `answer`, says of the node, or why
    /// it cannot be read.
    fn status(&self, answer: &str) -> Result<NodeStatus, NodeUnread>;
}

/// What a sharing node's status answer says of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeStatus {
    pub login: NodeLogin,
    /// The link to log in with, while one is needed.
    pub link: Option<String>,
    /// The node's full name on its network, such as
    /// `kkt-1.tail1234.ts.net`, once it has one.
    pub name: Option<String>,
    /// Whether HTTPS certificates can be issued for the node.
    pub https: bool,
}

/// Where a sharing node's login stands, as its status answer says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeLogin {
    LoggedIn,
    NeedsLogin,
    /// Another state, in the node's own word for it.
    Other(String),
}

/// kkt's containerized service over `projects`.
pub struct Containerized<P> {
    projects: P,
    tell: Box<dyn Tell>,
    sharing: Box<dyn SharingNode>,
    /// Where a sharing setup's login link opens when the person asks.
    browser: Box<dyn Browser>,
    /// The key copies on this machine.
    key_files: Box<dyn KeyFiles>,
    /// The folder the person ran kkt in, which paths they type are
    /// relative to, or why it could not be found.
    working_dir: Result<PathBuf, NoWorkingDir>,
}

impl<P: Projects> Containerized<P> {
    /// The service over `projects`, telling the person what they need to know through `tell`, reading what the
    /// sharing node says of itself through `sharing`, opening a login link
    /// in `browser`, and writing the server's key copies through
    /// `key_files`. `working_dir` is the folder the person ran kkt in, read
    /// once by the caller, or why it could not be found.
    pub fn new(
        projects: P,
        tell: Box<dyn Tell>,
        sharing: Box<dyn SharingNode>,
        browser: Box<dyn Browser>,
        key_files: Box<dyn KeyFiles>,
        working_dir: Result<PathBuf, NoWorkingDir>,
    ) -> Self {
        Containerized {
            projects,
            tell,
            sharing,
            browser,
            key_files,
            working_dir,
        }
    }

    /// What setup works with, at `clock`, showing a login link through
    /// `link`.
    fn setup_parts<'a>(
        &'a self,
        clock: &'a dyn Clock,
        link: &'a dyn LoginLink,
    ) -> setup::Parts<'a> {
        setup::Parts {
            projects: &self.projects,
            tell: &*self.tell,
            clock,
            link,
            browser: &*self.browser,
            sharing: &*self.sharing,
            key_files: &*self.key_files,
            settings: setup::Settings::DEFAULT,
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

impl<P: Projects> ServiceContainer for Containerized<P> {
    fn provision<'a>(
        &'a self,
        clock: &'a dyn Clock,
        login: &'a dyn LoginLink,
        folder: &'a Folder,
        inputs: &'a Inputs,
        keys: &'a KeyCopies,
    ) -> LocalBoxFuture<'a, Result<Provisioned, ContainerError>> {
        Box::pin(async move {
            let parts = self.setup_parts(clock, login);
            setup::provision(&parts, &self.project(folder), inputs, keys).await
        })
    }
    fn add_admin_key<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        keys: &'a KeyCopies,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<PathBuf, ContainerError>> {
        Box::pin(async move {
            let parts = self.setup_parts(clock, &NoLogin);
            admin_key::add_admin_key(&parts, &self.project(folder), keys, name).await
        })
    }
    fn status(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default_name_pack: &str,
        config_file: &Path,
    ) -> Result<State, ContainerError> {
        status::status(
            &self.projects,
            &self.project(folder),
            &*self.sharing,
            reader,
            default_name_pack,
            config_file,
        )
    }
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, ContainerError> {
        reads::existing(&self.projects, &self.project(folder))
    }
    fn compare_web_key(
        &self,
        folder: &Folder,
        web_key_file: &Path,
    ) -> Result<KeyCopy, ContainerError> {
        keys::compare_web_key(&self.projects, &self.project(folder), web_key_file)
    }
    fn up(&self, folder: &Folder) -> Result<(), ContainerError> {
        services::up(&self.projects, &self.project(folder))
    }
    fn down(&self, folder: &Folder, remove_data: bool) -> Result<(), ContainerError> {
        services::down(&self.projects, &self.project(folder), remove_data)
    }
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, ContainerError> {
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
    ) -> LocalBoxFuture<'a, Result<(), ContainerError>> {
        Box::pin(async move {
            let parts = self.setup_parts(clock, &NoLogin);
            upgrade::upgrade(&parts, &self.project(folder), new_version, keys).await
        })
    }
    fn run_in_server(
        &self,
        folder: &Folder,
        command: &InServer<'_>,
    ) -> Result<String, ContainerError> {
        in_server::run_in_server(&self.projects, &*self.tell, &self.project(folder), command)
    }
    fn config_home_file(&self, folder: &Folder, relative: &Path) -> Result<String, ContainerError> {
        in_server::config_home_file(&self.projects, &self.project(folder), relative)
    }
    fn has_settings(&self, folder: &Folder) -> bool {
        reads::has_settings(&self.projects, &self.project(folder))
    }
    fn settings_file(&self, folder: &Folder) -> bool {
        reads::settings_file(&self.projects, &self.project(folder))
    }
    fn version(&self, folder: &Folder) -> Result<String, ContainerError> {
        reads::version(&self.projects, &self.project(folder))
    }
    fn sharing_key(&self, path: &Path) -> Result<String, ContainerError> {
        reads::sharing_key(&self.projects, path)
    }
}
