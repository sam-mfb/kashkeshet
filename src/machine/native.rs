//! The install's and the operator's commands' parts on this machine: the
//! program's native providers and its serving settings, adapted to each.
//! `main.rs` builds one `NativeParts` and gives each command its parts:
//! the install's commands [`NativeParts::install_parts`], and the
//! operator's commands [`NativeParts::operator_parts`], with what is
//! installed here read through the install's parts. It also picks the
//! server whose lanes `kkt lanes` and `kkt setup --lanes` act on: the local
//! database's or the container's, from what is installed here.

use super::connection::ConnectionReads;
use super::settings::{self, DefaultDb, Report, ServeArgs, ServeOptions};
use kkt_files::Files;
use kkt_install_commands::{Config, InstallParts, ServeFacts, SetupFiles};
use kkt_keys::AccessKey;
use kkt_machine::{Env, Machine, Platform};
use kkt_machine_os::OsMachine;
use kkt_operator_commands::OperatorParts;
use kkt_operator_commands::lanes::{LaneServer, LaneServers, LanesOn};
use std::fmt::Arguments;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The operator's parts on this machine, from the implementations the
/// program chooses. `D` is whether the parts can ask what an installed
/// service runs with (`with_service_args`), or `()` if not.
pub struct NativeParts<'a, D = ()> {
    serve: ServeReads<'a>,
    connection: ConnectionReads<'a>,
    service_defaults: D,
    service: Box<dyn kkt_bare_service::Service>,
    prober: super::diagnostics::Probe,
    login_link: crate::terminal::browser_login::TerminalLink,
    /// kkt's containerized service over Docker and Compose, opening a
    /// sharing setup's login link with the system's own opener.
    container: Containerized,
}

impl<'a> NativeParts<'a> {
    /// The parts for `env` and `platform`, without the service capability.
    pub fn new(env: &'a Env, platform: Platform) -> Self {
        Self {
            serve: ServeReads { env, platform },
            connection: ConnectionReads { env, platform },
            service_defaults: (),
            service: super::service::for_this_os(env),
            prober: super::diagnostics::prober(),
            login_link: crate::terminal::browser_login::TerminalLink::new(),
            container: containerized(),
        }
    }

    /// The same parts, able to ask what an installed service runs with, by
    /// reading its arguments as `kkt serve`'s command line (`serve_args`).
    pub fn with_service_args(self, serve_args: ServeArgs) -> NativeParts<'a, ServiceArgs<'a>> {
        let service_defaults = ServiceArgs {
            env: self.serve.env,
            platform: self.serve.platform,
            serve_args,
        };
        NativeParts {
            serve: self.serve,
            connection: self.connection,
            service_defaults,
            service: self.service,
            prober: self.prober,
            login_link: self.login_link,
            container: self.container,
        }
    }
}

