//! [`OsSteps`] and [`OsSwitchSteps`]: what `kkt setup` and `kkt switch` do
//! to this machine, composed from the program's supplied providers.

use super::InstallOptions;
use crate::doctor::{clients_server_mode, setup_of};
use crate::keys::{admin_key_collision, new_key_file};
use crate::record::{
    DOCKER_KEY_FILE_NAME, docker_folder, docker_url, read_setup, read_unfinished, read_unrecorded,
    write_setup_record,
};
use crate::service::{self, ServiceAction, ServiceInstall};
use kkt_deployment::connection::{self, ConnectionOptions, ConnectionSource};
use kkt_diagnostics::{self as diagnostics, Inspector, point_server_fix_at};
use kkt_llm_agent_config::{self as clients, Client, Setup};
use kkt_machine::{Env, Platform};
use kkt_names::AdminName;
use kkt_setup as setup;
use std::path::{Path, PathBuf};

/// What `kkt setup` does to this machine.
pub(super) struct OsSteps<'a, 'p, C: ConnectionSource, D> {
    pub(super) env: &'a Env,
    pub(super) platform: Platform,
    pub(super) parts: &'a crate::InstallParts<'p, C, D>,
    pub(super) options: InstallOptions,
}

/// Adds `host` to `allow_hosts` in the default config file, making the file
/// if there is none.
fn allow_host_in_config(
    files: &dyn crate::ConfigText,
    serve: &dyn crate::ServeFacts,
    host: &str,
) -> Result<(), StepFailure> {
    let path = serve
        .config_file()
        .ok_or(StepFailure::NoHome(HomeFor::ConfigFile))?;
    let text = files.read(&path)?.unwrap_or_else(String::new);
    let updated =
        files
            .with_allowed_host(&text, host)
            .map_err(|problem| StepFailure::InvalidSetting {
                problem,
                config: path.clone(),
            })?;
    if updated == text {
        return Ok(());
    }
    Ok(files.write(&path, &updated)?)
}

impl<C: ConnectionSource, D: crate::ServiceDefaults> setup::Steps for OsSteps<'_, '_, C, D> {
    fn install_service(
        &self,
        tailscale: bool,
        allow_hosts: &[String],
    ) -> Result<setup::Installed, setup::StepError> {
        (|| -> Result<setup::Installed, StepFailure> {
            // Before anything changes: Tailscale must be logged in, with HTTPS.
            let node = match tailscale {
                true => Some(self.parts.remote.prepare()?),
                false => None,
            };
            if let Some(node) = &node {
                allow_host_in_config(self.parts.config_file, self.parts.serve, &node.dns_name)?;
            }
            for host in allow_hosts {
                allow_host_in_config(self.parts.config_file, self.parts.serve, host)?;
            }
            let had_web_key = self
                .parts
                .serve
                .configured_access_key()
                .ok()
                .flatten()
                .or_else(|| self.parts.serve.default_access_key())
                .map(|access| kkt_keys::web_key_path(&access))
                .is_some_and(|path| self.parts.key_files.try_exists(&path).unwrap_or(false));
            // Setup records the served access key for this service, so
            // the web key's hint goes by it, not by the record as it is.
            tokio::runtime::Handle::current().block_on(service::run_for(
                self.parts,
                ServiceAction::Install(ServiceInstall {
                    config: None,
                    db: None,
                    access_key_file: None,
                    no_access_key: false,
                    dry_run: false,
                }),
                service::KeyLookup::Served,
            ))?;
            let connection = connection::serve_connection(
                self.parts.connection,
                ConnectionOptions::NONE,
                false,
            )?;
            let url = match &node {
                Some(node) => {
                    self.parts.remote.serve(self.parts.serve.bind()?)?;
                    format!("https://{}/mcp", node.dns_name)
                }
                None => connection.url,
            };
            if had_web_key && let Some(access) = &connection.key_file {
                self.parts.output.output(format_args!(
                    "The web key is in {}. Read it with `kkt key show --web`.\n",
                    kkt_keys::web_key_path(access).display()
                ));
            }
            Ok(setup::Installed {
                url,
                key_file: connection.key_file,
            })
        })()
        .map_err(step_error)
    }

    fn docker_data_notice(&self) -> Option<String> {
        let folder = kkt_container::Folder(docker_folder(self.env, self.platform).ok()?);
        let volume = format!("{}_kkt-data", self.options.docker_project);
        let path = folder.path().display();
        match self.parts.container.reuses_data(&folder, &volume) {
            Ok(false) => None,
            Ok(true) => Some(format!(
                "Docker setup will reuse existing storage. The server will use volume {volume} \
                 and managed folder {path} and start with any data already there."
            )),
            Err(_) => Some(format!(
                "Docker could not check for volume {volume}. Setup in {path} will use its data if it exists."
            )),
        }
    }

    fn install_docker(
        &self,
        tailscale: Option<&setup::TailscaleLogin>,
        allow_hosts: &[String],
    ) -> Result<(setup::Installed, setup::DockerSetup), setup::StepError> {
        (|| -> Result<(setup::Installed, setup::DockerSetup), StepFailure> {
            let options = &self.options;
            let folder = docker_folder(self.env, self.platform)?;
            let key_file = self
                .parts
                .serve
                .config_dir()
                .ok_or(StepFailure::NoHome(HomeFor::AccessKey))?
                .join(DOCKER_KEY_FILE_NAME);
            let tailscale_inputs = match tailscale {
                Some(login) => Some(kkt_containerized_service::Sharing {
                    hostname: login.hostname.clone(),
                    auth_key: match &login.auth_key_file {
                        Some(path) => Some(self.parts.container.sharing_key(path)?),
                        None => None,
                    },
                }),
                None => None,
            };
            let inputs = kkt_containerized_service::Inputs {
                version: options.kkt_version.clone(),
                project: options.docker_project.clone(),
                port: options.docker_port,
                allow_hosts: allow_hosts.to_vec(),
                sharing: tailscale_inputs,
            };
            let tailscale = tailscale.is_some();
            let provisioned =
                tokio::runtime::Handle::current().block_on(self.parts.container.provision(
                    self.parts.clock.as_ref(),
                    self.parts.login_link,
                    &kkt_container::Folder(folder.clone()),
                    &inputs,
                    &kkt_deployment::docker_key_copies(&key_file),
                ))?;
            let url = docker_url(
                options.docker_port,
                provisioned.node.as_ref(),
                self.parts.mcp_path,
            );
            let (hostname, tailnet) = provisioned.node.unzip();
            Ok((
                setup::Installed {
                    url,
                    key_file: Some(key_file),
                },
                setup::DockerSetup {
                    folder,
                    tailscale,
                    hostname,
                    tailnet,
                },
            ))
        })()
        .map_err(step_error)
    }

