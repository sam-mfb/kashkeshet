//! What this crate needs from a client's own command line: some clients
//! keep their settings where only their own command should change them
//! (Claude Code's `claude mcp`). This crate decides when to ask, and
//! restores the client's file if a call fails; the program supplies the
//! command.

use std::error::Error;
use std::path::Path;

/// How a client's own command failed, in this contract's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandFailure {
    /// The client's command is not installed.
    NotInstalled,
    /// The client's command could not be started.
    NotRun,
    /// The client's command ran and failed.
    Failed,
}

/// Why a client's own command did not do what was asked: the kind, and
/// the implementation's own error, which says it for people.
#[derive(Debug, thiserror::Error)]
#[error("{cause}")]
pub struct CommandError {
    pub kind: CommandFailure,
    #[source]
    pub cause: Box<dyn Error + Send + Sync>,
}

/// A client's own command, as this crate asks it. `config_dir` is the
/// client's settings folder, when it is not the client's default. An error
/// is for people, and never holds the command's output, which may hold
/// credentials.
pub trait ClientCommand {
    /// Removes kkt's server from the client's settings.
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), CommandError>;
    /// Adds kkt's server, as `entry`, the client's own JSON for it.
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), CommandError>;
}