impl<'a, D> NativeParts<'a, D> {
    /// The parts, as the install's commands take them, with `import_lanes`
    /// for the lanes `kkt setup --lanes` imports ([`Self::setup_lanes`]).
    pub fn install_parts<'b>(
        &'b self,
        import_lanes: &'b dyn kkt_install_commands::ImportLanes,
    ) -> InstallParts<'b, ConnectionReads<'a>, D> {
        InstallParts {
            container: &self.container,
            container_configs: &settings::ContainerConfigs,
            login_link: &self.login_link,
            service: self.service.as_ref(),
            try_bind: &kkt_listen::try_bind,
            remote: &REMOTE_ACCESS,
            clients: &super::client_config::ClientHost,
            client_command: &super::client_config::ClaudeCommand,
            prober: &self.prober,
            import_lanes,
            lines: &crate::terminal::TerminalLines,
            config_file: &NativeConfigText,
            connection: &self.connection,
            clock: Arc::new(kkt_clock_system::SystemClock),
            serve: &self.serve,
            service_defaults: &self.service_defaults,
            setup_files: &NativeSetupFiles,
            key_files: &kkt_keys_os::OsKeyFiles,
            entropy: &kkt_entropy_os::OsEntropy,
            database: &NativeDatabases,
            backups: &BACKUPS,
            output: &NativeOutput,
            mcp_path: kkt_mcp::MCP_PATH,
        }
    }

    /// The parts, as the operator's commands take them, with `installed`,
    /// what is installed here, and `lane_servers`, the server `kkt lanes`
    /// acts on ([`Self::lane_servers`]).
    pub fn operator_parts<'b>(
        &'b self,
        installed: &'b dyn kkt_operator_commands::InstalledHere,
        lane_servers: &'b dyn LaneServers,
    ) -> OperatorParts<'b> {
        OperatorParts {
            installed,
            serve: &self.serve,
            paths: &NativeSetupFiles,
            key_files: &kkt_keys_os::OsKeyFiles,
            entropy: &kkt_entropy_os::OsEntropy,
            lane_servers,
            lanes: &crate::commands::lanes::LaneDisk,
            lane_commit: &crate::commands::lanes::LaneGit,
            database: &NativeDatabases,
            clock: Arc::new(kkt_clock_system::SystemClock),
            output: &NativeOutput,
        }
    }

    /// The parts the operator's lane helpers take: setup's lane steps and
    /// the program's tests of them use these alone.
    pub fn lane_parts(&self) -> kkt_operator_commands::LaneParts<'_> {
        kkt_operator_commands::LaneParts {
            serve: &self.serve,
            database: &NativeDatabases,
            clock: Arc::new(kkt_clock_system::SystemClock),
            lanes: &crate::commands::lanes::LaneDisk,
            lane_commit: &crate::commands::lanes::LaneGit,
            output: &NativeOutput,
        }
    }

    /// The lane import `kkt setup --lanes` makes, over the operator's lane
    /// helpers and [`LaneParts`](kkt_operator_commands::LaneParts), on the
    /// server picked as for `kkt lanes`.
    pub fn setup_lanes(&self) -> SetupLanes<'_, 'a, D> {
        SetupLanes(self)
    }

    /// The server `kkt lanes` acts on, picked from `installed`, what is
    /// installed here.
    pub fn lane_servers<'b>(
        &'b self,
        installed: &'b dyn kkt_operator_commands::InstalledHere,
    ) -> HereLanes<'b, 'a, D> {
        HereLanes {
            native: self,
            installed,
        }
    }

    /// The lanes of the server `on` names: kkt in a container, or the local
    /// database `kkt serve` would use unless `db` names another, with the
    /// config file `config`, else the default. Nothing is read until a lane
    /// command asks.
    fn lanes_on(
        &self,
        on: LanesOn,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    ) -> Box<dyn LaneServer + '_> {
        match on {
            LanesOn::Docker(folder) => Box::new(kkt_operator_commands::lanes::ContainerLanes {
                container: Box::new(crate::commands::lanes::LanesInContainer {
                    container: &self.container,
                    configs: &settings::ContainerConfigs,
                    folder: kkt_container::Folder(folder),
                }),
                paths: &NativeSetupFiles,
                output: &NativeOutput,
            }),
            LanesOn::Local => Box::new(kkt_operator_commands::lanes::LocalLanes::new(
                self.lane_parts(),
                db,
                config,
            )),
        }
    }
}

/// kkt's containerized service as the program runs it: over Docker as the
/// general `Projects`.
type Containerized = kkt_containerized_service::Containerized<kkt_container_docker::Docker>;

fn containerized() -> Containerized {
    // The folder kkt was run in, read once: the paths a person types are
    // relative to it.
    let working_dir = OsMachine
        .current_dir()
        .map_err(kkt_containerized_service::NoWorkingDir::new);
    kkt_containerized_service::Containerized::new(
        kkt_container_docker::Docker::new(),
        Box::new(TerminalTell),
        Box::new(super::sharing_node::TailscaleNode),
        Box::new(super::browser::OpenerBrowser::new()),
        Box::new(kkt_keys_os::OsKeyFiles),
        working_dir,
    )
}

/// The containerized service's lines, on the terminal, a line each.
struct TerminalTell;

impl kkt_containerized_service::Tell for TerminalTell {
    fn say(&self, line: &str) {
        kkt_terminal::Terminal.output(format_args!("{line}\n"));
    }
}