    fn stage_key(&self, key: &str) -> Result<PathBuf, setup::StepError> {
        (|| -> Result<PathBuf, StepFailure> {
            let dir = self
                .parts
                .serve
                .config_dir()
                .ok_or(StepFailure::NoHome(HomeFor::Key))?;
            self.parts.key_files.create_dir_all(&dir)?;
            let path = staged_client_key(&dir);
            self.parts.key_files.write_key(&path, key, true)?;
            Ok(path)
        })()
        .map_err(step_error)
    }

    fn keep_key(&self, staged: &Path, url: &str) -> Result<PathBuf, setup::StepError> {
        (|| -> Result<PathBuf, StepFailure> {
            let path = client_key(staged, url)?;
            self.parts.key_files.keep_staged(staged, &path)?;
            Ok(path)
        })()
        .map_err(step_error)
    }

    fn discard_key(&self, staged: &Path) -> Result<(), setup::StepError> {
        self.parts
            .key_files
            .remove(staged)
            .map_err(|cause| StepFailure::NotRemoved {
                path: staged.to_path_buf(),
                cause,
            })
            .map_err(step_error)
    }

    fn detect_clients(&self) -> Vec<Client> {
        let dirs = self.parts.setup_files.path_dirs();
        let on_path = |program: &str| self.parts.setup_files.on_path(&dirs, program);
        Client::ALL
            .into_iter()
            .filter(|client| client.installed(self.env, &on_path, self.parts.clients))
            .collect()
    }

    fn allow_kkt(&self, client: Client) -> Result<String, setup::StepError> {
        clients::allow_kkt(client, self.env, self.parts.clients).map_err(step_error)
    }

    fn doctor(&self, record: &setup::Record) -> Result<String, setup::StepError> {
        (|| -> Result<String, StepFailure> {
            // The server the clients use: on a machine whose clients were
            // switched, another machine's, checked as a client's.
            let server = record.for_clients();
            let mode = clients_server_mode(record.clients.is_some(), &record.mode);
            let kind = match mode {
                setup::Mode::Client => diagnostics::SetupServer::Elsewhere,
                _ => diagnostics::SetupServer::New,
            };
            let note = waiting_note(&server.url, &mode);
            let mut checks =
                tokio::runtime::Handle::current().block_on(diagnostics::until_reachable(
                    Inspector::new(self.parts.prober),
                    self.parts.clock.as_ref(),
                    &server.url,
                    server.key_file.as_deref(),
                    diagnostics::setup_wait(diagnostics::Settings::DEFAULT, kind),
                    &mut || self.parts.output.output(format_args!("{note}\n")),
                ))?;
            if let Some(fix) = diagnostics::server_fix(&setup_of(&mode)) {
                point_server_fix_at(&mut checks, &fix);
            }
            match diagnostics::report(checks) {
                (report, true) => Ok(report.trim_end().to_string()),
                (report, false) => Err(StepFailure::Checks {
                    report: report.trim_end().to_string(),
                }),
            }
        })()
        .map_err(step_error)
    }

    fn unfinished(&self) -> Result<Option<setup::Unfinished>, setup::StepError> {
        read_unfinished(
            self.parts.serve,
            self.parts.setup_files,
            self.parts.container,
            self.parts.service,
        )
        .map_err(step_error)
    }

    fn save_unfinished(&self, unfinished: &setup::Unfinished) -> Result<(), setup::StepError> {
        (|| -> Result<(), StepFailure> {
            let config_dir = self
                .parts
                .serve
                .config_dir()
                .ok_or(StepFailure::NoHome(HomeFor::Note))?;
            Ok(self
                .parts
                .setup_files
                .write_unfinished(&config_dir, unfinished)?)
        })()
        .map_err(step_error)
    }

    fn forget_unfinished(&self) -> Result<(), setup::StepError> {
        (|| -> Result<(), StepFailure> {
            let Some(config_dir) = self.parts.serve.config_dir() else {
                return Ok(());
            };
            Ok(self.parts.setup_files.remove_unfinished(&config_dir)?)
        })()
        .map_err(step_error)
    }

    fn add_admin_key(
        &self,
        record: &setup::Record,
        name: &AdminName,
    ) -> Result<PathBuf, setup::StepError> {
        (|| -> Result<PathBuf, StepFailure> {
        let access = record
            .key_file
            .as_deref()
            .ok_or(StepFailure::AdminKeyNeedsAccessKey)?;
        match &record.mode {
            setup::Mode::Docker(install) => tokio::runtime::Handle::current()
                .block_on(self.parts.container.add_admin_key(
                    self.parts.clock.as_ref(),
                    &kkt_container::Folder(install.folder.clone()),
                    &kkt_deployment::docker_key_copies(access),
                    name.as_str(),
                ))
                .map_err(StepFailure::from),
            setup::Mode::Service { .. } => {
                let admin = kkt_keys::admin_key_path(access)
                    .ok_or_else(admin_key_collision)?;
                // A key file that cannot be inspected is not replaced:
                // `new_key_file` refuses to write over one. Only a new key
                // is given the name.
                let service = self.parts.service;
                let made = service
                    .ready()
                    .and_then(|()| {
                        service.add_key(&admin, &|path| {
                            let key =
                                new_key_file(self.parts.key_files, self.parts.entropy, path, false)
                                    .map_err(|e| kkt_bare_service::KeyError(e.into()))?;
                            kkt_deployment::admin_name_written(
                                path,
                                Some(&name.to_string()),
                                self.parts.key_files.write_admin_name(
                                    path,
                                    Some(kkt_keys::bound_to(name, &key).as_str()),
                                ),
                            )
                            .map(drop)
                            .map_err(|e| kkt_bare_service::KeyError(e.into()))
                        })
                    })
                    ?;
                self.parts.output.output(format_args!(
                    "{} admin key for /mcp-admin is in {}, and the server was restarted to use it. Read it with `kkt key show --admin`.\n",
                    if made { "A new" } else { "The" },
                    admin.display()
                ));
                Ok(admin)
            }
            setup::Mode::Client => Err(StepFailure::AdminKeyOnClient),
        }
    })()
        .map_err(step_error)
    }

