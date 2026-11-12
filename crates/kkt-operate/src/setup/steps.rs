//! [`OsSteps`] and [`OsSwitchSteps`]: what `kkt setup` and `kkt switch` do
//! to this machine, composed from the program's supplied providers.

use super::InstallOptions;
use crate::doctor::server_fix;
use crate::keys::{admin_key_collision, new_key_file, write_admin_name, write_key_file};
use crate::lanes::{
    check_lanes_folder, docker_import_lanes, docker_lane_limit, local_import_lanes,
    local_lane_limit,
};
use crate::record::{
    DOCKER_KEY_FILE_NAME, docker_folder, docker_url, read_setup, read_unfinished, read_unrecorded,
    write_setup_record,
};
use crate::service::{self, ServiceAction, ServiceInstall};
use kkt_client_config::{self as clients, Client, Setup};
use kkt_diagnostics::{self as diagnostics, Inspector, point_server_fix_at};
use kkt_lane_files as lane_files;
use kkt_machine::{Env, Platform};
use kkt_names::AdminName;
use kkt_service::Manager;
use kkt_setup as setup;
use kkt_setup::connection::{self, ConnectionOptions, ConnectionSource};
use setup::waiting_note;
use std::path::{Path, PathBuf};

/// What `kkt setup` does to this machine.
pub(super) struct OsSteps<'a, 'p, C: ConnectionSource, D> {
    pub(super) env: &'a Env,
    pub(super) platform: Platform,
    pub(super) parts: &'a crate::OperateParts<'p, C, D>,
    pub(super) options: InstallOptions,
}

