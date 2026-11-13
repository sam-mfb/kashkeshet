//! `ClientCommand` for Claude Code: `claude mcp remove` and
//! `claude mcp add-json`, at user scope, under the name `kkt`, with
//! `CLAUDE_CONFIG_DIR` set to the profile's folder or unset for the default.
//! The command goes through a private runner, so the arguments and the
//! wording are tested without running it; running it is behind `real`.

mod runner;

#[cfg(feature = "real")]
use crate::ClientCommand;
use runner::{Ran, Runner};
use std::io;
use std::path::Path;

/// The name kkt's server has in Claude Code's settings.
const SERVER_NAME: &str = "kkt";

/// Claude Code's own command, which the program injects as the
/// `ClientCommand`.
#[cfg(feature = "real")]
pub struct Claude;

#[cfg(feature = "real")]
impl ClientCommand for Claude {
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), String> {
        remove_server(&runner::OsRunner, config_dir)
    }
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), String> {
        add_server(&runner::OsRunner, entry, config_dir)
    }
}

fn remove_server(runner: &dyn Runner, config_dir: Option<&Path>) -> Result<(), String> {
    run(
        runner,
        &["mcp", "remove", "--scope", "user", SERVER_NAME],
        config_dir,
    )
}

fn add_server(runner: &dyn Runner, entry: &str, config_dir: Option<&Path>) -> Result<(), String> {
    run(
        runner,
        &["mcp", "add-json", "--scope", "user", SERVER_NAME, entry],
        config_dir,
    )
}

/// Runs `claude` with `args`. The error never holds the command's output,
/// which may hold credentials: only its exit status.
fn run(runner: &dyn Runner, args: &[&str], config_dir: Option<&Path>) -> Result<(), String> {
    let ran = runner.run(args, config_dir).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            "claude was not found on PATH; install Claude Code, then retry --write".to_string()
        } else {
            format!("could not run claude: {e}")
        }
    })?;
    match ran {
        Ran { success: true, .. } => Ok(()),
        Ran { status, .. } => Err(format!("claude mcp failed with {status}")),
    }
}

#[cfg(test)]
mod tests;
