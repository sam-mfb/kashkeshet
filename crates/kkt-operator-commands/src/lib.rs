//! The operator's commands on this machine's server: `kkt key` (create,
//! header, show), `kkt lanes` (export, import) and `kkt agent` (list,
//! revoke), and the lane import and check that `kkt setup` asks for. What
//! is installed here comes as kkt-setup's [`InstalledHere`]; every other
//! effect and serving fact comes through [`OperatorParts`].
//!
//! [`InstalledHere`]: kkt_setup::InstalledHere

pub mod agent;
pub mod key;
pub mod keys;
pub mod lanes;

use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The facts these commands read from the serve config.
pub struct Config {
    pub db: Option<PathBuf>,
    pub max_lane_chars: usize,
}

/// Questions about serving; no file is read until a command asks.
pub trait ServeConfig {
    /// The serve config's facts, from `file` if given, else the default.
    fn config(&self, file: Option<PathBuf>) -> Result<Config, String>;
    /// The access key file the serve config names; `None` when it turns
    /// keys off.
    fn configured_access_key(&self) -> Result<Option<PathBuf>, String>;
    /// The database `kkt serve` would use: `given`, else the config's.
    fn database(
        &self,
        given: Option<PathBuf>,
        from_config: Option<PathBuf>,
    ) -> Result<PathBuf, String>;
    /// Where `kkt key create` writes an access key with no `--path`.
    fn default_access_key(&self) -> Option<PathBuf>;
}

/// Paths made whole, as the operating system resolves them.
pub trait Paths {
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
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

/// Implementations chosen by the program.
pub struct OperatorParts<'a> {
    /// What is installed on this machine.
    pub installed: &'a dyn kkt_setup::InstalledHere,
    pub serve: &'a dyn ServeConfig,
    pub paths: &'a dyn Paths,
    /// Private key files: the server's, and the client key copies setup
    /// writes.
    pub key_files: &'a dyn kkt_keys::KeyFiles,
    /// Where new keys are drawn from.
    pub entropy: &'a dyn kkt_entropy::Entropy,
    pub container: &'a dyn kkt_container::Container,
    pub container_configs: &'a dyn kkt_container::ConfigReader,
    pub lanes: &'a dyn kkt_lane_files::Disk,
    pub lane_commit: &'a dyn kkt_lane_files::LaneCommit,
    pub database: &'a dyn Databases,
    pub clock: Arc<dyn kkt_clock::Clock>,
    pub output: &'a dyn Output,
}

/// What the lane helpers need: the serve config and database, the clock, the
/// container, the lane folder and its commit, and where to say things.
/// Setup's lane steps and `kkt agent`'s database take these alone.
pub struct LaneParts<'a> {
    pub serve: &'a dyn ServeConfig,
    pub database: &'a dyn Databases,
    pub clock: Arc<dyn kkt_clock::Clock>,
    pub container: &'a dyn kkt_container::Container,
    pub container_configs: &'a dyn kkt_container::ConfigReader,
    pub lanes: &'a dyn kkt_lane_files::Disk,
    pub lane_commit: &'a dyn kkt_lane_files::LaneCommit,
    pub output: &'a dyn Output,
}

impl OperatorParts<'_> {
    /// The parts the lane helpers take.
    pub fn lane_parts(&self) -> LaneParts<'_> {
        LaneParts {
            serve: self.serve,
            database: self.database,
            clock: self.clock.clone(),
            container: self.container,
            container_configs: self.container_configs,
            lanes: self.lanes,
            lane_commit: self.lane_commit,
            output: self.output,
        }
    }
}
