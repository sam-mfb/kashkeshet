//! Adapts the machine's client-config operations to the logic's host, and
//! Claude Code's command to the logic's `ClientCommand`.

use kkt_client_command::Claude;
use kkt_client_config::{ClientCommand, Host};
use kkt_client_config_os::OsHost;
use std::io;
use std::path::{Path, PathBuf};

/// The program's real client-config host, also used by its integration tests.
pub struct ClientHost;

impl Host for ClientHost {
    fn read(&self, path: &Path) -> io::Result<String> {
        OsHost.read(path)
    }
    fn exists(&self, path: &Path) -> bool {
        OsHost.exists(path)
    }
    fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
        OsHost.create_private(path, text)
    }
    fn restrict(&self, path: &Path) -> io::Result<()> {
        OsHost.restrict(path)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        OsHost.create_dir_all(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        OsHost.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        OsHost.remove_file(path)
    }
    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        OsHost.absolute(path)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        OsHost.canonicalize(path)
    }
    fn process_id(&self) -> u32 {
        OsHost.process_id()
    }
}

/// The program's client command: Claude Code's, adapted to
/// kkt-client-config's `ClientCommand`. Also used by its integration tests.
pub struct ClaudeCommand;

impl ClientCommand for ClaudeCommand {
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), String> {
        Claude.remove_server(config_dir)
    }
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), String> {
        Claude.add_server(entry, config_dir)
    }
}