/// What is installed on this machine, as kkt-operator-commands declares
/// it: kkt-install-commands' reads of the setup record, the note of an
/// unfinished setup and a server found with no record, read into
/// kkt-deployment's facts. One assembly may not depend on another, so the
/// program joins the two.
pub struct InstalledOnThisMachine<'r, 'a, 'p, C: kkt_deployment::connection::ConnectionSource, D>(
    pub &'r kkt_install_commands::record::InstallReads<'a, 'p, C, D>,
);

impl<C: kkt_deployment::connection::ConnectionSource, D: kkt_install_commands::ServiceDefaults>
    kkt_operator_commands::InstalledHere for InstalledOnThisMachine<'_, '_, '_, C, D>
{
    fn recorded(
        &self,
    ) -> Result<Option<kkt_deployment::ServerKeys>, kkt_operator_commands::PartError> {
        Ok(self
            .0
            .record()
            .map_err(|said| kkt_operator_commands::PartError(said.into()))?
            .as_ref()
            .map(kkt_install_commands::deployment::server_keys))
    }
    fn unfinished(
        &self,
    ) -> Result<Option<kkt_deployment::UnfinishedSetup>, kkt_operator_commands::PartError> {
        Ok(self
            .0
            .unfinished()
            .map_err(|said| kkt_operator_commands::PartError(said.into()))?
            .as_ref()
            .map(kkt_install_commands::deployment::unfinished_setup))
    }
    /// The reason a server cannot be made out, without how it runs, which
    /// the key lookups do not use.
    fn unrecorded(
        &self,
    ) -> Result<Option<kkt_deployment::ServerKeys>, kkt_operator_commands::PartError> {
        match self.0.unrecorded() {
            Ok(found) => Ok(found
                .as_ref()
                .map(kkt_install_commands::deployment::found_server_keys)),
            Err((_, said)) => Err(kkt_operator_commands::PartError(said.into())),
        }
    }
}

/// The server `kkt lanes` acts on: as kkt-operator-commands' `lanes_on`
/// says, from the deployment this machine's setup record names.
pub struct HereLanes<'n, 'a, D> {
    native: &'n NativeParts<'a, D>,
    installed: &'n dyn kkt_operator_commands::InstalledHere,
}

impl<D> LaneServers for HereLanes<'_, '_, D> {
    fn here(
        &self,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    ) -> Result<Box<dyn LaneServer + '_>, kkt_operator_commands::PartError> {
        let deployment = self.installed.recorded()?.map(|keys| keys.deployment);
        let on = kkt_operator_commands::lanes::lanes_on(
            deployment.as_ref(),
            db.is_some() || config.is_some(),
        )
        .map_err(|e| kkt_operator_commands::PartError(Box::new(e)))?;
        Ok(self.native.lanes_on(on, db, config))
    }
}

/// The install's lane import, adapted to the operator's lane helpers.
pub struct SetupLanes<'n, 'a, D>(&'n NativeParts<'a, D>);

/// Lanes go to a server on this machine only.
const LANES_ON_THE_SERVER: &str = "lanes are imported on the server's machine";

impl<D> kkt_install_commands::ImportLanes for SetupLanes<'_, '_, D> {
    /// Against the server `kkt setup` sets up for `purpose`: the local
    /// serve config's limit, or the Docker container's in the folder setup
    /// uses.
    fn check(
        &self,
        folder: &Path,
        purpose: kkt_setup::Purpose,
    ) -> Result<usize, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let native = self.0;
            let on = match purpose {
                kkt_setup::Purpose::Service => LanesOn::Local,
                kkt_setup::Purpose::Docker => LanesOn::Docker(
                    kkt_deployment::docker_folder(native.serve.env, native.serve.platform)
                        .map_err(|e| e.to_string())?,
                ),
                kkt_setup::Purpose::Client => return Err(LANES_ON_THE_SERVER.into()),
            };
            let server = native.lanes_on(on, None, None);
            kkt_operator_commands::lanes::check_lanes_for_setup(
                &native.lane_parts(),
                &*server,
                folder,
            )
            .map_err(|e| e.to_string())
        })())
    }
    /// Into the server `record` says this machine runs; a service runs on
    /// the config file and database `kkt serve` finds by default.
    fn import(
        &self,
        record: &kkt_setup::Record,
        folder: &Path,
    ) -> Result<String, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let native = self.0;
            let on = match &record.mode {
                kkt_setup::Mode::Docker(install) => LanesOn::Docker(install.folder.clone()),
                kkt_setup::Mode::Service { .. } => LanesOn::Local,
                kkt_setup::Mode::Client => return Err(LANES_ON_THE_SERVER.into()),
            };
            let server = native.lanes_on(on, None, None);
            kkt_operator_commands::lanes::import_lanes_for_setup(
                &native.lane_parts(),
                &*server,
                folder,
            )
            .map_err(|e| e.to_string())
        })())
    }
}

