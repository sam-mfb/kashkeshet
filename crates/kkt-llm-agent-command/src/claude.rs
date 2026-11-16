//! Claude Code's command: `claude mcp remove` and
//! `claude mcp add-json`, at user scope, under the server name the caller
//! gives, with `CLAUDE_CONFIG_DIR` set to the profile's folder or unset for
//! the default.
//! The command goes through a private runner, so the arguments and the
//! wording are tested without running it.

mod runner;

use runner::{Ran, Runner};
use std::io;
use std::path::Path;

/// Claude Code's own command, for one MCP server. `config_dir` is the
/// profile's settings folder, set as `CLAUDE_CONFIG_DIR`, or `None` to
/// unset it. An error is for people, and never holds the command's output,
/// which may hold credentials.
pub struct Claude {
    /// The server's name in Claude Code's settings.
    server_name: &'static str,
    /// What a person should retry with once Claude Code is installed, as
    /// the refusal words it, such as a flag.
    retry: &'static str,
}

impl Claude {
    /// `server_name` is the server's name in Claude Code's settings;
    /// `retry` is what a person should retry with once Claude Code is
    /// installed, which a missing `claude` names.
    pub const fn new(server_name: &'static str, retry: &'static str) -> Claude {
        Claude { server_name, retry }
    }

    /// Removes the server from Claude Code's settings.
    pub fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), String> {
        remove_server(&runner::OsRunner, self, config_dir)
    }
    /// Adds the server, as `entry`, Claude Code's own JSON for it.
    pub fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), String> {
        add_server(&runner::OsRunner, self, entry, config_dir)
    }
}

fn remove_server(
    runner: &dyn Runner,
    claude: &Claude,
    config_dir: Option<&Path>,
) -> Result<(), String> {
    run(
        runner,
        claude,
        &["mcp", "remove", "--scope", "user", claude.server_name],
        config_dir,
    )
}

fn add_server(
    runner: &dyn Runner,
    claude: &Claude,
    entry: &str,
    config_dir: Option<&Path>,
) -> Result<(), String> {
    run(
        runner,
        claude,
        &[
            "mcp",
            "add-json",
            "--scope",
            "user",
            claude.server_name,
            entry,
        ],
        config_dir,
    )
}

/// Runs `claude` with `args`. The error never holds the command's output,
/// which may hold credentials: only its exit status.
fn run(
    runner: &dyn Runner,
    claude: &Claude,
    args: &[&str],
    config_dir: Option<&Path>,
) -> Result<(), String> {
    let ran = runner.run(args, config_dir).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            format!(
                "claude was not found on PATH; install Claude Code, then retry {}",
                claude.retry
            )
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
