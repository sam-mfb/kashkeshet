//! The wrapper's private process boundary: tests replace it without running
//! Claude Code.

use std::io;
use std::path::Path;

pub(super) trait Runner {
    /// Runs `claude` with `args`, with `CLAUDE_CONFIG_DIR` set to
    /// `config_dir` or unset for `None`.
    fn run(&self, args: &[&str], config_dir: Option<&Path>) -> io::Result<Ran>;
}

/// How a run ended: whether it succeeded, and its exit status as the
/// system words it.
pub(super) struct Ran {
    pub success: bool,
    pub status: String,
}

#[cfg(feature = "real")]
pub(super) struct OsRunner;

#[cfg(feature = "real")]
impl Runner for OsRunner {
    fn run(&self, args: &[&str], config_dir: Option<&Path>) -> io::Result<Ran> {
        let mut command = std::process::Command::new("claude");
        command.args(args);
        match config_dir {
            Some(dir) => command.env("CLAUDE_CONFIG_DIR", dir),
            None => command.env_remove("CLAUDE_CONFIG_DIR"),
        };
        let output = command.output()?;
        Ok(Ran {
            success: output.status.success(),
            status: output.status.to_string(),
        })
    }
}