/// Adds `host` to `allow_hosts` in the default config file, making the file
/// if there is none.
fn allow_host_in_config(
    files: &dyn crate::ConfigText,
    env: &Env,
    host: &str,
) -> Result<(), String> {
    let path = kkt_config_file::config_file(env.config_home().as_deref())
        .ok_or("set HOME to keep kkt's config file")?;
    let text = files
        .read(&path)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(String::new);
    let updated = kkt_config_file::with_allowed_host(&text, host)
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
    ) -> Result<setup::Installed, String> {
        // Before anything changes: Tailscale must be logged in, with HTTPS.
        let node = match tailscale {
            true => Some(kkt_remote_access::prepare(self.parts.remote)?),
            false => None,
        };
        if let Some(node) = &node {
            allow_host_in_config(self.parts.config_file, self.env, &node.dns_name)?;
        }
        for host in allow_hosts {
            allow_host_in_config(self.parts.config_file, self.env, host)?;
        }
        tokio::runtime::Handle::current().block_on(service::run(
            self.parts,
            self.env,
            ServiceAction::Install(ServiceInstall {
                config: None,
                db: None,
                access_key_file: None,
                no_access_key: false,
                dry_run: false,
            }),
        ))?;
        let connection =
            connection::serve_connection(self.parts.connection, ConnectionOptions::NONE, false)?;
        let url = match &node {
            Some(node) => {
                kkt_remote_access::serve(self.parts.remote, self.parts.serve.bind()?)?;
                format!("https://{}/mcp", node.dns_name)
            }
            None => connection.url,
        };
        if let Some(access) = &connection.key_file {
            self.parts.output.output(format_args!(
                "The web key is in {}. Read it with `kkt key show --web`.\n",
                kkt_admission::web_key_path(access).display()
            ));
        }
        Ok(setup::Installed {
            url,
            key_file: connection.key_file,
        })
    }

    fn install_docker(
        &self,
        tailscale: Option<&setup::TailscaleLogin>,
        allow_hosts: &[String],
    ) -> Result<(setup::Installed, setup::DockerSetup), String> {
        let options = &self.options;
        let folder = docker_folder(self.env, self.platform)?;
        let key_file = kkt_config_file::config_dir(self.env.config_home().as_deref())
            .ok_or("set HOME to keep the access key")?
            .join(DOCKER_KEY_FILE_NAME);
        let tailscale_inputs = match tailscale {
            Some(login) => Some(kkt_container::Tailscale {
                hostname: login.hostname.clone(),
                auth_key: match &login.auth_key_file {
                    Some(path) => Some(kkt_container::tailscale_auth_key(
                        self.parts.container,
                        path,
                    )?),
                    None => None,
                },
            }),
            None => None,
        };
        let inputs = kkt_container::Inputs {
            version: options.kkt_version.clone(),
            project: options.docker_project.clone(),
            port: options.docker_port,
            allow_hosts: allow_hosts.to_vec(),
            tailscale: tailscale_inputs,
        };
        let tailscale = tailscale.is_some();
        let provisioned = tokio::runtime::Handle::current()
            .block_on(kkt_container::provision(
                self.parts.container,
                self.parts.clock.as_ref(),
                kkt_container::Settings::DEFAULT,
                self.parts.login_link,
                &kkt_container::Folder(folder.clone()),
                &inputs,
                &key_file,
            ))
            .map_err(|e| e.to_string())?;
        let url = docker_url(options.docker_port, provisioned.node.as_ref());
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
    }

    fn stage_key(&self, key: &str) -> Result<PathBuf, String> {
        let dir = kkt_config_file::config_dir(self.env.config_home().as_deref())
            .ok_or("set HOME to keep the key in ~/.config/kkt")?;
        self.parts.key_files.create_dir_all(&dir)?;
        let path = setup::staged_client_key(&dir);
        write_key_file(self.parts.key_files, &path, key, true)?;
        Ok(path)
    }

    fn keep_key(&self, staged: &Path, url: &str) -> Result<PathBuf, String> {
        let path = setup::client_key(staged, url);
        self.parts.key_files.keep_staged(staged, &path)?;
        Ok(path)
    }

    fn discard_key(&self, staged: &Path) -> Result<(), String> {
        self.parts
            .key_files
            .remove(staged)
            .map_err(|e| format!("could not remove {}: {e}", staged.display()))
    }

    fn detect_clients(&self) -> Vec<Client> {
        let dirs = self.parts.setup_files.path_dirs();
        let on_path = |program: &str| self.parts.setup_files.on_path(&dirs, program);
        Client::ALL
            .into_iter()
            .filter(|client| client.installed(self.env, &on_path, self.parts.clients))
            .collect()
    }

    fn allow_kkt(&self, client: Client) -> Result<String, String> {
        clients::allow_kkt(client, self.env, self.parts.clients)
    }

    fn doctor(&self, record: &setup::Record) -> Result<String, String> {
        // The server the clients use: on a machine whose clients were
        // switched, another machine's, checked as a client's.
        let server = record.for_clients();
        let mode = setup::clients_server_mode(record.clients.is_some(), &record.mode);
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
        if let Some(fix) = setup::server_fix(&mode) {
            point_server_fix_at(&mut checks, &server_fix(&fix));
        }
        match diagnostics::report(checks) {
            (report, true) => Ok(report.trim_end().to_string()),
            (report, false) => Err(report.trim_end().to_string()),
        }
    }

    fn unfinished(&self) -> Result<Option<setup::Unfinished>, String> {
        read_unfinished(
            self.env,
            self.parts.setup_files,
            self.parts.container,
            self.parts.service,
        )
    }

    fn save_unfinished(&self, unfinished: &setup::Unfinished) -> Result<(), String> {
        let config_dir = kkt_config_file::config_dir(self.env.config_home().as_deref())
            .ok_or("set HOME to note the install")?;
        self.parts
            .setup_files
            .write_unfinished(&config_dir, unfinished)
    }

    fn forget_unfinished(&self) -> Result<(), String> {
        let Some(config_dir) = kkt_config_file::config_dir(self.env.config_home().as_deref())
        else {
            return Ok(());
        };
        self.parts.setup_files.remove_unfinished(&config_dir)
    }

    fn add_admin_key(&self, record: &setup::Record, name: &AdminName) -> Result<PathBuf, String> {
        let access = record
            .key_file
            .as_deref()
            .ok_or("an admin key needs the server to have an access key")?;
        match &record.mode {
            setup::Mode::Docker(install) => tokio::runtime::Handle::current()
                .block_on(kkt_container::add_admin_key(
                    self.parts.container,
                    self.parts.clock.as_ref(),
                    kkt_container::Settings::DEFAULT,
                    &kkt_container::Folder(install.folder.clone()),
                    access,
                    name,
                ))
                .map_err(|e| e.to_string()),
            setup::Mode::Service { .. } => {
                let admin =
                    kkt_admission::admin_key_path(access).ok_or_else(admin_key_collision)?;
                // A key file that cannot be inspected is not replaced:
                // `new_key_file` refuses to write over one. Only a new key
                // is given the name.
                let made = Manager::for_this_os(
                    self.env.home.as_deref(),
                    self.env.config_home().as_deref(),
                )
                .and_then(|manager| {
                    manager.add_key(self.parts.service, &admin, &|path| {
                        let key = new_key_file(self.parts.key_files, path, false)?;
                        write_admin_name(self.parts.key_files, path, &key, Some(name)).map(drop)
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
    }

    fn check_lanes(&self, folder: &Path, purpose: setup::Purpose) -> Result<usize, String> {
        let max_lane_chars = setup::setup_lane_limit(
            purpose,
            || local_lane_limit(self.parts),
            || docker_lane_limit(self.parts, self.env, self.platform),
        )?;
        check_lanes_folder(self.parts.lanes, self.parts.output, folder, max_lane_chars)
    }

    fn import_lanes(&self, record: &setup::Record, folder: &Path) -> Result<String, String> {
        // `check_lanes` warned of the skipped files before setup began.
        let read = lane_files::read_folder(self.parts.lanes, folder)?;
        let report = match &record.mode {
            setup::Mode::Docker(install) => docker_import_lanes(
                self.parts.container,
                &kkt_container::Folder(install.folder.clone()),
                read,
                false,
            ),
            // The service runs on the config file and database `kkt serve`
            // finds by default.
            setup::Mode::Service { .. } => tokio::runtime::Handle::current().block_on(
                local_import_lanes(self.parts, None, None, read, false, None),
            ),
            setup::Mode::Client => Err("lanes are imported on the server's machine".into()),
        }?;
        Ok(report.trim_end().to_string())
    }
}

impl<C: ConnectionSource, D> setup::SwitchSteps for OsSteps<'_, '_, C, D> {
    fn connected_clients(&self) -> Result<Vec<Client>, String> {
        connected_clients(self.env, self.parts.clients)
    }

    fn recorded(&self) -> Result<Option<setup::Record>, String> {
        read_setup(self.env, self.parts.setup_files)
    }

    fn write_record(&self, record: &setup::Record) -> Result<(), String> {
        write_setup_record(self.env, self.parts.setup_files, record)
    }

    fn connect(&self, client: Client, record: &setup::Record) -> Result<String, String> {
        connect(
            self.env,
            self.parts.clients,
            self.parts.setup_files,
            client,
            record,
        )
    }

    fn check_address(&self, url: &str) -> Result<(), String> {
        check_address(self.parts.prober, url)
    }

    fn check_key(&self, url: &str, key: &setup::ClientKey) -> Result<(), String> {
        check_key(self.parts.prober, url, key)
    }
}

/// What `kkt switch` does to this machine. It installs nothing, so it holds
/// only what switching uses; it shares each step with [`OsSteps`].
pub(super) struct OsSwitchSteps<'a, 'p, C: ConnectionSource, D> {
    pub(super) env: &'a Env,
    pub(super) parts: &'a crate::OperateParts<'p, C, D>,
}

impl<C: ConnectionSource, D> setup::SwitchSteps for OsSwitchSteps<'_, '_, C, D> {
    fn connected_clients(&self) -> Result<Vec<Client>, String> {
        connected_clients(self.env, self.parts.clients)
    }

    fn recorded(&self) -> Result<Option<setup::Record>, String> {
        read_setup(self.env, self.parts.setup_files)
    }

    fn write_record(&self, record: &setup::Record) -> Result<(), String> {
        write_setup_record(self.env, self.parts.setup_files, record)
    }

    fn connect(&self, client: Client, record: &setup::Record) -> Result<String, String> {
        connect(
            self.env,
            self.parts.clients,
            self.parts.setup_files,
            client,
            record,
        )
    }

    fn check_address(&self, url: &str) -> Result<(), String> {
        check_address(self.parts.prober, url)
    }

    fn check_key(&self, url: &str, key: &setup::ClientKey) -> Result<(), String> {
        check_key(self.parts.prober, url, key)
    }
}

/// The clients whose config has a kkt entry now, whatever server it names.
fn connected_clients(env: &Env, host: &dyn clients::Host) -> Result<Vec<Client>, String> {
    let mut connected = Vec::new();
    for client in Client::ALL {
        if client.connected(env, host)? {
            connected.push(client);
        }
    }
    Ok(connected)
}

/// Points `client` at the server `record`'s clients use.
fn connect(
    env: &Env,
    host: &dyn clients::Host,
    program: &dyn crate::SetupFiles,
    client: Client,
    record: &setup::Record,
) -> Result<String, String> {
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
    clients::configure(client, &setup, true, host)
}

/// Checks, changing nothing, that a server answers at `url` and accepts its
/// host name.
fn check_address(prober: &dyn diagnostics::Prober, url: &str) -> Result<(), String> {
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
    prober: &dyn diagnostics::Prober,
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
    }?;
    only_problems(checks)
}

impl<C: ConnectionSource, D: crate::ServiceDefaults> setup::AdoptSteps for OsSteps<'_, '_, C, D> {
    fn unrecorded(&self) -> Result<Option<setup::Unrecorded>, String> {
        if read_setup(self.env, self.parts.setup_files)?.is_some()
            || read_unfinished(
                self.env,
                self.parts.setup_files,
                self.parts.container,
                self.parts.service,
            )?
            .is_some()
        {
            return Ok(None);
        }
        read_unrecorded(self.parts, self.env, self.platform).map_err(|(_, e)| e)
    }
}

/// The doctor's lines for the problems in `checks` as an error, if any.
fn only_problems(checks: Vec<diagnostics::Check>) -> Result<(), String> {
    let problems: Vec<_> = checks.into_iter().filter(|check| !check.ok).collect();
    if problems.is_empty() {
        return Ok(());
    }
    Err(diagnostics::report(problems).0.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docker(tailscale: bool) -> setup::Mode {
        setup::Mode::Docker(setup::DockerSetup {
            folder: PathBuf::from("/data/kkt docker"),
            tailscale,
            hostname: None,
            tailnet: None,
        })
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
