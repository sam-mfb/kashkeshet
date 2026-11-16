//! Adapts the machine's files and this process's ID to the logic's host, and
//! Claude Code's command to the logic's `ClientCommand`.

use kkt_client_command::Claude;

/// Claude Code's command, for kkt's server: named `kkt` in Claude Code's
/// settings, and `--write` is what a person retries with.
const CLAUDE: Claude = Claude::new("kkt", "--write");
use kkt_client_config::{ClientCommand, Host};
use kkt_files::Files;
use std::io;
use std::path::{Path, PathBuf};

/// The program's real client-config host, also used by its integration tests.
pub struct ClientHost;

impl Host for ClientHost {
    fn read(&self, path: &Path) -> io::Result<String> {
        Files.read(path)
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
        kkt_machine_os::process_id()
    }
}

/// The program's client command: Claude Code's, adapted to
/// kkt-client-config's `ClientCommand`. Also used by its integration tests.
pub struct ClaudeCommand;

impl ClientCommand for ClaudeCommand {
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), String> {
        CLAUDE.remove_server(config_dir)
    }
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), String> {
        CLAUDE.add_server(entry, config_dir)
    }
}
