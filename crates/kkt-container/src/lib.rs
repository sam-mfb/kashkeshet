//! The operations and records of a kkt container installation. Implementations
//! own their container tool, managed files and output; callers supply requests.

mod config;
mod login;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use config::{ConfigReader, ContainerConfig};
use futures::future::LocalBoxFuture;
use kkt_clock::Clock;
use kkt_names::AdminName;
pub use login::LoginLink;
use std::path::{Path, PathBuf};

/// The host copy of the operator key.
pub const WEB_KEY_FILE_NAME: &str = "docker-web-key";
/// The host copy of the admin key.
pub const ADMIN_KEY_FILE_NAME: &str = "docker-admin-key";
/// The exit code of a key request when no admin key exists.
pub const NO_ADMIN_KEY_EXIT: u8 = 4;

/// The folder managed for a container installation and its backups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder(pub PathBuf);

impl Folder {
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn backups(&self) -> PathBuf {
        self.0.join("backups")
    }
}

/// The version, installation name, port and network access requested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    pub version: String,
    pub project: String,
    pub port: u16,
    pub allow_hosts: Vec<String>,
    pub tailscale: Option<Sharing>,
}

/// The network name and optional login key requested for sharing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sharing {
    pub hostname: String,
    pub auth_key: Option<String>,
}

/// An installation's files, running server, version and network login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub files_present: bool,
    pub running: bool,
    pub version: Option<String>,
    pub name_pack: Result<String, String>,
    pub tailscale: bool,
    pub login: Option<Login>,
}

/// Whether the installation's sharing connection is logged in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    LoggedIn,
    NeedsLogin,
    Unreachable,
    Other(String),
}

/// An installation discovered in the managed folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Existing {
    pub tailscale: bool,
    pub node: Option<(String, String)>,
    pub port: u16,
}

/// Whether the saved web key still matches the running server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyCopy {
    Same,
    Differs,
}

/// Lane files and their original source to import into the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanesImport {
    pub files: Vec<PathBuf>,
    pub source_folder: String,
    pub source_commit: Option<String>,
    pub retire_missing: bool,
}

/// The network name discovered while starting the installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provisioned {
    pub node: Option<(String, String)>,
}

/// Running and managing a kkt container installation.
pub trait Container {
    /// Create or finish the installation, start it, and save its keys.
    fn provision<'a>(
        &'a self,
        clock: &'a dyn Clock,
        login: &'a dyn LoginLink,
        folder: &'a Folder,
        inputs: &'a Inputs,
        key_file: &'a Path,
    ) -> LocalBoxFuture<'a, Result<Provisioned, String>>;
    /// Create an admin key if absent, restart the server and save its keys.
    fn add_admin_key<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        key_file: &'a Path,
        name: &'a AdminName,
    ) -> LocalBoxFuture<'a, Result<PathBuf, String>>;
    /// Inspect the installation, parsing its config with the supplied reader.
    fn status(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default_name_pack: &str,
    ) -> Result<State, String>;
    /// Discover an existing installation; return a refusal if it is incomplete.
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, String>;
    /// Compare the saved web key with the server without exposing either key.
    fn compare_web_key(&self, folder: &Folder, web_key_file: &Path) -> Result<KeyCopy, String>;
    /// Start the installation.
    fn up(&self, folder: &Folder) -> Result<(), String>;
    /// Stop the installation, optionally removing its data. Managed settings remain.
    fn down(&self, folder: &Folder, remove_data: bool) -> Result<(), String>;
    /// Back up the server, optionally copying the backup to a destination.
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, String>;
    /// Start the installation.
    /// Back up and upgrade the server, preserving its settings and saved keys.
    fn upgrade<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        new_version: &'a str,
        key_file: Option<&'a Path>,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    /// Import lane files into the installed server and return its output.
    fn import_lanes(&self, folder: &Folder, import: &LanesImport) -> Result<String, String>;
    /// Export lanes to a folder and return the server's output.
    fn export_lanes(&self, folder: &Folder, dest: &Path) -> Result<String, String>;
    /// Read the installed server's lane limit, falling back to the given default.
    fn lane_limit(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default: usize,
    ) -> Result<usize, String>;
    /// Whether the managed settings exist, including an unfinished installation.
    fn has_settings(&self, folder: &Folder) -> bool;
    /// Whether a managed settings file is present for resuming setup.
    fn settings_file(&self, folder: &Folder) -> bool;
    /// Read the version selected in the installation's settings.
    fn version(&self, folder: &Folder) -> Result<String, String>;
    /// Read the login key the operator supplied, trimmed for the request.
    fn sharing_key(&self, path: &Path) -> Result<String, String>;
}