    fn check_lanes(
        &self,
        folder: &Path,
        purpose: setup::Purpose,
    ) -> Result<usize, setup::StepError> {
        self.parts
            .import_lanes
            .check(folder, purpose)
            .map_err(step_error)
    }

    fn import_lanes(
        &self,
        record: &setup::Record,
        folder: &Path,
    ) -> Result<String, setup::StepError> {
        self.parts
            .import_lanes
            .import(record, folder)
            .map_err(step_error)
    }

    fn check_typed_key(&self, key: &str) -> Result<(), setup::StepError> {
        check_typed_key(key).map_err(step_error)
    }

    fn https_absent(&self, url: &str) -> Result<bool, setup::StepError> {
        https_absent(self.parts.prober, url).map_err(step_error)
    }
}

/// Checks that `key`, typed in for a client, is a valid access key, by
/// kkt-keys' rule. Returns why not.
fn check_typed_key(key: &str) -> Result<(), StepFailure> {
    Ok(kkt_keys::check_access_key(key)?)
}

/// Whether there is no https at `url`: nothing listens there, or what
/// answers does not speak TLS. Asks with no key.
fn https_absent(prober: &dyn kkt_diagnostics::Prober, url: &str) -> Result<bool, StepFailure> {
    let tls = tokio::runtime::Handle::current().block_on(prober.tls(url))?;
    Ok(matches!(
        tls,
        kkt_diagnostics::Tls::NoListener | kkt_diagnostics::Tls::NotTls
    ))
}

impl<C: ConnectionSource, D> setup::SwitchSteps for OsSteps<'_, '_, C, D> {
    fn connected_clients(&self) -> Result<Vec<Client>, setup::StepError> {
        connected_clients(self.env, self.parts.clients).map_err(step_error)
    }

    fn client_url(&self, client: Client) -> Result<Option<String>, setup::StepError> {
        client_url(self.env, self.parts.clients, client).map_err(step_error)
    }

    fn recorded(&self) -> Result<Option<setup::Record>, setup::StepError> {
        read_setup(self.parts.serve, self.parts.setup_files).map_err(step_error)
    }

    fn write_record(&self, record: &setup::Record) -> Result<(), setup::StepError> {
        write_setup_record(self.parts.serve, self.parts.setup_files, record).map_err(step_error)
    }

    fn connect(&self, client: Client, record: &setup::Record) -> Result<String, setup::StepError> {
        connect(
            self.env,
            self.parts.clients,
            self.parts.client_command,
            self.parts.setup_files,
            client,
            record,
        )
        .map_err(step_error)
    }

    fn validate_address(&self, url: &str) -> Result<(), setup::StepError> {
        kkt_deployment::connection::server_address(url, None)
            .map(|_| ())
            .map_err(step_error)
    }

    fn check_address(&self, url: &str) -> Result<(), setup::StepError> {
        check_address(self.parts.prober, url).map_err(step_error)
    }

    fn check_key(&self, url: &str, key: &setup::ClientKey) -> Result<(), setup::StepError> {
        check_key(self.parts.prober, url, key).map_err(step_error)
    }
}

/// What `kkt switch` does to this machine. It installs nothing, so it holds
/// only what switching uses; it shares each step with [`OsSteps`].
pub(super) struct OsSwitchSteps<'a, 'p, C: ConnectionSource, D> {
    pub(super) env: &'a Env,
    pub(super) parts: &'a crate::InstallParts<'p, C, D>,
}

impl<C: ConnectionSource, D> setup::SwitchSteps for OsSwitchSteps<'_, '_, C, D> {
    fn connected_clients(&self) -> Result<Vec<Client>, setup::StepError> {
        connected_clients(self.env, self.parts.clients).map_err(step_error)
    }

    fn client_url(&self, client: Client) -> Result<Option<String>, setup::StepError> {
        client_url(self.env, self.parts.clients, client).map_err(step_error)
    }

    fn recorded(&self) -> Result<Option<setup::Record>, setup::StepError> {
        read_setup(self.parts.serve, self.parts.setup_files).map_err(step_error)
    }

    fn write_record(&self, record: &setup::Record) -> Result<(), setup::StepError> {
        write_setup_record(self.parts.serve, self.parts.setup_files, record).map_err(step_error)
    }

    fn connect(&self, client: Client, record: &setup::Record) -> Result<String, setup::StepError> {
        connect(
            self.env,
            self.parts.clients,
            self.parts.client_command,
            self.parts.setup_files,
            client,
            record,
        )
        .map_err(step_error)
    }

    fn validate_address(&self, url: &str) -> Result<(), setup::StepError> {
        kkt_deployment::connection::server_address(url, None)
            .map(|_| ())
            .map_err(step_error)
    }

    fn check_address(&self, url: &str) -> Result<(), setup::StepError> {
        check_address(self.parts.prober, url).map_err(step_error)
    }

    fn check_key(&self, url: &str, key: &setup::ClientKey) -> Result<(), setup::StepError> {
        check_key(self.parts.prober, url, key).map_err(step_error)
    }
}

/// The clients whose config has a kkt entry now, whatever server it names.
fn connected_clients(env: &Env, host: &dyn clients::Host) -> Result<Vec<Client>, StepFailure> {
    let mut connected = Vec::new();
    for client in Client::ALL {
        if client.connected(env, host)? {
            connected.push(client);
        }
    }
    Ok(connected)
}

/// The kkt server URL currently in `client`'s config, if present and safe to
/// print by kkt-deployment's server-address rule.
fn client_url(
    env: &Env,
    host: &dyn clients::Host,
    client: Client,
) -> Result<Option<String>, StepFailure> {
    Ok(client
        .configured_url(env, host)?
        .and_then(|url| kkt_deployment::connection::server_address(&url, None).ok()))
}

/// Points `client` at the server `record`'s clients use.
fn connect(
    env: &Env,
    host: &dyn clients::Host,
    command: &dyn kkt_llm_agent_config::ClientCommand,
    program: &dyn crate::SetupFiles,
    client: Client,
    record: &setup::Record,
) -> Result<String, StepFailure> {
    let executable = program.kkt_path()?;
    let client_config = client.file(env, host)?;
    let server = record.for_clients();
    let setup = Setup {
        env,
        url: &server.url,
        executable: &executable,
        key_file: server.key_file.as_deref(),
        client_config: &client_config,
        source: client.source(env),
    };
    Ok(clients::configure(client, &setup, true, host, command)?)
}