/// The program's words for why one of its parts could not answer, passed on
/// to the install commands as their part error.
fn part<T>(said: Result<T, String>) -> Result<T, kkt_install_commands::PartError> {
    said.map_err(|said| kkt_install_commands::PartError(said.into()))
}

pub(crate) struct ServeReads<'a> {
    env: &'a Env,
    platform: Platform,
}

impl<'a> ServeReads<'a> {
    pub(crate) fn new(env: &'a Env, platform: Platform) -> Self {
        Self { env, platform }
    }
}

impl kkt_operator_commands::ServeConfig for ServeReads<'_> {
    fn config(
        &self,
        file: Option<PathBuf>,
    ) -> Result<kkt_operator_commands::Config, kkt_operator_commands::PartError> {
        let config = ServeFacts::config(self, file)
            .map_err(|said| kkt_operator_commands::PartError(said.into()))?;
        Ok(kkt_operator_commands::Config {
            db: config.db,
            max_lane_chars: config.max_lane_chars,
        })
    }
    fn configured_access_key(&self) -> Result<Option<PathBuf>, kkt_operator_commands::PartError> {
        ServeFacts::configured_access_key(self)
            .map_err(|said| kkt_operator_commands::PartError(said.into()))
    }
    fn database(
        &self,
        given: Option<PathBuf>,
        from_config: Option<PathBuf>,
    ) -> Result<PathBuf, kkt_operator_commands::PartError> {
        ServeFacts::database(self, given, from_config)
            .map_err(|said| kkt_operator_commands::PartError(said.into()))
    }
    fn default_access_key(&self) -> Option<PathBuf> {
        ServeFacts::default_access_key(self)
    }
    fn config_dir(&self) -> Option<PathBuf> {
        ServeFacts::config_dir(self)
    }
}

impl ServeFacts for ServeReads<'_> {
    fn config(&self, file: Option<PathBuf>) -> Result<Config, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let (file, options) = settings::read_config(self.env, file)?;
            Ok(Config {
                file,
                max_lane_chars: options.lane_limit(),
                name_pack: options.name_pack_name().to_string(),
                db: options.db,
                access_key_file: options.access_key_file,
                no_access_key: options.no_access_key,
            })
        })())
    }
    fn default_name_pack(&self) -> String {
        kkt_rules::Settings::DEFAULT.name_pack.name().to_string()
    }
    fn config_dir(&self) -> Option<PathBuf> {
        kkt_config_file::config_dir(self.env.config_home().as_deref())
    }
    fn config_file(&self) -> Option<PathBuf> {
        kkt_config_file::config_file(self.env.config_home().as_deref())
    }
    fn config_file_in_home(&self) -> PathBuf {
        kkt_config_file::config_file_in_home()
    }
    fn configured_access_key(&self) -> Result<Option<PathBuf>, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
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
        })())
    }
    fn database(
        &self,
        given: Option<PathBuf>,
        from_config: Option<PathBuf>,
    ) -> Result<PathBuf, kkt_install_commands::PartError> {
        part(settings::database(
            self.env,
            self.platform,
            given.or(from_config),
            DefaultDb::Required(settings::NO_HOME_TO_FIND_DB),
        ))
    }
    fn bind(&self) -> Result<std::net::SocketAddr, kkt_install_commands::PartError> {
        part(settings::serve_bind(self.env, self.platform))
    }
    fn service_settings(
        &self,
        config: Option<PathBuf>,
        flags: kkt_deployment::ServiceSettings,
    ) -> Result<kkt_install_commands::ServiceInputs<'_>, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let flags = ServeOptions {
                db: flags.db,
                access_key_file: flags.access_key_file,
                no_access_key: flags.no_access_key,
                ..ServeOptions::NONE
            };
            let loaded = settings::load_serve_options(self.env, config, flags)?;
            let sources = loaded.sources();
            let request = kkt_deployment::Request {
                config_file: loaded.config_file.clone(),
                flags: service_settings(&loaded.flags),
                from_file: service_settings(&loaded.from_file),
                default_db: kkt_deployment::default_db(self.env, self.platform),
                default_key: settings::default_access_key(self.env),
            };
            Ok(kkt_install_commands::ServiceInputs {
                request,
                check: Box::new(ServiceCheck {
                    env: self.env,
                    platform: self.platform,
                    loaded,
                    sources,
                }),
            })
        })())
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

