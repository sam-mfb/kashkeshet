//! Adapts this program's native providers and serving settings to operator commands.

use super::connection::ConnectionReads;
use super::settings::{self, DefaultDb, Report, ServeArgs, ServeOptions};
use kkt_admission::{AccessKey, SharedKey};
use kkt_machine::{Env, Platform};
use kkt_operate::{Config, Databases, KeyFiles, OperateParts, Output, ServeFacts, SetupFiles};
use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct NativeOperate<'a, D = ()> {
    serve: ServeReads<'a>,
    connection: ConnectionReads<'a>,
    service_defaults: D,
}

impl<'a> NativeOperate<'a> {
    pub(crate) fn new(env: &'a Env, platform: Platform) -> Self {
        Self {
            serve: ServeReads { env, platform },
            connection: ConnectionReads { env, platform },
            service_defaults: (),
        }
    }

    pub(crate) fn with_service_args(
        self,
        serve_args: ServeArgs,
    ) -> NativeOperate<'a, ServiceArgs<'a>> {
        let service_defaults = ServiceArgs {
            env: self.serve.env,
            platform: self.serve.platform,
            serve_args,
        };
        NativeOperate {
            serve: self.serve,
            connection: self.connection,
            service_defaults,
        }
    }
}

impl<'a, D> NativeOperate<'a, D> {
    pub(crate) fn parts(&self) -> OperateParts<'_, ConnectionReads<'a>, D> {
        OperateParts {
            container: &super::container::ContainerHost,
            container_configs: &settings::ContainerConfigs,
            service: &super::service::ServiceSystem,
            try_bind: &kkt_service_os::try_bind,
            remote: &super::remote_access::RemoteHost,
            clients: &super::client_config::ClientHost,
            prober: &super::diagnostics::DiagnosticProber,
            lanes: &crate::commands::lanes::LaneDisk,
            connection: &self.connection,
            clock: Arc::new(kkt_clock_system::SystemClock),
            serve: &self.serve,
            service_defaults: &self.service_defaults,
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
    fn service_settings(
        &self,
        config: Option<PathBuf>,
        flags: kkt_service::ServiceSettings,
    ) -> Result<kkt_operate::ServiceInputs<'_>, String> {
        let flags = ServeOptions {
            db: flags.db,
            access_key_file: flags.access_key_file,
            no_access_key: flags.no_access_key,
            ..ServeOptions::NONE
        };
        let loaded = settings::load_serve_options(self.env, config, flags)?;
        let sources = loaded.sources();
        let request = kkt_service::Request {
            config_file: loaded.config_file.clone(),
            flags: service_settings(&loaded.flags),
            from_file: service_settings(&loaded.from_file),
            default_db: kkt_setup::default_db(self.env, self.platform),
            default_key: settings::default_access_key(self.env),
        };
        Ok(kkt_operate::ServiceInputs {
            request,
            check: Box::new(ServiceCheck {
                env: self.env,
                platform: self.platform,
                loaded,
                sources,
            }),
        })
    }
    fn default_access_key(&self) -> Option<PathBuf> {
        settings::default_access_key(self.env)
    }
}

pub(crate) struct ServiceArgs<'a> {
    env: &'a Env,
    platform: Platform,
    serve_args: ServeArgs,
}

impl kkt_operate::ServiceDefaults for ServiceArgs<'_> {
    fn matches_defaults(&self, args: &[String]) -> Result<bool, String> {
        let parse = self.serve_args;
        let installed = settings::serve_settings_from_args(self.env, self.platform, parse, args)?;
        let defaults = settings::serve_settings(self.env, self.platform, None, ServeOptions::NONE)?;
        Ok(settings::as_running(installed) == settings::as_running(defaults))
    }
}

fn service_settings(options: &ServeOptions) -> kkt_service::ServiceSettings {
    kkt_service::ServiceSettings {
        db: options.db.clone(),
        access_key_file: options.access_key_file.clone(),
        no_access_key: options.no_access_key,
    }
}

struct ServiceCheck<'a> {
    env: &'a Env,
    platform: Platform,
    loaded: settings::Loaded,
    sources: settings::Sources,
}
impl kkt_operate::CheckService for ServiceCheck<'_> {
    fn check(
        self: Box<Self>,
        plan: &kkt_service::Plan,
    ) -> Result<kkt_operate::CheckedService, String> {
        let check = ServeOptions {
            db: plan.check.db.clone(),
            access_key_file: plan.check.access_key_file.clone(),
            no_access_key: plan.check.no_access_key,
            ..self.loaded.flags.over(self.loaded.from_file)
        };
        let mut resolved = settings::resolve(
            self.env,
            self.platform,
            check,
            DefaultDb::Required(settings::NO_HOME_TO_KEEP_DB),
            Report::Traced(&self.sources),
        )?;
        resolved.web_key = plan.check_web_key.clone();
        let checked = settings::check_settings(resolved, &self.sources)?;
        Ok(kkt_operate::CheckedService {
            bind: checked.bind,
            config_file: checked.config_file.or_else(|| {
                kkt_config_file::config_file(self.env.config_home().as_deref())
                    .map(|p| p.display().to_string())
            }),
        })
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
    fn program_path(&self) -> std::io::Result<PathBuf> {
        kkt_setup_os::program_path()
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