/// Checks, changing nothing, that `url` is a server address a client may be
/// given (kkt-deployment's rule, before any server is asked), then that a
/// server answers there and accepts its host name.
fn check_address(prober: &dyn kkt_diagnostics::Prober, url: &str) -> Result<(), StepFailure> {
    kkt_deployment::connection::server_address(url, None)?;
    let mut checks =
        tokio::runtime::Handle::current().block_on(diagnostics::inspect_address(prober, url))?;
    point_server_fix_at(
        &mut checks,
        "check the address, and that the server is running on its machine",
    );
    only_problems(checks)
}

/// Checks, changing nothing, that the server at `url` accepts `key`.
fn check_key(
    prober: &dyn kkt_diagnostics::Prober,
    url: &str,
    key: &setup::ClientKey,
) -> Result<(), StepFailure> {
    let runtime = tokio::runtime::Handle::current();
    let checks = match key {
        setup::ClientKey::None => {
            runtime.block_on(diagnostics::inspect_with_key(prober, url, None))
        }
        setup::ClientKey::Typed(key) => {
            runtime.block_on(diagnostics::inspect_with_key(prober, url, Some(key)))
        }
        setup::ClientKey::File(path) => {
            runtime.block_on(diagnostics::inspect(prober, url, Some(path)))
        }
    }?;
    only_problems(checks)
}

impl<C: ConnectionSource, D: crate::ServiceDefaults> setup::AdoptSteps for OsSteps<'_, '_, C, D> {
    fn unrecorded(&self) -> Result<Option<setup::Unrecorded>, setup::StepError> {
        self.find_unrecorded().map_err(step_error)
    }
}

impl<C: ConnectionSource, D: crate::ServiceDefaults> OsSteps<'_, '_, C, D> {
    /// A server kkt installed here that no record or note covers, as the
    /// adopt step finds it, in this crate's words.
    fn find_unrecorded(&self) -> Result<Option<setup::Unrecorded>, StepFailure> {
        if read_setup(self.parts.serve, self.parts.setup_files)?.is_some()
            || read_unfinished(
                self.parts.serve,
                self.parts.setup_files,
                self.parts.container,
                self.parts.service,
            )?
            .is_some()
        {
            return Ok(None);
        }
        read_unrecorded(self.parts, self.env, self.platform).map_err(|(_, e)| e.into())
    }
}

