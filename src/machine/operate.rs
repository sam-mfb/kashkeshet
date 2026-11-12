//! Adapts this program's native providers and serving settings to operator commands.

use super::connection::ConnectionReads;
use super::settings::{self, DefaultDb, Report, ServeArgs, ServeOptions};
use kkt_admission::{AccessKey, SharedKey};
use kkt_machine::{Env, Platform};
use kkt_operate::{Config, Databases, KeyFiles, OperateParts, Output, ServeFacts, SetupFiles};
use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct NativeOperate<'a> {
    serve: ServeReads<'a>,
    connection: ConnectionReads<'a>,
}

impl<'a> NativeOperate<'a> {
    pub(crate) fn new(env: &'a Env, platform: Platform, serve_args: Option<ServeArgs>) -> Self {
        Self {
            serve: ServeReads {
                env,
                platform,
                serve_args,
            },
            connection: ConnectionReads { env, platform },
        }
    }

    pub(crate) fn parts(&self) -> OperateParts<'_, ConnectionReads<'a>> {
        OperateParts {
            container: &super::container::ContainerHost,
            container_configs: &settings::ContainerConfigs,
            service: &super::service::ServiceSystem,
            remote: &super::remote_access::RemoteHost,
            clients: &super::client_config::ClientHost,
            prober: &super::diagnostics::DiagnosticProber,
            lanes: &crate::commands::lanes::LaneDisk,
            connection: &self.connection,
            clock: Arc::new(kkt_clock_system::SystemClock),
            serve: &self.serve,
            setup_files: &NativeSetupFiles,
            key_files: &NativeKeyFiles,
            database: &NativeDatabases,
            output: &NativeOutput,
        }
    }
}

struct ServeReads<'a> {
    env: &'a Env,
    platform: Platform,
    serve_args: Option<ServeArgs>,
}

impl ServeFacts for ServeReads<'_> {
    fn config(&self, file: Option<PathBuf>) -> Result<Config, String> {
        let (_, options) = settings::read_config(self.env, file)?;
        Ok(Config {
            max_lane_chars: options.lane_limit(),
            db: options.db,
        })
    }
    fn configured_access_key(&self) -> Result<Option<PathBuf>, String> {
        let (_, options) = settings::read_config(self.env, None)?;
        let settings = settings::resolve(
            self.env,
            self.platform,
            options,
            DefaultDb::OrEmpty,
            Report::Bare,
        )?;
        Ok(match settings.access_key {
            AccessKey::None => None,
            AccessKey::File(path) | AccessKey::Default(path) => Some(path),
        })
    }
    fn database(
        &self,
        given: Option<PathBuf>,
        from_config: Option<PathBuf>,
    ) -> Result<PathBuf, String> {
        settings::database(
            self.env,
            self.platform,
            given.or(from_config),
            DefaultDb::Required(settings::NO_HOME_TO_FIND_DB),
        )
    }
    fn bind(&self) -> Result<std::net::SocketAddr, String> {
        settings::serve_bind(self.env, self.platform)
    }
    fn service_matches_defaults(&self, args: &[String]) -> Result<bool, String> {
        let parse = self
            .serve_args
            .ok_or("service arguments were not supplied to this command")?;
        let installed = settings::serve_settings_from_args(self.env, self.platform, parse, args)?;
        let defaults = settings::serve_settings(self.env, self.platform, None, ServeOptions::NONE)?;
        Ok(settings::as_running(installed) == settings::as_running(defaults))
    }
    fn default_access_key(&self) -> Option<PathBuf> {
        settings::default_access_key(self.env)
    }
}

pub(crate) struct NativeSetupFiles;
impl SetupFiles for NativeSetupFiles {
    fn read_record(&self, dir: &Path) -> Result<Option<kkt_setup::Record>, String> {
        kkt_setup_os::read_record(dir)
    }
    fn read_unfinished(&self, dir: &Path) -> Result<Option<kkt_setup::Unfinished>, String> {
        kkt_setup_os::read_unfinished(dir)
    }
    fn write_record(&self, dir: &Path, record: &kkt_setup::Record) -> Result<(), String> {
        kkt_setup_os::write_record(dir, record)
    }
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
        kkt_setup_os::absolute(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        kkt_setup_os::canonicalize(path)
    }
}

pub(crate) struct NativeKeyFiles;
impl KeyFiles for NativeKeyFiles {
    fn read(&self, path: &Path) -> std::io::Result<String> {
        kkt_admission_os::KeyFiles::read(path).map_err(|e| e.error)
    }
    fn read_optional(&self, path: &Path) -> std::io::Result<Option<String>> {
        kkt_admission_os::KeyFiles::read_optional(path).map_err(|e| e.error)
    }
    fn exists(&self, path: &Path) -> bool {
        kkt_admission_os::KeyFiles::exists(path)
    }
    fn is_file(&self, path: &Path) -> bool {
        kkt_admission_os::KeyFiles::is_file(path)
    }
    fn new_key(&self, path: &Path, force: bool) -> Result<String, String> {
        kkt_admission_os::new_key_file(path, force, || {
            SharedKey::generate(&kkt_entropy_os::OsEntropy)
        })
    }
    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String> {
        kkt_admission_os::write_key_file(path, key, force)
    }
    fn write_admin_name(&self, path: &Path, bound: Option<&str>) -> Result<(), String> {
        kkt_admission_os::write_admin_name(path, bound).map_err(|e| e.to_string())
    }
}

struct NativeDatabases;
impl Databases for NativeDatabases {
    fn exists(&self, path: &Path) -> bool {
        kkt_db_sqlite::SqlitePlace::exists(path)
    }
    fn open(&self, path: &Path) -> Result<kkt_rules::Db, String> {
        super::database::open(path)
    }
}

pub(crate) struct NativeOutput;
impl Output for NativeOutput {
    fn output(&self, text: Arguments<'_>) {
        kkt_terminal_os::Terminal.output(text);
    }
    fn errors(&self, text: Arguments<'_>) {
        kkt_terminal_os::Terminal.errors(text);
    }
}
