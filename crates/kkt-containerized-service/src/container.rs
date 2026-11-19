//! kkt's server installed in a container, as a contract: `ServiceContainer`, the
//! operations its install commands use, with the records they take and
//! give, and what it is given (`ConfigReader`, `LoginLink`).

use crate::{ConfigReader, ContainerError, LoginLink};
use futures::future::LocalBoxFuture;
use kkt_clock::Clock;
use kkt_container::Folder;
use std::path::{Path, PathBuf};

/// The version, installation name, port and network access requested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// The kkt version whose image to run, such as `0.4.0`.
    pub version: String,
    /// The installation name, grouping its containers and volumes.
    /// `kkt` unless testing beside a real setup.
    pub project: String,
    /// The port on the host's `127.0.0.1` that kkt is published on.
    /// The port of kkt's default bind address, 7420, unless testing beside
    /// a real setup.
    pub port: u16,
    /// Host names kkt answers to besides its own, such as a container host's
    /// name.
    pub allow_hosts: Vec<String>,
    /// Share the server over the configured network, if given.
    pub sharing: Option<Sharing>,
}

/// The network name and optional login key requested for sharing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sharing {
    /// The name asked for on the tailnet.
    pub hostname: String,
    /// An auth key, if there is one. Without it the person logs in with a
    /// link, which setup shows.
    pub auth_key: Option<String>,
}

/// An installation's files, running server, version and network login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// Whether the managed folder has its settings file.
    pub files_present: bool,
    /// Whether the kkt container is running.
    pub running: bool,
    /// The kkt version the settings file names.
    pub version: Option<String>,
    /// The selected name pack, or why it could not be read.
    pub name_pack: Result<String, String>,
    /// Whether it is set up to be shared over the network.
    pub sharing: bool,
    /// The network login, for an installation configured for sharing.
    pub login: Option<Login>,
}

/// Whether the installation's sharing connection is logged in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    /// Logged in to the sharing network.
    LoggedIn,
    /// Not logged in: the machine's key expired, or it was logged out. It is
    /// off the tailnet until someone logs it in again.
    NeedsLogin,
    /// The sharing connection does not answer, such as when the installation
    /// is not running.
    Unreachable,
    /// Some other state, as the network provider reports it.
    Other(String),
}

/// An installation discovered in the managed folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Existing {
    /// Shared over the network.
    pub sharing: bool,
    /// The node's name on the tailnet, as `(hostname, tailnet)`, when it is
    /// shared over the network.
    pub node: Option<(String, String)>,
    /// The port on the host's `127.0.0.1` that kkt is published on.
    pub port: u16,
}

/// Whether the saved web key still matches the running server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyCopy {
    /// The saved and running server keys match.
    Same,
    /// The container's key changed after the copy was saved, such as when
    /// it was rotated inside the container.
    Differs,
}

/// Where the host keeps its copies of the installation's keys, each
/// readable only by its owner. The caller names them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCopies {
    /// The access key's copy.
    pub access: PathBuf,
    /// The web key's copy.
    pub web: PathBuf,
    /// The admin key's copy, written only when the server has an admin key.
    pub admin: PathBuf,
}

/// A run of the server's own program inside the installed container, in a
/// new private work folder that is removed afterwards, with files carried
/// in before the run or out after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InServer<'a> {
    /// The program's arguments, after its name.
    pub args: &'a [Arg],
    /// The start of the work folder's path in the container, such as
    /// `/tmp/name`; the folder is made with this prefix, a dot and random
    /// characters.
    pub folder_prefix: &'a str,
    /// The files carried into the work folder, or the folder it is
    /// carried out to.
    pub carry: Carry,
}

/// One argument of a run inside the container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// This text, as it is.
    Text(String),
    /// The work folder's path in the container.
    Folder,
}

/// What a run inside the container carries across.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Carry {
    /// These files on the host, by absolute path, are copied into the work
    /// folder, readable by the program, before it runs.
    In(Vec<PathBuf>),
    /// After the run, the work folder's contents are copied to this
    /// absolute folder on the host, made if missing.
    Out(PathBuf),
}

/// The network name discovered while starting the installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provisioned {
    /// The node's name on the tailnet, as `(hostname, tailnet)`, if kkt is
    /// shared over the network. kkt answers to `<hostname>.<tailnet>`.
    pub node: Option<(String, String)>,
}

/// Running and managing a kkt container installation.
pub trait ServiceContainer {
    /// Create or finish the installation, start it, and save its keys.
    fn provision<'a>(
        &'a self,
        clock: &'a dyn Clock,
        login: &'a dyn LoginLink,
        folder: &'a Folder,
        inputs: &'a Inputs,
        keys: &'a KeyCopies,
    ) -> LocalBoxFuture<'a, Result<Provisioned, ContainerError>>;
    /// Create an admin key named `name` if absent, restart the server and
    /// save its keys. Returns the admin key's copy.
    fn add_admin_key<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        keys: &'a KeyCopies,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<PathBuf, ContainerError>>;
    /// Inspect the installation, parsing its config with the supplied
    /// reader. `config_file` is where the server's default config file is,
    /// relative to its config home.
    fn status(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default_name_pack: &str,
        config_file: &Path,
    ) -> Result<State, ContainerError>;
    /// Discover an existing installation; return a refusal if it is incomplete.
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, ContainerError>;
    /// Compare the saved web key with the server without exposing either key.
    fn compare_web_key(
        &self,
        folder: &Folder,
        web_key_file: &Path,
    ) -> Result<KeyCopy, ContainerError>;
    /// Start the installation.
    fn up(&self, folder: &Folder) -> Result<(), ContainerError>;
    /// Stop the installation, optionally removing its data. Managed settings remain.
    fn down(&self, folder: &Folder, remove_data: bool) -> Result<(), ContainerError>;
    /// Back up the server, optionally copying the backup to a destination.
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, ContainerError>;
    /// Back up and upgrade the server, preserving its settings and saved keys.
    fn upgrade<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        new_version: &'a str,
        keys: Option<&'a KeyCopies>,
    ) -> LocalBoxFuture<'a, Result<(), ContainerError>>;
    /// Run the server's own program in the installed container as `command`
    /// says, and return what it printed.
    fn run_in_server(
        &self,
        folder: &Folder,
        command: &InServer<'_>,
    ) -> Result<String, ContainerError>;
    /// The text of the file at `relative` under the container's config
    /// home, read in a new container on the installation's data, so the
    /// installation need not be running; empty when there is no such file.
    /// `relative` is plain: letters, digits, `.`, `-`, `_` and `/`.
    fn config_home_file(&self, folder: &Folder, relative: &Path) -> Result<String, ContainerError>;
    /// Whether the managed settings path exists, including a directory or an
    /// unfinished installation. Follows symbolic links.
    fn has_settings(&self, folder: &Folder) -> bool;
    /// Whether the managed settings path is a regular file, following symbolic
    /// links, so an unfinished setup can resume from it.
    fn settings_file(&self, folder: &Folder) -> bool;
    /// Read the version selected in the installation's settings.
    fn version(&self, folder: &Folder) -> Result<String, ContainerError>;
    /// Read the login key the operator supplied, trimmed for the request.
    fn sharing_key(&self, path: &Path) -> Result<String, ContainerError>;
}
