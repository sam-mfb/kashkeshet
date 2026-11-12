//! Configuring an MCP client through that client's own command line, in
//! kkt's terms: removing kkt's server from the client's settings, and adding
//! it as the client's own entry. Some clients keep their settings where only
//! their own command should change them. kkt-client-config decides when to
//! ask and restores the client's file if a call fails;
//! kkt-client-command-claude implements this for Claude Code.

use std::path::Path;

/// A client's own command, as kkt-client-config asks it. `config_dir` is
/// the client's settings folder, when it is not the client's default. An
/// error is for people, and never holds the command's output, which may
/// hold credentials.
pub trait ClientCommand {
    /// Removes kkt's server from the client's settings.
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), String>;
    /// Adds kkt's server, as `entry`, the client's own JSON for it.
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), String>;
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
