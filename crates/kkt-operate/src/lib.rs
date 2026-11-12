//! The operator's commands and this machine's install. The program supplies
//! every effect and answers questions about its serving settings lazily.

pub mod agent;
pub mod doctor;
pub mod key;
pub mod keys;
pub mod lanes;
pub mod record;

use kkt_setup::connection::ConnectionSource;
use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The facts these commands read from the program's serve configuration.
pub struct Config {
    pub db: Option<PathBuf>,
    pub max_lane_chars: usize,
}

/// Questions about serving; no file is read until a command asks.
pub trait ServeFacts {
    fn config(&self, file: Option<PathBuf>) -> Result<Config, String>;
    fn configured_access_key(&self) -> Result<Option<PathBuf>, String>;
    fn database(
        &self,
        given: Option<PathBuf>,
        from_config: Option<PathBuf>,
    ) -> Result<PathBuf, String>;
    fn bind(&self) -> Result<std::net::SocketAddr, String>;
    /// Resolve the installed arguments first, then defaults, and compare as a running server.
    fn service_matches_defaults(&self, args: &[String]) -> Result<bool, String>;
    fn default_access_key(&self) -> Option<PathBuf>;
}

/// The setup record and note, and path operations, supplied by the program.
pub trait SetupFiles {
    fn read_record(&self, dir: &Path) -> Result<Option<kkt_setup::Record>, String>;
    fn read_unfinished(&self, dir: &Path) -> Result<Option<kkt_setup::Unfinished>, String>;
    fn write_record(&self, dir: &Path, record: &kkt_setup::Record) -> Result<(), String>;
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
}

/// Key files, including generation of a fresh key by the chosen entropy source.
pub trait KeyFiles {
    fn read(&self, path: &Path) -> std::io::Result<String>;
    fn read_optional(&self, path: &Path) -> std::io::Result<Option<String>>;
    fn exists(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn new_key(&self, path: &Path, force: bool) -> Result<String, String>;
    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String>;
    fn write_admin_name(&self, path: &Path, bound: Option<&str>) -> Result<(), String>;
}

/// The local database chosen by the program, opened only after its path is checked.
pub trait Databases {
    fn exists(&self, path: &Path) -> bool;
    fn open(&self, path: &Path) -> Result<kkt_rules::Db, String>;
}

/// Command output, with standard output and standard error kept separate.
pub trait Output {
    fn output(&self, text: Arguments<'_>);
    fn errors(&self, text: Arguments<'_>);
}

/// Implementations chosen by the program. Connection source context remains opaque.
pub struct OperateParts<'a, C: ConnectionSource> {
    pub container: &'a dyn kkt_container::Host,
    pub container_configs: &'a dyn kkt_container::ConfigReader,
    pub service: &'a dyn kkt_service::System,
    pub remote: &'a dyn kkt_remote_access::Host,
    pub clients: &'a dyn kkt_client_config::Host,
    pub prober: &'a dyn kkt_diagnostics::Prober,
    pub lanes: &'a dyn kkt_lane_files::Disk,
    pub connection: &'a C,
    pub clock: Arc<dyn kkt_clock::Clock>,
    pub serve: &'a dyn ServeFacts,
    pub setup_files: &'a dyn SetupFiles,
    pub key_files: &'a dyn KeyFiles,
    pub database: &'a dyn Databases,
    pub output: &'a dyn Output,
}
