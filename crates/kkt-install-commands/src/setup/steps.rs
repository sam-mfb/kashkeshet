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
) -> Result<(), String> {
    let path = serve
        .config_file()
        .ok_or("set HOME to keep kkt's config file")?;
    let text = files
        .read(&path)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(String::new);
    let updated = files
        .with_allowed_host(&text, host)
        .map_err(|e| format!("invalid setting: {e} (in {})", path.display()))?;
    if updated == text {
        return Ok(());
    }
    files.write(&path, &updated).map_err(|e| e.to_string())
}

impl<C: ConnectionSource, D: crate::ServiceDefaults> setup::Steps for OsSteps<'_, '_, C, D> {
    fn install_service(
        &self,
        tailscale: bool,
        allow_hosts: &[String],
    ) -> Result<setup::Installed, setup::StepError> {
        (|| -> Result<setup::Installed, String> {
            // Before anything changes: Tailscale must be logged in, with HTTPS.
            let node = match tailscale {
                true => Some(self.parts.remote.prepare().map_err(|e| e.to_string())?),
                false => None,
            };
            if let Some(node) = &node {
                allow_host_in_config(self.parts.config_file, self.parts.serve, &node.dns_name)?;
            }
            for host in allow_hosts {
                allow_host_in_config(self.parts.config_file, self.parts.serve, host)?;
            }
            tokio::runtime::Handle::current().block_on(service::run(
                self.parts,
                ServiceAction::Install(ServiceInstall {
                    config: None,
                    db: None,
                    access_key_file: None,
                    no_access_key: false,
                    dry_run: false,
                }),
            ))?;
            let connection =
                connection::serve_connection(self.parts.connection, ConnectionOptions::NONE, false)
                    .map_err(|e| e.to_string())?;
            let url = match &node {
                Some(node) => {
                    self.parts
                        .remote
                        .serve(self.parts.serve.bind().map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                    format!("https://{}/mcp", node.dns_name)
                }
                None => connection.url,
            };
            if let Some(access) = &connection.key_file {
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

    fn install_docker(
        &self,
        tailscale: Option<&setup::TailscaleLogin>,
        allow_hosts: &[String],
    ) -> Result<(setup::Installed, setup::DockerSetup), setup::StepError> {
        (|| -> Result<(setup::Installed, setup::DockerSetup), String> {
            let options = &self.options;
            let folder = docker_folder(self.env, self.platform).map_err(|e| e.to_string())?;
            let key_file = self
                .parts
                .serve
                .config_dir()
                .ok_or("set HOME to keep the access key")?
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
            let provisioned = tokio::runtime::Handle::current()
                .block_on(self.parts.container.provision(
                    self.parts.clock.as_ref(),
                    self.parts.login_link,
                    &kkt_container::Folder(folder.clone()),
                    &inputs,
                    &kkt_deployment::docker_key_copies(&key_file),
                ))
                .map_err(|e| e.to_string())?;
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
        (|| -> Result<PathBuf, String> {
            let dir = self
                .parts
                .serve
                .config_dir()
                .ok_or("set HOME to keep the key in ~/.config/kkt")?;
            self.parts
                .key_files
                .create_dir_all(&dir)
                .map_err(|e| e.to_string())?;
            let path = staged_client_key(&dir);
            self.parts
                .key_files
                .write_key(&path, key, true)
                .map_err(|e| e.to_string())?;
            Ok(path)
        })()
        .map_err(step_error)
    }

    fn keep_key(&self, staged: &Path, url: &str) -> Result<PathBuf, setup::StepError> {
        (|| -> Result<PathBuf, String> {
            let path = client_key(staged, url)?;
            self.parts
                .key_files
                .keep_staged(staged, &path)
                .map_err(|e| e.to_string())?;
            Ok(path)
        })()
        .map_err(step_error)
    }

    fn discard_key(&self, staged: &Path) -> Result<(), setup::StepError> {
        self.parts
            .key_files
            .remove(staged)
            .map_err(|e| format!("could not remove {}: {e}", staged.display()))
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
        clients::allow_kkt(client, self.env, self.parts.clients)
            .map_err(|e| e.to_string())
            .map_err(step_error)
    }

    fn doctor(&self, record: &setup::Record) -> Result<String, setup::StepError> {
        (|| -> Result<String, String> {
            // The server the clients use: on a machine whose clients were
            // switched, another machine's, checked as a client's.
            let server = record.for_clients();
            let mode = clients_server_mode(record.clients.is_some(), &record.mode);
            let kind = match mode {
                setup::Mode::Client => diagnostics::SetupServer::Elsewhere,
                _ => diagnostics::SetupServer::New,
            };
            let note = waiting_note(&server.url, &mode);
            let mut checks = tokio::runtime::Handle::current()
                .block_on(diagnostics::until_reachable(
                    Inspector::new(self.parts.prober),
                    self.parts.clock.as_ref(),
                    &server.url,
                    server.key_file.as_deref(),
                    diagnostics::setup_wait(diagnostics::Settings::DEFAULT, kind),
                    &mut || self.parts.output.output(format_args!("{note}\n")),
                ))
                .map_err(|e| e.to_string())?;
            if let Some(fix) = diagnostics::server_fix(&setup_of(&mode)) {
                point_server_fix_at(&mut checks, &fix);
            }
            match diagnostics::report(checks) {
                (report, true) => Ok(report.trim_end().to_string()),
                (report, false) => Err(report.trim_end().to_string()),
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
        .map_err(|e| e.to_string())
        .map_err(step_error)
    }

    fn save_unfinished(&self, unfinished: &setup::Unfinished) -> Result<(), setup::StepError> {
        (|| -> Result<(), String> {
            let config_dir = self
                .parts
                .serve
                .config_dir()
                .ok_or("set HOME to note the install")?;
            self.parts
                .setup_files
                .write_unfinished(&config_dir, unfinished)
                .map_err(|e| e.to_string())
        })()
        .map_err(step_error)
    }

    fn forget_unfinished(&self) -> Result<(), setup::StepError> {
        (|| -> Result<(), String> {
            let Some(config_dir) = self.parts.serve.config_dir() else {
                return Ok(());
            };
            self.parts
                .setup_files
                .remove_unfinished(&config_dir)
                .map_err(|e| e.to_string())
        })()
        .map_err(step_error)
    }

    fn add_admin_key(
        &self,
        record: &setup::Record,
        name: &AdminName,
    ) -> Result<PathBuf, setup::StepError> {
        (|| -> Result<PathBuf, String> {
        let access = record
            .key_file
            .as_deref()
            .ok_or("an admin key needs the server to have an access key")?;
        match &record.mode {
            setup::Mode::Docker(install) => tokio::runtime::Handle::current()
                .block_on(self.parts.container.add_admin_key(
                    self.parts.clock.as_ref(),
                    &kkt_container::Folder(install.folder.clone()),
                    &kkt_deployment::docker_key_copies(access),
                    name.as_str(),
                ))
                .map_err(|e| e.to_string()),
            setup::Mode::Service { .. } => {
                let admin = kkt_keys::admin_key_path(access)
                    .ok_or_else(|| admin_key_collision().to_string())?;
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
                                    .map_err(|e| e.to_string())?;
                            kkt_deployment::admin_name_written(
                                path,
                                Some(&name.to_string()),
                                self.parts.key_files.write_admin_name(
                                    path,
                                    Some(kkt_keys::bound_to(name, &key).as_str()),
                                ),
                            )
                            .map(drop)
                            .map_err(|e| e.to_string())
                        })
                    })
                    .map_err(|e| e.to_string())?;
                self.parts.output.output(format_args!(
                    "{} admin key for /mcp-admin is in {}, and the server was restarted to use it. Read it with `kkt key show --admin`.\n",
                    if made { "A new" } else { "The" },
                    admin.display()
                ));
                Ok(admin)
            }
            setup::Mode::Client => Err("an admin key is made on the server's machine".into()),
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
            .map_err(|e| e.to_string())
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
            .map_err(|e| e.to_string())
            .map_err(step_error)
    }

    fn check_typed_key(&self, key: &str) -> Result<(), setup::StepError> {
        check_typed_key(key).map_err(step_error)
    }
}

/// Checks that `key`, typed in for a client, is a valid access key, by
/// kkt-keys' rule. Returns why not.
fn check_typed_key(key: &str) -> Result<(), String> {
    kkt_keys::check_access_key(key).map_err(|e| e.to_string())
}

impl<C: ConnectionSource, D> setup::SwitchSteps for OsSteps<'_, '_, C, D> {
    fn connected_clients(&self) -> Result<Vec<Client>, setup::StepError> {
        connected_clients(self.env, self.parts.clients).map_err(step_error)
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

    fn check_address(&self, url: &str) -> Result<(), setup::StepError> {
        check_address(self.parts.prober, url).map_err(step_error)
    }

    fn check_key(&self, url: &str, key: &setup::ClientKey) -> Result<(), setup::StepError> {
        check_key(self.parts.prober, url, key).map_err(step_error)
    }
}

/// The clients whose config has a kkt entry now, whatever server it names.
fn connected_clients(env: &Env, host: &dyn clients::Host) -> Result<Vec<Client>, String> {
    let mut connected = Vec::new();
    for client in Client::ALL {
        if client.connected(env, host).map_err(|e| e.to_string())? {
            connected.push(client);
        }
    }
    Ok(connected)
}

/// Points `client` at the server `record`'s clients use.
fn connect(
    env: &Env,
    host: &dyn clients::Host,
    command: &dyn kkt_llm_agent_config::ClientCommand,
    program: &dyn crate::SetupFiles,
    client: Client,
    record: &setup::Record,
) -> Result<String, String> {
    let executable = program.kkt_path().map_err(|e| e.to_string())?;
    let client_config = client.file(env, host).map_err(|e| e.to_string())?;
    let server = record.for_clients();
    let setup = Setup {
        env,
        url: &server.url,
        executable: &executable,
        key_file: server.key_file.as_deref(),
        client_config: &client_config,
        source: client.source(env),
    };
    clients::configure(client, &setup, true, host, command).map_err(|e| e.to_string())
}

/// Checks, changing nothing, that `url` is a server address a client may be
/// given (kkt-deployment's rule, before any server is asked), then that a
/// server answers there and accepts its host name.
fn check_address(prober: &dyn kkt_diagnostics::Prober, url: &str) -> Result<(), String> {
    kkt_deployment::connection::server_address(url, None).map_err(|e| e.to_string())?;
    let mut checks = tokio::runtime::Handle::current()
        .block_on(diagnostics::inspect_address(prober, url))
        .map_err(|e| e.to_string())?;
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
) -> Result<(), String> {
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
    }
    .map_err(|e| e.to_string())?;
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
    fn find_unrecorded(&self) -> Result<Option<setup::Unrecorded>, String> {
        if read_setup(self.parts.serve, self.parts.setup_files)
            .map_err(|e| e.to_string())?
            .is_some()
            || read_unfinished(
                self.parts.serve,
                self.parts.setup_files,
                self.parts.container,
                self.parts.service,
            )
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Ok(None);
        }
        read_unrecorded(self.parts, self.env, self.platform).map_err(|(_, e)| e.to_string())
    }
}

/// A step's text, as kkt-setup's steps carry it.
fn step_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> setup::StepError {
    setup::StepError(error.into())
}

/// The doctor's lines for the problems in `checks` as an error, if any.
fn only_problems(checks: Vec<kkt_diagnostics::Check>) -> Result<(), String> {
    let problems: Vec<_> = checks.into_iter().filter(|check| !check.ok).collect();
    if problems.is_empty() {
        return Ok(());
    }
    Err(diagnostics::report(problems).0.trim_end().to_string())
}

/// The client key file's name, beside the config file.
const CLIENT_KEY_FILE_NAME: &str = "client-access-key";

pub(crate) fn staged_client_key(dir: &Path) -> PathBuf {
    dir.join(format!("{CLIENT_KEY_FILE_NAME}.partial"))
}

/// One key per server, so another server's saved key is never replaced. It
/// goes beside the staged key; a staged path in no folder is refused.
pub(crate) fn client_key(staged: &Path, url: &str) -> Result<PathBuf, String> {
    let dir = staged
        .parent()
        .ok_or_else(|| format!("{} is not in a folder to keep keys in", staged.display()))?;
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
        super::allow_host_in_config(&none, &Home(true), "h.example").expect("edited");
        assert_eq!(*none.read.borrow(), std::slice::from_ref(&config));
        assert_eq!(
            *none.written.borrow(),
            [(
                config.clone(),
                "allow_hosts = [\"h.example\"]\n".to_string()
            )]
        );

        let listed = ConfigFile::holding(Some("allow_hosts = [\"h.example\"]\n"));
        super::allow_host_in_config(&listed, &Home(true), "h.example").expect("edited");
        assert!(listed.written.borrow().is_empty());

        let broken = ConfigFile::holding(Some("allow_hosts = ["));
        assert_eq!(
            super::allow_host_in_config(&broken, &Home(true), "h.example"),
            Err(
                "invalid setting: config is not valid TOML: unclosed array, expected `]` \
                 (in /h/.config/kkt/config.toml)"
                    .to_string()
            )
        );
        assert!(broken.written.borrow().is_empty());

        let homeless = ConfigFile::holding(None);
        assert_eq!(
            super::allow_host_in_config(&homeless, &Home(false), "h.example"),
            Err("set HOME to keep kkt's config file".to_string())
        );
        assert!(homeless.read.borrow().is_empty());
    }

    #[test]
    fn a_malformed_typed_key_is_refused_with_kkt_keys_words() {
        assert_eq!(
            super::check_typed_key("short"),
            Err("access_key_file must hold a key of at least 32 characters".to_string())
        );
        assert_eq!(
            super::check_typed_key("0123456789abcdef 0123456789abcdef"),
            Err("access_key_file must hold only printable ASCII with no spaces".to_string())
        );
        assert_eq!(
            super::check_typed_key("0123456789abcdefghijklmnopqrstuvwxyz"),
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
