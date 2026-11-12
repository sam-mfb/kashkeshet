//! The operator's commands and this machine's install. The program supplies
//! every effect and answers questions about its serving settings lazily.

pub mod agent;
pub mod doctor;
pub mod key;
pub mod keys;
pub mod lanes;
pub mod record;
pub mod service;
pub mod setup;

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
    fn default_access_key(&self) -> Option<PathBuf>;
    fn service_settings(
        &self,
        config: Option<PathBuf>,
        flags: kkt_service::ServiceSettings,
    ) -> Result<ServiceInputs<'_>, String>;
}

/// Inputs to the service plan, plus a deferred check with opaque program context.
pub struct ServiceInputs<'a> {
    pub request: kkt_service::Request,
    pub check: Box<dyn CheckService + 'a>,
}

/// The program checks the plan against its whole serving surface, with its source context.
pub trait CheckService {
    fn check(self: Box<Self>, plan: &kkt_service::Plan) -> Result<CheckedService, String>;
}

/// What the service command needs after the program's settings check.
pub struct CheckedService {
    pub bind: std::net::SocketAddr,
    pub config_file: Option<String>,
}

/// Required only by commands that inspect an unrecorded service. The program
/// supplies its argument parser when constructing this capability.
/// Parts without it cannot inspect an unrecorded service:
///
/// ```compile_fail,E0277
/// use kkt_machine::{Env, Platform};
/// use kkt_operate::OperateParts;
/// use kkt_setup::connection::ConnectionSource;
/// fn inspect<C: ConnectionSource>(parts: &OperateParts<'_, C>, env: &Env, platform: Platform) {
///     let _ = kkt_operate::record::read_unrecorded(parts, env, platform);
/// }
/// ```
pub trait ServiceDefaults {
    /// Resolve installed arguments first, then defaults, and compare as a running server.
    fn matches_defaults(&self, args: &[String]) -> Result<bool, String>;
}

/// The setup record and note, and path operations, supplied by the program.
pub trait SetupFiles {
    fn read_record(&self, dir: &Path) -> Result<Option<kkt_setup::Record>, String>;
    fn read_unfinished(&self, dir: &Path) -> Result<Option<kkt_setup::Unfinished>, String>;
    fn write_record(&self, dir: &Path, record: &kkt_setup::Record) -> Result<(), String>;
    fn write_unfinished(
        &self,
        dir: &Path,
        unfinished: &kkt_setup::Unfinished,
    ) -> Result<(), String>;
    fn remove_unfinished(&self, dir: &Path) -> Result<(), String>;
    fn kkt_path(&self) -> Result<PathBuf, String>;
    fn path_dirs(&self) -> Vec<PathBuf>;
    fn on_path(&self, dirs: &[PathBuf], program: &str) -> bool;
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn program_path(&self) -> std::io::Result<PathBuf>;
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
}

/// Key files, including generation of a fresh key by the chosen entropy source.
pub trait KeyFiles {
    fn read(&self, path: &Path) -> std::io::Result<String>;
    fn read_optional(&self, path: &Path) -> std::io::Result<Option<String>>;
    fn exists(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn create_dir_all(&self, dir: &Path) -> Result<(), String>;
    fn keep_staged(&self, staged: &Path, path: &Path) -> Result<(), String>;
    fn remove(&self, path: &Path) -> std::io::Result<()>;
    fn new_key(&self, path: &Path, force: bool) -> Result<String, String>;
    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String>;
    fn write_admin_name(&self, path: &Path, bound: Option<&str>) -> Result<(), String>;
}

/// Config text read and written by setup when allowing a server host.
pub trait ConfigText {
    fn read(&self, path: &Path) -> Result<Option<String>, String>;
    fn write(&self, path: &Path, text: &str) -> Result<(), String>;
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
/// Commands that inspect an unrecorded service require `D: ServiceDefaults`;
/// record/config-only commands may use `()` and cannot ask that question.
pub struct OperateParts<'a, C: ConnectionSource, D = ()> {
    pub container: &'a dyn kkt_container::Host,
    pub container_configs: &'a dyn kkt_container::ConfigReader,
    pub login_link: &'a dyn kkt_container::LoginLink,
    pub service: &'a dyn kkt_service::System,
    pub try_bind: &'a dyn Fn(std::net::SocketAddr) -> std::io::Result<()>,
    pub remote: &'a dyn kkt_remote_access::Host,
    pub clients: &'a dyn kkt_client_config::Host,
    pub prober: &'a dyn kkt_diagnostics::Prober,
    pub lanes: &'a dyn kkt_lane_files::Disk,
    pub lines: &'a dyn kkt_setup::Lines,
    pub config_file: &'a dyn ConfigText,
    pub connection: &'a C,
    pub clock: Arc<dyn kkt_clock::Clock>,
    pub serve: &'a dyn ServeFacts,
    pub service_defaults: &'a D,
    pub setup_files: &'a dyn SetupFiles,
    pub key_files: &'a dyn KeyFiles,
    pub database: &'a dyn Databases,
    pub output: &'a dyn Output,
}