impl kkt_install_commands::ServiceDefaults for ServiceArgs<'_> {
    fn matches_defaults(&self, args: &[String]) -> Result<bool, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let parse = self.serve_args;
            let installed =
                settings::serve_settings_from_args(self.env, self.platform, parse, args)?;
            let defaults =
                settings::serve_settings(self.env, self.platform, None, ServeOptions::NONE)?;
            Ok(settings::as_running(installed) == settings::as_running(defaults))
        })())
    }
}

fn service_settings(options: &ServeOptions) -> kkt_deployment::ServiceSettings {
    kkt_deployment::ServiceSettings {
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
impl kkt_install_commands::CheckService for ServiceCheck<'_> {
    fn check(
        self: Box<Self>,
        plan: &kkt_deployment::Plan,
    ) -> Result<kkt_install_commands::CheckedService, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
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
            Ok(kkt_install_commands::CheckedService {
                bind: checked.bind,
                config_file: checked.config_file.or_else(|| {
                    kkt_config_file::config_file(self.env.config_home().as_deref())
                        .map(|p| p.display().to_string())
                }),
            })
        })())
    }
}

/// Setup's record and unfinished note on this machine's files, and this
/// program's path and `$PATH`, with setup's wording of what went wrong.
pub(crate) struct NativeSetupFiles;
impl SetupFiles for NativeSetupFiles {
    fn read_record(
        &self,
        dir: &Path,
    ) -> Result<Option<kkt_setup::Record>, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let path = kkt_setup::record_path(dir);
            read_if_present(&path)?
                .map(|text| kkt_setup::Record::parse(&text, &path))
                .transpose()
                .map_err(|e| e.to_string())
        })())
    }
    fn read_unfinished(
        &self,
        dir: &Path,
    ) -> Result<Option<kkt_setup::Unfinished>, kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            let path = kkt_setup::unfinished_path(dir);
            read_if_present(&path)?
                .map(|text| kkt_setup::Unfinished::parse(&text, &path))
                .transpose()
                .map_err(|e| e.to_string())
        })())
    }
    fn write_record(
        &self,
        dir: &Path,
        record: &kkt_setup::Record,
    ) -> Result<(), kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            write_whole(
                &kkt_setup::record_path(dir),
                &record.render().map_err(|e| e.to_string())?,
            )
        })())
    }
    fn write_unfinished(
        &self,
        dir: &Path,
        unfinished: &kkt_setup::Unfinished,
    ) -> Result<(), kkt_install_commands::PartError> {
        part((|| -> Result<_, String> {
            write_whole(
                &kkt_setup::unfinished_path(dir),
                &unfinished.render().map_err(|e| e.to_string())?,
            )
        })())
    }
    fn remove_unfinished(&self, dir: &Path) -> Result<(), kkt_install_commands::PartError> {
        let path = kkt_setup::unfinished_path(dir);
        part(
            Files
                .remove_if_present(&path)
                .map_err(|e| format!("could not remove {}: {e}", path.display())),
        )
    }
    fn kkt_path(&self) -> Result<PathBuf, kkt_install_commands::PartError> {
        part(crate::process::kkt_path())
    }
    fn path_dirs(&self) -> Vec<PathBuf> {
        OsMachine.path_dirs()
    }
    fn on_path(&self, dirs: &[PathBuf], program: &str) -> bool {
        dirs.iter().any(|dir| Files.is_file(&dir.join(program)))
    }
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
        Files.absolute(path)
    }
    fn program_path(&self) -> std::io::Result<PathBuf> {
        crate::process::program_path()
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        Files.canonicalize(path)
    }
    fn remove_record(&self, dir: &Path) -> Result<(), kkt_install_commands::PartError> {
        let path = kkt_setup::record_path(dir);
        part(
            Files
                .remove_file(&path)
                .map_err(|e| format!("could not remove {}: {e}", path.display())),
        )
    }
}

