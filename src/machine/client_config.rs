//! Adapts the machine's files and this process's ID to the logic's host, and
//! Claude Code's command to the logic's `ClientCommand`.

use kkt_llm_agent_command::Claude;

/// Claude Code's command, for kkt's server: named `kkt` in Claude Code's
/// settings, and `--write` is what a person retries with.
const CLAUDE: Claude = Claude::new("kkt", "--write");
use kkt_files::Files;
use kkt_llm_agent_config::{ClientCommand, CommandError, CommandFailure, Host};
use kkt_machine::Machine;
use kkt_machine_os::OsMachine;
use std::io;
use std::path::{Path, PathBuf};

/// The program's real client-config host, also used by its integration tests.
pub struct ClientHost;

impl Host for ClientHost {
    fn read(&self, path: &Path) -> io::Result<String> {
        Files.read(path)
    }
    fn read_independent_file(&self, candidate: &Path, other: &Path) -> io::Result<Option<String>> {
        Files.read_independent_file(candidate, other)
    }
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<std::ffi::OsString>> {
        Files.dir_entries(dir)
    }
    fn exists(&self, path: &Path) -> bool {
        Files.exists(path)
    }
    fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
        Files.create_private(path, text)
    }
    fn restrict(&self, path: &Path) -> io::Result<()> {
        Files.restrict(path)
    }
    fn mode(&self, path: &Path) -> io::Result<Option<u32>> {
        Files.mode(path)
    }
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        Files.set_mode(path, mode)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        Files.create_dir_all(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        Files.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        Files.remove_file(path)
    }
    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Files.absolute(path)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Files.canonicalize(path)
    }
    fn process_id(&self) -> u32 {
        OsMachine.process_id()
    }
}

/// The program's client command: Claude Code's, adapted to
/// kkt-llm-agent-config's `ClientCommand`. Also used by its integration tests.
pub struct ClaudeCommand;

impl ClientCommand for ClaudeCommand {
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), CommandError> {
        CLAUDE.remove_server(config_dir).map_err(command_error)
    }
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), CommandError> {
        CLAUDE.add_server(entry, config_dir).map_err(command_error)
    }
}

/// Claude Code's failure, as the contract's kind, with Claude's own error
/// as the cause.
fn command_error(error: kkt_llm_agent_command::ClaudeError) -> CommandError {
    use kkt_llm_agent_command::ClaudeError;
    let kind = match &error {
        ClaudeError::NotFound { .. } => CommandFailure::NotInstalled,
        ClaudeError::NotRun(_) => CommandFailure::NotRun,
        ClaudeError::Failed { .. } => CommandFailure::Failed,
    };
    CommandError {
        kind,
        cause: Box::new(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_llm_agent_command::ClaudeError;

    #[test]
    fn each_claude_failure_is_the_contracts_kind_with_claudes_words() {
        for (error, kind, said) in [
            (
                ClaudeError::NotFound { retry: "--write" },
                CommandFailure::NotInstalled,
                "claude was not found on PATH; install Claude Code, then retry --write",
            ),
            (
                ClaudeError::NotRun(io::Error::other("denied")),
                CommandFailure::NotRun,
                "could not run claude: denied",
            ),
            (
                ClaudeError::Failed {
                    status: "exit status: 1".into(),
                },
                CommandFailure::Failed,
                "claude mcp failed with exit status: 1",
            ),
        ] {
            let mapped = command_error(error);
            assert_eq!(mapped.kind, kind);
            assert_eq!(mapped.to_string(), said);
        }
    }
}
