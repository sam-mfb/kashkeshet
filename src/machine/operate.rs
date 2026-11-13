//! kkt-operate's parts on this machine: the program's native providers and
//! its serving settings, adapted to the operator's commands. `main.rs` builds
//! one `NativeOperate` and gives every operator command its `parts`.

use super::connection::ConnectionReads;
use super::settings::{self, DefaultDb, Report, ServeArgs, ServeOptions};
use kkt_admission::{AccessKey, SharedKey};
use kkt_machine::{Env, Platform};
use kkt_operate::{
    BackupWriter, Config, Databases, KeyFiles, OperateParts, Output, ServeFacts, SetupFiles,
};
use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The operator's parts on this machine, from the implementations the
/// program chooses. `D` is whether the parts can ask what an installed
/// service runs with (`with_service_args`), or `()` if not.
pub struct NativeOperate<'a, D = ()> {
    serve: ServeReads<'a>,
    connection: ConnectionReads<'a>,
    service_defaults: D,
    service: Box<dyn kkt_service::Service>,
    prober: kkt_probe::HttpProber,
    login_link: crate::terminal::browser_login::TerminalLink,
    /// Docker and Compose, opening a sharing setup's login link with the
    /// system's own opener.
    docker: kkt_container::Docker,
}

impl<'a> NativeOperate<'a> {
    /// The parts for `env` and `platform`, without the service capability.
    pub fn new(env: &'a Env, platform: Platform) -> Self {
        Self {
            serve: ServeReads { env, platform },
            connection: ConnectionReads { env, platform },
            service_defaults: (),
            service: super::service::for_this_os(env),
            prober: super::diagnostics::prober(),
            login_link: crate::terminal::browser_login::TerminalLink::new(),
            docker: kkt_container::Docker::new(Box::new(kkt_browser::SystemBrowser::new())),
        }
    }

    /// The same parts, able to ask what an installed service runs with, by
    /// reading its arguments as `kkt serve`'s command line (`serve_args`).
    pub fn with_service_args(self, serve_args: ServeArgs) -> NativeOperate<'a, ServiceArgs<'a>> {
        let service_defaults = ServiceArgs {
            env: self.serve.env,
            platform: self.serve.platform,
            serve_args,
        };
        NativeOperate {
            serve: self.serve,
            connection: self.connection,
            service_defaults,
            service: self.service,
            prober: self.prober,
            login_link: self.login_link,
            docker: self.docker,
        }
    }
}

impl<'a, D> NativeOperate<'a, D> {
    /// The parts, as kkt-operate's commands take them.
    pub fn parts(&self) -> OperateParts<'_, ConnectionReads<'a>, D> {
        OperateParts {
            container: &self.docker,
            container_configs: &settings::ContainerConfigs,
            login_link: &self.login_link,
            service: self.service.as_ref(),
            try_bind: &kkt_service_os::try_bind,
            remote: &kkt_remote_access::Tailscale,
            clients: &super::client_config::ClientHost,
            client_command: &kkt_client_command_claude::Claude,
            prober: &self.prober,
            lanes: &crate::commands::lanes::LaneDisk,
            lane_commit: &kkt_lane_commit::Git,
            lines: &crate::terminal::TerminalLines,
            config_file: &NativeConfigText,
            connection: &self.connection,
            clock: Arc::new(kkt_clock_system::SystemClock),
            serve: &self.serve,
            service_defaults: &self.service_defaults,
            setup_files: &NativeSetupFiles,
            key_files: &NativeKeyFiles,
            database: &NativeDatabases,
            backups: &NativeBackups,
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
        let (file, options) = settings::read_config(self.env, file)?;
        Ok(Config {
            file,
            max_lane_chars: options.lane_limit(),
            name_pack: options.name_pack_name().to_string(),
            db: options.db,
            access_key_file: options.access_key_file,
            no_access_key: options.no_access_key,
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

/// The service capability: what an installed service runs with, compared
/// with the defaults, as `kkt serve` would read its arguments.
pub struct ServiceArgs<'a> {
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
    fn write_unfinished(
        &self,
        dir: &Path,
        unfinished: &kkt_setup::Unfinished,
    ) -> Result<(), String> {
        kkt_setup_os::write_unfinished(dir, unfinished)
    }
    fn remove_unfinished(&self, dir: &Path) -> Result<(), String> {
        kkt_setup_os::remove_unfinished(dir)
    }
    fn kkt_path(&self) -> Result<PathBuf, String> {
        kkt_setup_os::kkt_path()
    }
    fn path_dirs(&self) -> Vec<PathBuf> {
        kkt_setup_os::path_dirs()
    }
    fn on_path(&self, dirs: &[PathBuf], program: &str) -> bool {
        kkt_setup_os::on_path(dirs, program)
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
    fn remove_record(&self, dir: &Path) -> Result<(), String> {
        kkt_setup_os::remove_record(dir)
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
    fn try_exists(&self, path: &Path) -> std::io::Result<bool> {
        kkt_admission_os::KeyFiles::try_exists(path)
    }
    fn is_file(&self, path: &Path) -> bool {
        kkt_admission_os::KeyFiles::is_file(path)
    }
    fn create_dir_all(&self, dir: &Path) -> Result<(), String> {
        kkt_admission_os::KeyFiles::create_dir_all(dir)
    }
    fn keep_staged(&self, staged: &Path, path: &Path) -> Result<(), String> {
        kkt_admission_os::KeyFiles::keep_staged(staged, path)
    }
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        kkt_admission_os::KeyFiles::remove(path)
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

struct NativeConfigText;
impl kkt_operate::ConfigText for NativeConfigText {
    fn read(&self, path: &Path) -> Result<Option<String>, String> {
        kkt_config_file_os::read(path).map_err(|e| e.to_string())
    }
    fn write(&self, path: &Path, text: &str) -> Result<(), String> {
        kkt_config_file_os::write(path, text).map_err(|e| e.to_string())
    }
}

/// Backups written to the machine's files, the database copied with
/// SQLite's online backup.
pub(crate) struct NativeBackups;
impl BackupWriter for NativeBackups {
    fn write(&self, folder: &Path, layout: &kkt_backup::Layout<'_>) -> Result<PathBuf, String> {
        kkt_backup_os::write(folder, layout, super::database::copy)
    }
}
