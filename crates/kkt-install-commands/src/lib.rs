//! The commands for this machine's kkt install: `kkt setup` and `kkt
//! switch`, `kkt status`, `kkt upgrade`, `kkt uninstall`, `kkt service`,
//! `kkt backup` and `kkt doctor`, and the check `kkt serve` makes before it
//! starts. It reads what is installed here ([`record::InstallReads`]), which
//! the program also hands to the operator's commands. The program supplies
//! every effect and answers questions about its serving settings lazily.

pub mod doctor;
pub mod keys;
pub mod lifecycle;
pub mod record;
pub mod service;
pub mod setup;

use kkt_setup::connection::ConnectionSource;
use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The facts these commands read from the program's serve configuration.
pub struct Config {
    /// The config file read, if there was one.
    pub file: Option<PathBuf>,
    pub db: Option<PathBuf>,
    pub max_lane_chars: usize,
    /// The access key file the config file names, if it names one.
    pub access_key_file: Option<PathBuf>,
    /// Whether the config file turns the access key off, if it says.
    pub no_access_key: Option<bool>,
    /// The name pack named, else the default's name, unchecked.
    pub name_pack: String,
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
    /// The name pack `kkt serve` uses when its settings name none.
    fn default_name_pack(&self) -> String;
    fn service_settings(
        &self,
        config: Option<PathBuf>,
        flags: kkt_bare_service::ServiceSettings,
    ) -> Result<ServiceInputs<'_>, String>;
}

/// Inputs to the service plan, plus a deferred check with opaque program context.
pub struct ServiceInputs<'a> {
    pub request: kkt_bare_service::Request,
    pub check: Box<dyn CheckService + 'a>,
}

/// The program checks the plan against its whole serving surface, with its source context.
pub trait CheckService {
    fn check(self: Box<Self>, plan: &kkt_bare_service::Plan) -> Result<CheckedService, String>;
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
/// use kkt_install_commands::InstallParts;
/// use kkt_setup::connection::ConnectionSource;
/// fn inspect<C: ConnectionSource>(parts: &InstallParts<'_, C>, env: &Env, platform: Platform) {
///     let _ = kkt_install_commands::record::read_unrecorded(parts, env, platform);
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
    /// Removes the setup record in `dir`, for `kkt uninstall`.
    fn remove_record(&self, dir: &Path) -> Result<(), String>;
}

/// Config text read and written by setup when allowing a server host.
pub trait ConfigText {
    fn read(&self, path: &Path) -> Result<Option<String>, String>;
    fn write(&self, path: &Path, text: &str) -> Result<(), String>;
}

/// The local database chosen by the program.
pub trait Databases {
    fn exists(&self, path: &Path) -> bool;
}

/// The lane folder `kkt setup --lanes` imports into the server it sets up.
pub trait ImportLanes {
    /// Reads the lane files in `folder` and checks them as an import into
    /// the server set up for `purpose` would, changing nothing. Returns how
    /// many lanes there are; an error names the file or lane that is wrong.
    fn check(&self, folder: &Path, purpose: kkt_setup::Purpose) -> Result<usize, String>;
    /// Imports the lanes in `folder` into the server `record` says this
    /// machine runs. Returns the report to print.
    fn import(&self, record: &kkt_setup::Record, folder: &Path) -> Result<String, String>;
}

/// Command output, with standard output and standard error kept separate.
pub trait Output {
    fn output(&self, text: Arguments<'_>);
    fn errors(&self, text: Arguments<'_>);
}

/// Implementations chosen by the program. Connection source context remains opaque.
/// Commands that inspect an unrecorded service require `D: ServiceDefaults`;
/// record/config-only commands may use `()` and cannot ask that question.
pub struct InstallParts<'a, C: ConnectionSource, D = ()> {
    pub container: &'a dyn kkt_containerized_service::Container,
    pub container_configs: &'a dyn kkt_containerized_service::ConfigReader,
    pub login_link: &'a dyn kkt_containerized_service::LoginLink,
    pub service: &'a dyn kkt_bare_service::Service,
    pub try_bind: &'a dyn Fn(std::net::SocketAddr) -> std::io::Result<()>,
    pub remote: &'a dyn kkt_remote_access::RemoteAccess,
    pub clients: &'a dyn kkt_llm_agent_config::Host,
    /// A client's own command, for the clients whose settings only it should
    /// change.
    pub client_command: &'a dyn kkt_llm_agent_config::ClientCommand,
    pub prober: &'a dyn kkt_diagnostics::Prober,
    /// Importing a lane folder into the server setup sets up.
    pub import_lanes: &'a dyn ImportLanes,
    pub lines: &'a dyn kkt_setup::Lines,
    pub config_file: &'a dyn ConfigText,
    pub connection: &'a C,
    pub clock: Arc<dyn kkt_clock::Clock>,
    pub serve: &'a dyn ServeFacts,
    pub service_defaults: &'a D,
    pub setup_files: &'a dyn SetupFiles,
    /// Private key files: the server's, and the client key copies setup
    /// writes.
    pub key_files: &'a dyn kkt_keys::KeyFiles,
    /// Where new keys are drawn from.
    pub entropy: &'a dyn kkt_entropy::Entropy,
    pub database: &'a dyn Databases,
    pub backups: &'a dyn kkt_backup::Backups,
    pub output: &'a dyn Output,
}