impl kkt_operator_commands::Paths for NativeSetupFiles {
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
        Files.absolute(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        Files.canonicalize(path)
    }
}

/// The text of the file at `path`, if there is one.
fn read_if_present(path: &Path) -> Result<Option<String>, String> {
    Files
        .read_if_present(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))
}

/// Writes `text` at `path` whole, making its folder first; a failure leaves
/// any earlier file as it was.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        Files
            .create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    Files
        .write_by_rename(path, text)
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Backups on the machine's disk, the database copied with SQLite's online
/// backup.
type CopyDatabase = fn(&Path, &Path) -> Result<(), kkt_backup_os::CopyError>;
static BACKUPS: kkt_backup_os::OsBackups<CopyDatabase> =
    kkt_backup_os::OsBackups::new(super::database::copy);

/// Sharing the server over Tailscale; a refusal tells the person to rerun
/// `kkt setup` once they have fixed what it names.
static REMOTE_ACCESS: kkt_remote_access_tailscale::Tailscale =
    kkt_remote_access_tailscale::Tailscale::new("`kkt setup`");

struct NativeDatabases;
impl kkt_install_commands::Databases for NativeDatabases {
    fn exists(&self, path: &Path) -> bool {
        kkt_db_sqlite::SqlitePlace::exists(path)
    }
}
impl kkt_operator_commands::Databases for NativeDatabases {
    fn exists(&self, path: &Path) -> bool {
        kkt_db_sqlite::SqlitePlace::exists(path)
    }
    fn open(&self, path: &Path) -> Result<kkt_rules::Db, kkt_operator_commands::PartError> {
        super::database::open(path).map_err(|said| kkt_operator_commands::PartError(said.into()))
    }
}

pub(crate) struct NativeOutput;
impl kkt_install_commands::Output for NativeOutput {
    fn output(&self, text: Arguments<'_>) {
        kkt_terminal::Terminal.output(text);
    }
    fn errors(&self, text: Arguments<'_>) {
        kkt_terminal::Terminal.errors(text);
    }
}
impl kkt_operator_commands::Output for NativeOutput {
    fn output(&self, text: Arguments<'_>) {
        kkt_terminal::Terminal.output(text);
    }
    fn errors(&self, text: Arguments<'_>) {
        kkt_terminal::Terminal.errors(text);
    }
}

