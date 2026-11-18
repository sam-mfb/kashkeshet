//! What follows from how kkt is deployed on this machine: as a service, in
//! Docker, or as a client of a server elsewhere. It says where kkt keeps
//! its data and its Docker files, whether `kkt serve` may run here, and
//! what `kkt status` and `kkt upgrade` do.
//!
//! It also says how a client reaches the server ([`connection`]), and what
//! a service installed on this machine runs ([`plan`]) and whether its
//! server could listen on its address ([`check_bind`]).
//!
//! It works on facts it declares itself ([`Deployment`], [`RecordedDeployment`],
//! [`UnrecordedDeployment`], [`connection::RecordedServer`]). Its caller reads how the machine is set up, hands it those
//! facts, and carries out what it returns, so every rule here is tested
//! directly.

mod bind;
pub mod connection;
mod guard;
mod keys;
mod lifecycle;
mod service_plan;
#[cfg(test)]
mod tests;
mod upgrade;

pub use bind::{BindError, check_bind};
pub use guard::{GuardError, guard};
pub use keys::{
    Here, KeyKind, KeyPlace, KeysOff, RecordedServerRead, ServeConfigReads, ServerKeys,
    UnfinishedSetup, UnrecordedReads, admin_key_collision, admin_name_line, admin_name_written,
    configured_key, create_asks_for_docker, create_checks, create_destination_check,
    docker_key_copies, find_here, find_key, find_operator_key, new_key_file, operator_key,
    recorded_key,
};
pub use lifecycle::{
    Replace, RunningVersion, Status, StatusThen, UpgradePlan, running_version, status,
    status_looks_for_unrecorded, upgrade_plan,
};
pub use service_plan::{Plan, PlanError, Request, ServiceSettings, plan};
pub use upgrade::{Upgrade, upgrade};

use kkt_machine::{Env, Platform};
use std::path::PathBuf;

/// How kkt is deployed on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deployment {
    /// The server runs here as a service of the user's login.
    Service {
        /// Shared over the host's Tailscale.
        tailscale: bool,
    },
    /// The server runs here in Docker, from the Compose files in `folder`.
    Docker {
        /// The folder holding the Compose files, the settings file and
        /// `backups`.
        folder: PathBuf,
        /// Shared over a Tailscale container.
        tailscale: bool,
        /// The node's name on the tailnet, once known.
        hostname: Option<String>,
        /// The tailnet's DNS name, once known.
        tailnet: Option<String>,
    },
    /// The server runs elsewhere; only clients are set up here.
    Client,
}

impl Deployment {
    /// The deployment's name in messages: `service`, `docker` or `client`.
    pub fn name(&self) -> &'static str {
        match self {
            Deployment::Service { .. } => "service",
            Deployment::Docker { .. } => "docker",
            Deployment::Client => "client",
        }
    }
}

/// The version of kkt that runs the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerVersion {
    /// This version, as kkt names it, such as `0.5.0`.
    Known(String),
    /// Not known: nothing says which version the server runs. `kkt upgrade`
    /// always replaces such a server.
    NotKnown,
}

/// This machine's deployment, as its setup record says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedDeployment {
    pub deployment: Deployment,
    /// The MCP endpoint of the server set up here, or, on a client machine,
    /// of the server it uses.
    pub url: String,
    /// The file holding that server's access key, if it has one.
    pub key_file: Option<PathBuf>,
    /// On a machine that runs a server: the endpoint of the other server its
    /// clients were switched to, if they were.
    pub clients_url: Option<String>,
    /// The version of kkt that runs the server set up here.
    pub version: ServerVersion,
}

/// A deployment `kkt status` reports beside the record, or in its place: one
/// an earlier setup began and did not finish, or one found installed with no
/// record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnrecordedDeployment {
    /// One sentence saying what was noted or found.
    pub summary: String,
    /// Its deployment and endpoint, or why its deployment is not known.
    pub deployment: Result<DeploymentEndpoint, String>,
}

/// An [`UnrecordedDeployment`]'s kind of deployment and its MCP endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentEndpoint {
    pub deployment: Deployment,
    /// Its MCP endpoint, or `None` while the install that sets it has not
    /// finished.
    pub url: Option<String>,
}

/// The database file's name in the per-user data directory, for `kkt serve`.
const DB_FILE_NAME: &str = "kashkeshet.db";
/// The per-user data directory on macOS, under the home directory.
const MACOS_DATA_DIR: &str = "Library/Application Support/kashkeshet";
/// The per-user data directory elsewhere, under the XDG data home.
const XDG_DATA_DIR: &str = "kashkeshet";
/// The folder in the data directory that holds kkt's Docker files.
const DOCKER_FOLDER: &str = "docker";

/// The per-user data directory: `~/Library/Application Support/kashkeshet`
/// on macOS, and `kashkeshet` in the XDG data home ([`Env::data_home`])
/// elsewhere. The database and the managed Docker folder go here.
pub fn data_dir(env: &Env, platform: Platform) -> Option<PathBuf> {
    match platform {
        Platform::MacOs => Some(env.home.as_ref()?.join(MACOS_DATA_DIR)),
        Platform::Xdg => Some(env.data_home()?.join(XDG_DATA_DIR)),
    }
}

/// The database `kkt serve` uses when none is given, in [`data_dir`].
pub fn default_db(env: &Env, platform: Platform) -> Option<PathBuf> {
    Some(data_dir(env, platform)?.join(DB_FILE_NAME))
}

/// The folder `kkt setup` keeps kkt's Docker files in.
pub fn docker_folder(env: &Env, platform: Platform) -> Result<PathBuf, String> {
    Ok(data_dir(env, platform)
        .ok_or("set HOME to keep kkt's Docker files")?
        .join(DOCKER_FOLDER))
}

/// The MCP endpoint of kkt in Docker, at `mcp_path`: published on the
/// host's `port`, or served over Tailscale as `node`, `(hostname, tailnet)`.
pub fn docker_url(port: u16, node: Option<&(String, String)>, mcp_path: &str) -> String {
    match node {
        Some((hostname, tailnet)) => format!("https://{hostname}.{tailnet}{mcp_path}"),
        None => format!("http://127.0.0.1:{port}{mcp_path}"),
    }
}
