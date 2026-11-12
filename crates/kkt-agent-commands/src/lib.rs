//! The agents' commands: `kkt register` and `kkt reclaim`, which get an
//! avatar from a server and save its credentials, `kkt watch`, which waits
//! for new messages from a shell, and `kkt connect`, which writes a client's
//! MCP config. It decides how a client reaches the server, through
//! kkt-setup's connection question, and how what the server refuses is
//! worded.
//!
//! It touches nothing itself. The program passes in every effect and every
//! fact about the local server in [`AgentParts`], each as a contract this
//! crate or a logic crate declares, and calls the commands with typed
//! requests.

mod agent;
mod client;
pub mod connect;
pub mod watch;

pub use agent::{NewAvatar, reclaim, register};

use kkt_agent_client::Servers;
use kkt_clock::Clock;
use kkt_setup::connection::ConnectionSource;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The effects and facts the agents' commands take. The program builds it
/// from the implementations it chooses.
pub struct AgentParts<C: ConnectionSource> {
    /// How a client reaches the server: the setup record, else the serve
    /// settings.
    pub connection: C,
    /// Where the admin key is, for `kkt watch --admin`.
    pub admin_key: Box<dyn AdminKey>,
    /// The admin endpoint's path, which takes the place of `/mcp` in the
    /// server's address for `kkt watch --admin`: the path the server's
    /// transport serves it at.
    pub admin_path: &'static str,
    /// Clients of a server, as kkt-agent-client's contract.
    pub servers: Box<dyn Servers>,
    /// Agent credential files and watch state files.
    pub credentials: Box<dyn Credentials>,
    /// Reading the client's key file.
    pub key_files: Box<dyn KeyText>,
    /// The client configs' files, for `kkt connect`.
    pub clients: Box<dyn kkt_client_config::Host>,
    /// A client's own command, for the clients whose settings only it should
    /// change.
    pub client_command: Box<dyn kkt_client_command::ClientCommand>,
    /// This program's path, for a client's config.
    pub program: Box<dyn Program>,
    /// Where `kkt watch` prints messages and notes.
    pub report: Box<dyn kkt_watch::Report>,
    /// Standard output, for the handle `register` and `reclaim` print and
    /// the config `connect` prints.
    pub output: Box<dyn Output>,
    /// The clock `kkt watch` and the admin endpoint's client keep time by.
    pub clock: Arc<dyn Clock>,
}

/// Where the admin key is.
pub trait AdminKey {
    /// The file `given`, else where `kkt key show --admin` finds it.
    fn path(&self, given: Option<PathBuf>) -> Result<PathBuf, String>;
}

/// Agent credential files and watch state files.
pub trait Credentials {
    /// Reserves `path` for a credential, creating it now so that a call to
    /// the server is not made for nothing; it is removed again if dropped
    /// unsaved.
    fn reserve(&self, path: PathBuf) -> Result<Box<dyn CredentialSlot>, String>;
    /// The token in `file`, else in the environment.
    fn token(&self, file: Option<&Path>) -> Result<String, String>;
    /// The watch state file at `path`, claimed for this watch alone.
    fn claim_state(&self, path: PathBuf) -> Result<Box<dyn kkt_watch::Place>, String>;
}

/// A reserved credential file.
pub trait CredentialSlot {
    fn save(&mut self, secret: &str) -> Result<(), String>;
}

/// Reading a key file.
pub trait KeyText {
    /// The file's text, as it is.
    fn read(&self, path: &Path) -> Result<String, KeyUnread>;
}

/// Why a key file could not be read.
pub struct KeyUnread {
    /// The file is not there.
    pub missing: bool,
    /// The reason, as the system gives it.
    pub reason: String,
}

/// This program.
pub trait Program {
    /// The path of `kkt`, which a client's config runs.
    fn kkt_path(&self) -> Result<PathBuf, String>;
}

/// Standard output.
pub trait Output {
    /// Writes `text` as it is.
    fn output(&self, text: &str);
}

#[cfg(test)]
mod tests;