struct NativeConfigText;
impl kkt_install_commands::ConfigText for NativeConfigText {
    fn read(&self, path: &Path) -> Result<Option<String>, kkt_install_commands::PartError> {
        part(
            kkt_config_file::files::read(&super::config_disk::ConfigDisk, path)
                .map_err(|e| e.to_string()),
        )
    }
    fn write(&self, path: &Path, text: &str) -> Result<(), kkt_install_commands::PartError> {
        part(
            kkt_config_file::files::write(&super::config_disk::ConfigDisk, path, text)
                .map_err(|e| e.to_string()),
        )
    }
    fn with_allowed_host(
        &self,
        text: &str,
        host: &str,
    ) -> Result<String, kkt_install_commands::PartError> {
        part(kkt_config_file::with_allowed_host(text, host).map_err(|e| e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{TEST_PLATFORM, test_env};
    use crate::scratch;

    /// The host edit `kkt setup` makes to the config file: each text given,
    /// with what it becomes or why it is refused.
    const HOST_EDITS: [(&str, Result<&str, &str>); 6] = [
        ("", Ok("allow_hosts = [\"h.example\"]\n")),
        (
            "# mine\nbind = \"127.0.0.1:7420\"\n",
            Ok("# mine\nbind = \"127.0.0.1:7420\"\nallow_hosts = [\"h.example\"]\n"),
        ),
        (
            "allow_hosts = [\"h.example\"]\n",
            Ok("allow_hosts = [\"h.example\"]\n"),
        ),
        (
            "allow_hosts = [\"a.example\"] # kept\n",
            Ok("allow_hosts = [\"a.example\", \"h.example\"] # kept\n"),
        ),
        (
            "allow_hosts = [",
            Err("config is not valid TOML: unclosed array, expected `]`"),
        ),
        (
            "allow_hosts = \"x\"\n",
            Err("allow_hosts in the config file must be a list"),
        ),
    ];

    /// The config folder, the config file and the config file's path under a
    /// config home, which the install's and the operator's commands use.
    #[test]
    fn the_config_paths_the_commands_use_are_pinned() {
        use kkt_operator_commands::ServeConfig;
        let env = test_env(Some(Path::new("/h")));
        let serve = ServeReads::new(&env, TEST_PLATFORM);
        assert_eq!(
            ServeFacts::config_dir(&serve),
            Some(PathBuf::from("/h/.config/kkt"))
        );
        assert_eq!(
            ServeConfig::config_dir(&serve),
            Some(PathBuf::from("/h/.config/kkt"))
        );
        assert_eq!(
            serve.config_file(),
            Some(PathBuf::from("/h/.config/kkt/config.toml"))
        );
        assert_eq!(
            serve.config_file_in_home(),
            PathBuf::from("kkt/config.toml")
        );
        let homeless = test_env(None);
        let serve = ServeReads::new(&homeless, TEST_PLATFORM);
        assert_eq!(ServeFacts::config_dir(&serve), None);
        assert_eq!(ServeConfig::config_dir(&serve), None);
        assert_eq!(serve.config_file(), None);
    }

    #[test]
    fn the_host_edit_is_pinned() {
        for (text, edited) in HOST_EDITS {
            assert_eq!(
                kkt_install_commands::ConfigText::with_allowed_host(
                    &NativeConfigText,
                    text,
                    "h.example"
                )
                .map_err(|e| e.to_string()),
                edited.map(str::to_string).map_err(str::to_string),
                "{text:?}"
            );
        }
    }

    /// The setup record, written and read over this program's parts, is in
    /// the config folder; with no home there is none to read, and none can
    /// be written.
    #[test]
    fn the_setup_record_is_in_the_config_folder() {
        let home = scratch::Dir::new("native-record-place");
        let env = test_env(Some(&home));
        let record = kkt_setup::Record {
            mode: kkt_setup::Mode::Client,
            written_by: "0.10.0".into(),
            url: "https://kkt.example/mcp".into(),
            key_file: None,
            clients: None,
        };
        let serve = ServeReads::new(&env, TEST_PLATFORM);
        kkt_install_commands::record::write_setup_record(&serve, &NativeSetupFiles, &record)
            .expect("written");
        assert!(home.join(".config/kkt/setup.toml").is_file());
        assert_eq!(
            kkt_install_commands::record::read_setup(&serve, &NativeSetupFiles)
                .map_err(|e| e.to_string()),
            Ok(Some(record.clone()))
        );
        let homeless = test_env(None);
        let serve = ServeReads::new(&homeless, TEST_PLATFORM);
        assert_eq!(
            kkt_install_commands::record::read_setup(&serve, &NativeSetupFiles)
                .map_err(|e| e.to_string()),
            Ok(None)
        );
        assert_eq!(
            kkt_install_commands::record::write_setup_record(&serve, &NativeSetupFiles, &record)
                .map_err(|e| e.to_string()),
            Err("set HOME to keep the setup record".to_string())
        );
    }
}
