//! Adapts the machine's client-config operations to the logic's host.

use kkt_client_config::Host;
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
    fn claude(&self, args: &[&str], config_dir: Option<&Path>) -> Result<(), String> {
        OsHost.claude(args, config_dir)
    }
}