/// Why one of setup's steps failed: a sentence worded here, or the error of
/// what it called, passed on as it is.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StepFailure {
    #[error("set HOME to {0}")]
    NoHome(HomeFor),
    #[error("invalid setting: {problem} (in {})", config.display())]
    InvalidSetting {
        problem: crate::PartError,
        config: PathBuf,
    },
    #[error("could not remove {}: {cause}", path.display())]
    NotRemoved {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("{} is not in a folder to keep keys in", staged.display())]
    NoKeyFolder { staged: PathBuf },
    #[error("an admin key needs the server to have an access key")]
    AdminKeyNeedsAccessKey,
    #[error("an admin key is made on the server's machine")]
    AdminKeyOnClient,
    /// The doctor's lines for the checks that failed.
    #[error("{report}")]
    Checks { report: String },
    #[error(transparent)]
    Part(#[from] crate::PartError),
    #[error(transparent)]
    Sharing(#[from] kkt_remote_access::SharingError),
    #[error(transparent)]
    Service(#[from] crate::service::ServiceCommandError),
    #[error(transparent)]
    ServiceManager(#[from] kkt_bare_service::ServiceError),
    #[error(transparent)]
    Connection(#[from] kkt_deployment::connection::ConnectionError),
    #[error(transparent)]
    DockerHome(#[from] kkt_deployment::NoDockerHome),
    #[error(transparent)]
    Container(#[from] kkt_containerized_service::ContainerError),
    #[error(transparent)]
    KeyFile(#[from] kkt_keys::KeyFileError),
    #[error(transparent)]
    AdminKeyCollision(#[from] kkt_deployment::AdminKeyCollision),
    #[error(transparent)]
    Unprobed(#[from] kkt_diagnostics::Unprobed),
    #[error(transparent)]
    Record(#[from] crate::record::ReadError),
    #[error(transparent)]
    Unrecorded(#[from] crate::record::UnrecordedError),
    #[error(transparent)]
    Client(#[from] kkt_llm_agent_config::ClientConfigError),
    #[error(transparent)]
    InvalidKey(#[from] kkt_keys::SettingError),
    #[error(transparent)]
    Address(#[from] kkt_deployment::connection::AddressError),
}

/// What a step needs a home folder for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HomeFor {
    ConfigFile,
    AccessKey,
    Key,
    Note,
}

impl std::fmt::Display for HomeFor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HomeFor::ConfigFile => "keep kkt's config file",
            HomeFor::AccessKey => "keep the access key",
            HomeFor::Key => "keep the key in ~/.config/kkt",
            HomeFor::Note => "note the install",
        })
    }
}

/// Why a step failed, as kkt-setup's steps carry it.
fn step_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> setup::StepError {
    setup::StepError(error.into())
}

/// The doctor's lines for the problems in `checks` as an error, if any.
fn only_problems(checks: Vec<kkt_diagnostics::Check>) -> Result<(), StepFailure> {
    let problems: Vec<_> = checks.into_iter().filter(|check| !check.ok).collect();
    if problems.is_empty() {
        return Ok(());
    }
    Err(StepFailure::Checks {
        report: diagnostics::report(problems).0.trim_end().to_string(),
    })
}

/// The client key file's name, beside the config file.
const CLIENT_KEY_FILE_NAME: &str = "client-access-key";

pub(crate) fn staged_client_key(dir: &Path) -> PathBuf {
    dir.join(format!("{CLIENT_KEY_FILE_NAME}.partial"))
}

/// One key per server, so another server's saved key is never replaced. It
/// goes beside the staged key; a staged path in no folder is refused.
pub(crate) fn client_key(staged: &Path, url: &str) -> Result<PathBuf, StepFailure> {
    let dir = staged.parent().ok_or_else(|| StepFailure::NoKeyFolder {
        staged: staged.to_path_buf(),
    })?;
    let server: String = kkt_hash::sha256(url.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(dir.join(format!("{CLIENT_KEY_FILE_NAME}-{server}")))
}

/// What setup says while it waits for the server at `url`, set up as `mode`,
/// to answer.
pub(crate) fn waiting_note(url: &str, mode: &setup::Mode) -> String {
    let tailscale = match mode {
        setup::Mode::Docker(install) => install.tailscale,
        setup::Mode::Service { tailscale } => *tailscale,
        setup::Mode::Client => false,
    };
    if tailscale {
        format!(
            "Waiting for the server to answer at {url} (a new Tailscale node gets its \
             HTTPS certificate on first use)..."
        )
    } else {
        format!("Waiting for the server to answer at {url}...")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    // The steps' helpers, as the text a person reads.
    fn allow_host_in_config(
        files: &dyn crate::ConfigText,
        serve: &dyn crate::ServeFacts,
        host: &str,
    ) -> Result<(), String> {
        super::allow_host_in_config(files, serve, host).map_err(|e| e.to_string())
    }
    fn check_typed_key(key: &str) -> Result<(), String> {
        super::check_typed_key(key).map_err(|e| e.to_string())
    }
    fn client_key(staged: &Path, url: &str) -> Result<PathBuf, String> {
        super::client_key(staged, url).map_err(|e| e.to_string())
    }
    fn check_address(prober: &dyn kkt_diagnostics::Prober, url: &str) -> Result<(), String> {
        super::check_address(prober, url).map_err(|e| e.to_string())
    }
    fn only_problems(checks: Vec<kkt_diagnostics::Check>) -> Result<(), String> {
        super::only_problems(checks).map_err(|e| e.to_string())
    }

    /// A config file holding `text`, keeping what is written to it.
    struct ConfigFile {
        text: Option<&'static str>,
        read: RefCell<Vec<PathBuf>>,
        written: RefCell<Vec<(PathBuf, String)>>,
    }

    impl ConfigFile {
        fn holding(text: Option<&'static str>) -> Self {
            ConfigFile {
                text,
                read: RefCell::default(),
                written: RefCell::default(),
            }
        }
    }

    impl crate::ConfigText for ConfigFile {
        fn read(&self, path: &Path) -> Result<Option<String>, crate::PartError> {
            self.read.borrow_mut().push(path.to_path_buf());
            Ok(self.text.map(str::to_string))
        }
        fn write(&self, path: &Path, text: &str) -> Result<(), crate::PartError> {
            self.written
                .borrow_mut()
                .push((path.to_path_buf(), text.to_string()));
            Ok(())
        }
        /// The program's edit, as its own test pins it, for the texts
        /// these tests give.
        fn with_allowed_host(&self, text: &str, host: &str) -> Result<String, crate::PartError> {
            assert_eq!(host, "h.example");
            match text {
                "" => Ok("allow_hosts = [\"h.example\"]\n".to_string()),
                "allow_hosts = [\"h.example\"]\n" => Ok(text.to_string()),
                "allow_hosts = [" => {
                    Err("config is not valid TOML: unclosed array, expected `]`".into())
                }
                other => panic!("no edit given for {other:?}"),
            }
        }
    }

    /// Serving facts with the config places under the home `/h`, or with
    /// no home; the host edit asks nothing else.
    struct Home(bool);

    impl crate::ServeFacts for Home {
        fn config(&self, _: Option<PathBuf>) -> Result<crate::Config, crate::PartError> {
            unreachable!("the host edit reads no settings")
        }
        fn configured_access_key(&self) -> Result<Option<PathBuf>, crate::PartError> {
            unreachable!("the host edit reads no settings")
        }
        fn database(
            &self,
            _: Option<PathBuf>,
            _: Option<PathBuf>,
        ) -> Result<PathBuf, crate::PartError> {
            unreachable!("the host edit reads no settings")
        }
        fn bind(&self) -> Result<std::net::SocketAddr, crate::PartError> {
            unreachable!("the host edit reads no settings")
        }
        fn default_access_key(&self) -> Option<PathBuf> {
            unreachable!("the host edit reads no settings")
        }
        fn default_name_pack(&self) -> String {
            unreachable!("the host edit reads no settings")
        }
        fn config_dir(&self) -> Option<PathBuf> {
            self.0.then(|| PathBuf::from("/h/.config/kkt"))
        }
        fn config_file(&self) -> Option<PathBuf> {
            self.0.then(|| PathBuf::from("/h/.config/kkt/config.toml"))
        }
        fn config_file_in_home(&self) -> PathBuf {
            PathBuf::from("kkt/config.toml")
        }
        fn service_settings(
            &self,
            _: Option<PathBuf>,
            _: kkt_deployment::ServiceSettings,
        ) -> Result<crate::ServiceInputs<'_>, crate::PartError> {
            unreachable!("the host edit reads no settings")
        }
    }

    /// Setup adds a host to the default config file under the config home:
    /// it writes the edited text there, writes nothing when the host is
    /// already listed, and says which file it could not edit, or that it
    /// needs a home.
    #[test]
    fn the_host_edit_writes_the_config_file_and_says_what_failed() {
        let config = PathBuf::from("/h/.config/kkt/config.toml");
        let none = ConfigFile::holding(None);
        allow_host_in_config(&none, &Home(true), "h.example").expect("edited");
        assert_eq!(*none.read.borrow(), std::slice::from_ref(&config));
        assert_eq!(
            *none.written.borrow(),
            [(
                config.clone(),
                "allow_hosts = [\"h.example\"]\n".to_string()
            )]
        );

        let listed = ConfigFile::holding(Some("allow_hosts = [\"h.example\"]\n"));
        allow_host_in_config(&listed, &Home(true), "h.example").expect("edited");
        assert!(listed.written.borrow().is_empty());

        let broken = ConfigFile::holding(Some("allow_hosts = ["));
        assert_eq!(
            allow_host_in_config(&broken, &Home(true), "h.example"),
            Err(
                "invalid setting: config is not valid TOML: unclosed array, expected `]` \
                 (in /h/.config/kkt/config.toml)"
                    .to_string()
            )
        );
        assert!(broken.written.borrow().is_empty());

        let homeless = ConfigFile::holding(None);
        assert_eq!(
            allow_host_in_config(&homeless, &Home(false), "h.example"),
            Err("set HOME to keep kkt's config file".to_string())
        );
        assert!(homeless.read.borrow().is_empty());
    }

    #[test]
    fn a_malformed_typed_key_is_refused_with_kkt_keys_words() {
        assert_eq!(
            check_typed_key("short"),
            Err("access_key_file must hold a key of at least 32 characters".to_string())
        );
        assert_eq!(
            check_typed_key("0123456789abcdef 0123456789abcdef"),
            Err("access_key_file must hold only printable ASCII with no spaces".to_string())
        );
        assert_eq!(
            check_typed_key("0123456789abcdefghijklmnopqrstuvwxyz"),
            Ok(())
        );
    }

    #[test]
    fn the_client_key_names_and_the_waiting_note_are_pinned() {
        // The client key names, hash and all.
        let staged = staged_client_key(Path::new("/c/kkt"));
        assert_eq!(staged, PathBuf::from("/c/kkt/client-access-key.partial"));
        assert_eq!(
            client_key(&staged, "https://a.example/mcp").expect("in a folder"),
            PathBuf::from("/c/kkt/client-access-key-42c16f3bfd38")
        );
        // The waiting note, plain and over Tailscale.
        assert_eq!(
            waiting_note("http://h/mcp", &setup::Mode::Service { tailscale: false }),
            "Waiting for the server to answer at http://h/mcp..."
        );
        assert_eq!(
            waiting_note("https://h/mcp", &setup::Mode::Service { tailscale: true }),
            "Waiting for the server to answer at https://h/mcp (a new Tailscale node gets its \
             HTTPS certificate on first use)..."
        );
        assert_eq!(
            waiting_note(
                "https://h/mcp",
                &setup::Mode::Docker(setup::DockerSetup {
                    folder: PathBuf::from("/d"),
                    tailscale: true,
                    hostname: None,
                    tailnet: None,
                })
            ),
            "Waiting for the server to answer at https://h/mcp (a new Tailscale node gets its \
             HTTPS certificate on first use)..."
        );
        assert_eq!(
            waiting_note("http://h/mcp", &setup::Mode::Client),
            "Waiting for the server to answer at http://h/mcp..."
        );
    }

    #[test]
    fn a_client_key_goes_beside_the_staged_one_named_for_its_server() {
        let staged = staged_client_key(Path::new("/c/kkt"));
        let kept = client_key(&staged, "https://a.example/mcp").expect("in a folder");
        assert_eq!(kept.parent(), Some(Path::new("/c/kkt")));
        assert_ne!(
            kept,
            client_key(&staged, "https://b.example/mcp").expect("in a folder")
        );
    }

    #[test]
    fn a_path_in_no_folder_is_refused_not_taken_as_here() {
        assert_eq!(
            client_key(Path::new("/"), "https://a.example/mcp"),
            Err("/ is not in a folder to keep keys in".to_string())
        );
    }

    use super::*;

    /// Every part a step's own refusal must not reach: each one fails the
    /// test, naming the part.
    struct Unused;

    impl kkt_containerized_service::ConfigReader for Unused {
        fn settings(
            &self,
            _: &str,
            _: &Path,
            _: Option<&Path>,
        ) -> Result<
            kkt_containerized_service::ContainerConfig,
            kkt_containerized_service::ConfigUnread,
        > {
            unreachable!("container_configs")
        }
    }
    impl kkt_containerized_service::LoginLink for Unused {
        fn show(&self, _: &str) {
            unreachable!("login_link")
        }
        fn open_requested(&self) -> bool {
            unreachable!("login_link")
        }
    }
    impl kkt_remote_access::RemoteAccess for Unused {
        fn prepare(&self) -> Result<kkt_remote_access::Node, kkt_remote_access::SharingError> {
            unreachable!("remote")
        }
        fn serve(&self, _: std::net::SocketAddr) -> Result<(), kkt_remote_access::SharingError> {
            unreachable!("remote")
        }
        fn served(
            &self,
            _: std::net::SocketAddr,
        ) -> Result<Option<kkt_remote_access::Node>, kkt_remote_access::SharingError> {
            unreachable!("remote")
        }
        fn stop(&self, _: std::net::SocketAddr) -> Result<(), kkt_remote_access::SharingError> {
            unreachable!("remote")
        }
    }
    impl clients::Host for Unused {
        fn read(&self, _: &Path) -> std::io::Result<String> {
            unreachable!("clients")
        }
        fn read_independent_file(&self, _: &Path, _: &Path) -> std::io::Result<Option<String>> {
            unreachable!("clients")
        }
        fn list_dir(&self, _: &Path) -> std::io::Result<Vec<std::ffi::OsString>> {
            unreachable!("clients")
        }
        fn exists(&self, _: &Path) -> bool {
            unreachable!("clients")
        }
        fn create_private(&self, _: &Path, _: &str) -> std::io::Result<()> {
            unreachable!("clients")
        }
        fn restrict(&self, _: &Path) -> std::io::Result<()> {
            unreachable!("clients")
        }
        fn mode(&self, _: &Path) -> std::io::Result<Option<u32>> {
            unreachable!("clients")
        }
        fn set_mode(&self, _: &Path, _: u32) -> std::io::Result<()> {
            unreachable!("clients")
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            unreachable!("clients")
        }
        fn rename(&self, _: &Path, _: &Path) -> std::io::Result<()> {
            unreachable!("clients")
        }
        fn remove_file(&self, _: &Path) -> std::io::Result<()> {
            unreachable!("clients")
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!("clients")
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!("clients")
        }
        fn process_id(&self) -> u32 {
            unreachable!("clients")
        }
    }
    impl clients::ClientCommand for Unused {
        fn remove_server(&self, _: Option<&Path>) -> Result<(), clients::CommandError> {
            unreachable!("client_command")
        }
        fn add_server(&self, _: &str, _: Option<&Path>) -> Result<(), clients::CommandError> {
            unreachable!("client_command")
        }
    }
    impl crate::ImportLanes for Unused {
        fn check(&self, _: &Path, _: setup::Purpose) -> Result<usize, crate::PartError> {
            unreachable!("import_lanes")
        }
        fn import(&self, _: &setup::Record, _: &Path) -> Result<String, crate::PartError> {
            unreachable!("import_lanes")
        }
    }
    impl kkt_setup::Lines for Unused {
        fn interactive(&self) -> bool {
            unreachable!("lines")
        }
        fn say(&self, _: &str) {
            unreachable!("lines")
        }
        fn read_line(&self, _: &str) -> Result<Option<String>, kkt_setup::LineError> {
            unreachable!("lines")
        }
    }
    impl ConnectionSource for Unused {
        type KeySource = ();
        fn recorded(&self) -> Result<Option<connection::RecordedServer>, connection::SourceError> {
            unreachable!("connection")
        }
        fn serve(
            &self,
            _: ConnectionOptions,
        ) -> Result<connection::ServeConnection<()>, connection::SourceError> {
            unreachable!("connection")
        }
        fn make_default_key(&self, _: &kkt_keys::AccessKey) -> Result<(), connection::SourceError> {
            unreachable!("connection")
        }
        fn check_key(&self, _: &Path, _: &()) -> Result<(), connection::SourceError> {
            unreachable!("connection")
        }
        fn absolute(&self, _: &Path) -> Result<PathBuf, connection::SourceError> {
            unreachable!("connection")
        }
    }
    impl crate::ServiceDefaults for Unused {
        fn endpoint(
            &self,
            _: &[String],
        ) -> Result<(std::net::SocketAddr, Option<PathBuf>), crate::PartError> {
            unreachable!("service endpoint")
        }

        fn matches_defaults(&self, _: &[String]) -> Result<bool, crate::PartError> {
            unreachable!("service_defaults")
        }
    }
    impl crate::SetupFiles for Unused {
        fn read_record(&self, _: &Path) -> Result<Option<setup::Record>, crate::PartError> {
            unreachable!("setup_files")
        }
        fn read_unfinished(&self, _: &Path) -> Result<Option<setup::Unfinished>, crate::PartError> {
            unreachable!("setup_files")
        }
        fn write_record(&self, _: &Path, _: &setup::Record) -> Result<(), crate::PartError> {
            unreachable!("setup_files")
        }
        fn write_unfinished(
            &self,
            _: &Path,
            _: &setup::Unfinished,
        ) -> Result<(), crate::PartError> {
            unreachable!("setup_files")
        }
        fn remove_unfinished(&self, _: &Path) -> Result<(), crate::PartError> {
            unreachable!("setup_files")
        }
        fn kkt_path(&self) -> Result<PathBuf, crate::PartError> {
            unreachable!("setup_files")
        }
        fn path_dirs(&self) -> Vec<PathBuf> {
            unreachable!("setup_files")
        }
        fn on_path(&self, _: &[PathBuf], _: &str) -> bool {
            unreachable!("setup_files")
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!("setup_files")
        }
        fn program_path(&self) -> std::io::Result<PathBuf> {
            unreachable!("setup_files")
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!("setup_files")
        }
        fn remove_record(&self, _: &Path) -> Result<(), crate::PartError> {
            unreachable!("setup_files")
        }
    }
    impl crate::Databases for Unused {
        fn exists(&self, _: &Path) -> bool {
            unreachable!("database")
        }
        fn highest_message_id(&self, _: &Path) -> Result<Option<i64>, crate::PartError> {
            unreachable!("database")
        }
    }
    impl kkt_backup::Backups for Unused {
        fn write(
            &self,
            _: &Path,
            _: &kkt_backup::Layout<'_>,
        ) -> Result<PathBuf, kkt_backup::BackupError> {
            unreachable!("backups")
        }
        fn record_highest(&self, _: &Path, _: i64) -> Result<(), kkt_backup::BackupError> {
            unreachable!("backups")
        }
        fn finished(&self, _: &Path) -> Result<Vec<PathBuf>, kkt_backup::BackupError> {
            unreachable!("no backups listed")
        }
        fn prune(&self, _: &Path, _: usize) -> Result<Vec<PathBuf>, kkt_backup::BackupError> {
            unreachable!("backups")
        }
    }
    impl crate::Output for Unused {
        fn output(&self, _: std::fmt::Arguments<'_>) {
            unreachable!("output")
        }
        fn errors(&self, _: std::fmt::Arguments<'_>) {
            unreachable!("output")
        }
    }
    impl kkt_clock::Clock for Unused {
        fn now(&self) -> kkt_clock::Time {
            unreachable!("clock")
        }
        fn sleep_until(&self, _: kkt_clock::Time) -> futures::future::BoxFuture<'_, ()> {
            unreachable!("clock")
        }
    }
    struct NoProbes;
    impl kkt_diagnostics::Prober for NoProbes {
        fn key_file(
            &self,
            _: &Path,
        ) -> Result<kkt_diagnostics::KeyFile, kkt_diagnostics::KeyFileUnread> {
            unreachable!("prober")
        }
        fn answer<'a>(
            &'a self,
            _: &'a str,
        ) -> kkt_diagnostics::Probing<'a, kkt_diagnostics::Answered> {
            unreachable!("prober")
        }
        fn initialize<'a>(
            &'a self,
            _: &'a str,
            _: Option<&'a str>,
        ) -> kkt_diagnostics::Probing<'a, kkt_diagnostics::Initialized> {
            unreachable!("prober")
        }
        fn tls<'a>(&'a self, _: &'a str) -> kkt_diagnostics::Probing<'a, kkt_diagnostics::Tls> {
            unreachable!("prober")
        }
    }

    /// Runs `step` on setup's steps over the parts given here: the home
    /// `/h`, or none, and `key_files`; every other part is [`Unused`].
    fn on_steps<T>(
        home: bool,
        key_files: &kkt_keys::testing::Memory,
        step: impl FnOnce(&OsSteps<'_, '_, Unused, Unused>) -> T,
    ) -> T {
        let env = Env {
            home: Some(PathBuf::from("/h")),
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let service = kkt_bare_service::testing::Memory::installed(vec!["serve".into()], true);
        let container = kkt_containerized_service::testing::Memory::new(None, String::new());
        let try_bind =
            |_: std::net::SocketAddr| -> std::io::Result<()> { unreachable!("try_bind") };
        let config_file = ConfigFile::holding(None);
        let parts = crate::InstallParts {
            container: &container,
            container_configs: &Unused,
            login_link: &Unused,
            service: &service,
            try_bind: &try_bind,
            remote: &Unused,
            clients: &Unused,
            client_command: &Unused,
            prober: &NoProbes,
            import_lanes: &Unused,
            lines: &Unused,
            config_file: &config_file,
            connection: &Unused,
            clock: std::sync::Arc::new(Unused),
            serve: &Home(home),
            service_defaults: &Unused,
            setup_files: &Unused,
            key_files,
            entropy: &kkt_entropy::testing::SeqEntropy::new(0),
            database: &Unused,
            backups: &Unused,
            output: &Unused,
            mcp_path: "/mcp",
        };
        let steps = OsSteps {
            env: &env,
            platform: Platform::Xdg,
            parts: &parts,
            options: InstallOptions {
                kkt_version: "0.0.0".into(),
                docker_project: "kkt".into(),
                docker_port: 7420,
            },
        };
        step(&steps)
    }

    fn record(mode: setup::Mode, key_file: Option<&str>) -> setup::Record {
        setup::Record {
            mode,
            written_by: "0.0.0".into(),
            url: "http://127.0.0.1:7420/mcp".into(),
            key_file: key_file.map(PathBuf::from),
            clients: None,
        }
    }

    /// Each sentence a step words itself, exactly.
    #[test]
    fn the_steps_own_refusals_are_pinned() {
        use setup::Steps as _;
        fn said<T>(result: Result<T, setup::StepError>) -> String {
            match result {
                Ok(_) => panic!("the step was refused"),
                Err(e) => e.to_string(),
            }
        }
        let keys = kkt_keys::testing::Memory::new();
        assert_eq!(
            on_steps(false, &keys, |s| said(s.install_docker(None, &[]))),
            "set HOME to keep the access key"
        );
        assert_eq!(
            on_steps(false, &keys, |s| said(s.stage_key("k"))),
            "set HOME to keep the key in ~/.config/kkt"
        );
        let unfinished = setup::Unfinished {
            purpose: setup::Purpose::Service,
            tailscale: false,
            allow_hosts: Vec::new(),
            clients: Vec::new(),
            tailscale_login: None,
            record: None,
        };
        assert_eq!(
            on_steps(false, &keys, |s| said(s.save_unfinished(&unfinished))),
            "set HOME to note the install"
        );
        assert_eq!(
            on_steps(true, &keys, |s| said(
                s.discard_key(Path::new("/h/.config/kkt/k.partial"))
            )),
            "could not remove /h/.config/kkt/k.partial: no such file"
        );
        let name =
            AdminName::new("ops", kkt_names::MAX_ADMIN_NAME_CHARS).expect("a valid admin name");
        assert_eq!(
            on_steps(true, &keys, |s| said(s.add_admin_key(
                &record(setup::Mode::Service { tailscale: false }, None),
                &name
            ))),
            "an admin key needs the server to have an access key"
        );
        assert_eq!(
            on_steps(true, &keys, |s| said(s.add_admin_key(
                &record(
                    setup::Mode::Client,
                    Some("/h/.config/kkt/client-access-key")
                ),
                &name
            ))),
            "an admin key is made on the server's machine"
        );
    }

    /// An admin key whose file cannot be made says why, in the key file's
    /// words, through the service's `add_key`.
    #[test]
    fn an_admin_key_that_cannot_be_made_says_why() {
        use setup::Steps as _;
        let access = "/h/.config/kkt/access-key";
        let admin = kkt_keys::admin_key_path(Path::new(access)).expect("beside the access key");
        let keys = kkt_keys::testing::Memory::new().failing(admin);
        let name =
            AdminName::new("ops", kkt_names::MAX_ADMIN_NAME_CHARS).expect("a valid admin name");
        let said = on_steps(true, &keys, |s| {
            s.add_admin_key(
                &record(setup::Mode::Service { tailscale: false }, Some(access)),
                &name,
            )
            .map(drop)
            .map_err(|e| e.to_string())
        });
        assert_eq!(
            said,
            Err("could not write /h/.config/kkt/admin-key: refused".to_string())
        );
    }

    /// The doctor's lines for the checks that failed, and nothing for the
    /// ones that passed.
    #[test]
    fn only_the_failed_checks_are_said() {
        let failed = kkt_diagnostics::Check {
            name: "server",
            ok: false,
            detail: "no answer at http://h/mcp".into(),
            fix: Some("start the server".into()),
        };
        let passed = kkt_diagnostics::Check::ok("key", "accepted");
        assert_eq!(only_problems(vec![passed.clone()]), Ok(()));
        assert_eq!(
            only_problems(vec![passed, failed.clone()]),
            Err(kkt_diagnostics::report(vec![failed])
                .0
                .trim_end()
                .to_string())
        );
    }

    fn docker(tailscale: bool) -> setup::Mode {
        setup::Mode::Docker(setup::DockerSetup {
            folder: PathBuf::from("/data/kkt docker"),
            tailscale,
            hostname: None,
            tailnet: None,
        })
    }

    /// What setup and switch refuse as a server address, and how they say
    /// so, before any server is asked: this test has no runtime, so asking a
    /// server would panic.
    #[test]
    fn an_address_is_refused_before_any_server_is_asked() {
        let prober = kkt_diagnostics::testing::Memory::new();
        let absolute = "URL must be an absolute http:// or https:// address";
        let secret = "put credentials in the key file, not in the URL; \
                      query strings and fragments are not supported";
        for (url, said) in [
            ("ftp://h/mcp", absolute),
            ("not a url", absolute),
            ("h:7420/mcp", absolute),
            ("http://h/\u{7}", "URL contains a control character"),
            ("http://u:p@h/mcp", secret),
            ("http://u@h/mcp", secret),
            ("http://h/mcp?q=1", secret),
            ("http://h/mcp#f", secret),
        ] {
            assert_eq!(check_address(&prober, url), Err(said.to_string()), "{url}");
        }
    }

    #[test]
    fn the_wait_names_tailscale_only_when_the_server_is_shared_over_it() {
        let url = "http://127.0.0.1:8080/mcp";
        for (mode, shared) in [
            (docker(false), false),
            (docker(true), true),
            (setup::Mode::Service { tailscale: false }, false),
            (setup::Mode::Service { tailscale: true }, true),
        ] {
            let note = waiting_note(url, &mode);
            assert!(
                note.starts_with(&format!("Waiting for the server to answer at {url}")),
                "{note}"
            );
            assert_eq!(note.contains("Tailscale"), shared, "{note}");
        }
    }
}
