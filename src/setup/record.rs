//! The setup record: TOML, written by `kkt setup` once the machine is set up.
//!
//! ```toml
//! format = 1
//! mode = "docker"
//! written_by = "0.4.0"
//! url = "https://kkt.tail1234.ts.net/mcp"
//! key_file = "/home/sam/.config/kkt/docker-access-key"
//!
//! [docker]
//! folder = "/home/sam/.local/share/kashkeshet/docker"
//! tailscale = true
//! hostname = "kkt"
//! tailnet = "tail1234.ts.net"
//! ```
//!
//! `mode` is `service`, `docker` or `client`. Paths are absolute. A
//! `[service]` table holds `tailscale` for the service mode; the client mode
//! has no table.

use super::flow::Purpose;
use crate::clients::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The only format this version reads and writes.
const FORMAT: u32 = 1;

/// What a machine was set up as, and how clients reach its server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub mode: Mode,
    /// The kkt version that wrote the record.
    pub written_by: String,
    /// The MCP endpoint clients on this machine use.
    pub url: String,
    /// The access key clients on this machine send, if the server has one.
    pub key_file: Option<PathBuf>,
}

/// What the machine is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The server runs here as a launchd or systemd service.
    Service {
        /// Shared over the host's Tailscale (`tailscale serve`).
        tailscale: bool,
    },
    /// The server runs here in Docker, from Compose files kkt manages.
    Docker(DockerSetup),
    /// The server is elsewhere; only clients are set up here.
    Client,
}

/// The Docker install kkt manages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerSetup {
    /// The folder holding the Compose files, `.env` and `backups`.
    pub folder: PathBuf,
    /// Shared over a Tailscale container.
    pub tailscale: bool,
    /// The node's name on the tailnet, once known, such as `kkt` or `kkt-1`.
    pub hostname: Option<String>,
    /// The tailnet's DNS name, once known, such as `tail1234.ts.net`.
    pub tailnet: Option<String>,
}

impl Mode {
    /// The name used for the mode in the record and in messages.
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Service { .. } => "service",
            Mode::Docker(_) => "docker",
            Mode::Client => "client",
        }
    }
}

/// The record as it is in the file.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: u32,
    mode: String,
    written_by: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service: Option<ServiceTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    docker: Option<DockerTable>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceTable {
    tailscale: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DockerTable {
    folder: PathBuf,
    tailscale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tailnet: Option<String>,
}

impl Record {
    /// The record in `text`, read from the file at `path`. Errors name the
    /// file and what is wrong with it.
    pub fn parse(text: &str, path: &Path) -> Result<Record, String> {
        let bad = |what: String| format!("the setup record {} {what}", path.display());
        let file: File = toml::from_str(text)
            .map_err(|e: toml::de::Error| bad(format!("is not valid: {}", e.message().trim())))?;
        Record::from_file(file, &bad)
    }

    /// The record `file` holds; `bad` names the file in an error.
    fn from_file(file: File, bad: &dyn Fn(String) -> String) -> Result<Record, String> {
        if file.format != FORMAT {
            return Err(bad(format!(
                "has format {}, but this kkt reads format {FORMAT}; run `kkt setup` again",
                file.format
            )));
        }
        let mode = match (file.mode.as_str(), file.service, file.docker) {
            ("service", Some(service), None) => Mode::Service {
                tailscale: service.tailscale,
            },
            ("docker", None, Some(docker)) => Mode::Docker(DockerSetup {
                folder: docker.folder,
                tailscale: docker.tailscale,
                hostname: docker.hostname,
                tailnet: docker.tailnet,
            }),
            ("client", None, None) => Mode::Client,
            (mode @ ("service" | "docker" | "client"), _, _) => {
                let tables = match mode {
                    "service" => "needs a [service] table and no [docker] table",
                    "docker" => "needs a [docker] table and no [service] table",
                    _ => "has no [service] or [docker] table",
                };
                return Err(bad(format!("has mode {mode:?}, which {tables}")));
            }
            (other, _, _) => {
                return Err(bad(format!(
                    "has an unknown mode {other:?}: expected service, docker or client"
                )));
            }
        };
        let mut paths: Vec<&Path> = file.key_file.iter().map(PathBuf::as_path).collect();
        if let Mode::Docker(docker) = &mode {
            paths.push(&docker.folder);
        }
        if let Some(relative) = paths.iter().find(|p| !p.is_absolute()) {
            return Err(bad(format!("has a relative path {}", relative.display())));
        }
        Ok(Record {
            mode,
            written_by: file.written_by,
            url: file.url,
            key_file: file.key_file,
        })
    }

    /// The record as file text, which [`Record::parse`] reads back.
    pub fn render(&self) -> Result<String, String> {
        toml::to_string(&self.to_file())
            .map_err(|e| format!("could not write the setup record: {e}"))
    }

    fn to_file(&self) -> File {
        let (service, docker) = match &self.mode {
            Mode::Service { tailscale } => (
                Some(ServiceTable {
                    tailscale: *tailscale,
                }),
                None,
            ),
            Mode::Docker(d) => (
                None,
                Some(DockerTable {
                    folder: d.folder.clone(),
                    tailscale: d.tailscale,
                    hostname: d.hostname.clone(),
                    tailnet: d.tailnet.clone(),
                }),
            ),
            Mode::Client => (None, None),
        };
        File {
            format: FORMAT,
            mode: self.mode.name().to_string(),
            written_by: self.written_by.clone(),
            url: self.url.clone(),
            key_file: self.key_file.clone(),
            service,
            docker,
        }
    }
}

/// A server `kkt setup` began to install on this machine but did not
/// record, because a step failed or setup was stopped. It is noted before
/// the install starts, so an install that fails partway is found too. It is
/// kept in its own file, `setup-unfinished.toml`, so an earlier record stays
/// as it was:
///
/// ```toml
/// mode = "docker"
/// tailscale = true
/// allow_hosts = ["host.docker.internal"]
/// clients = ["codex"]
/// # For Docker over Tailscale: the name asked for, and the file holding the
/// # auth key, if one was given (never the key itself).
/// ts_hostname = "kkt"
/// ts_authkey_file = "/home/sam/tailscale-key"
///
/// # Once the install has finished:
/// [record]
/// format = 1
/// mode = "docker"
/// # ... the rest of the record setup would have written
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unfinished {
    /// How the server is installed: [`Purpose::Docker`] or
    /// [`Purpose::Service`].
    pub purpose: Purpose,
    /// Whether the server is shared over Tailscale.
    pub tailscale: bool,
    /// The host names the server is installed to answer to.
    pub allow_hosts: Vec<String>,
    /// The clients chosen to connect to it.
    pub clients: Vec<Client>,
    /// How a Docker install joins the tailnet; `None` unless it is in Docker
    /// and shared over Tailscale.
    pub tailscale_login: Option<TailscaleLogin>,
    /// The record setup writes once the remaining steps work; `None` until
    /// the install has finished.
    pub record: Option<Record>,
}

/// How kkt's Tailscale container in Docker joins the tailnet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TailscaleLogin {
    /// The machine name to ask for, such as `kkt`.
    pub hostname: String,
    /// An absolute path to a file holding an auth key, to log in with
    /// instead of a link. Setup reads it when it installs.
    pub auth_key_file: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnfinishedFile {
    mode: String,
    tailscale: bool,
    allow_hosts: Vec<String>,
    clients: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts_hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts_authkey_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<File>,
}

impl Unfinished {
    /// What is in `text`, read from the file at `path`.
    pub fn parse(text: &str, path: &Path) -> Result<Unfinished, String> {
        let bad = |what: String| format!("the unfinished setup {} {what}", path.display());
        let file: UnfinishedFile = toml::from_str(text)
            .map_err(|e: toml::de::Error| bad(format!("is not valid: {}", e.message().trim())))?;
        let purpose = match Purpose::from_name(&file.mode) {
            Some(purpose @ (Purpose::Docker | Purpose::Service)) => purpose,
            _ => {
                return Err(bad(format!(
                    "has mode {:?}: expected docker or service",
                    file.mode
                )));
            }
        };
        let clients = file
            .clients
            .iter()
            .map(|name| {
                Client::from_name(name)
                    .ok_or_else(|| bad(format!("has an unknown client {name:?}")))
            })
            .collect::<Result<_, _>>()?;
        let tailscale_login = match (purpose, file.tailscale, file.ts_hostname) {
            (Purpose::Docker, true, Some(hostname)) => Some(TailscaleLogin {
                hostname,
                auth_key_file: file.ts_authkey_file,
            }),
            (Purpose::Docker, true, None) => {
                return Err(bad(
                    "is for Docker over Tailscale but has no ts_hostname".into()
                ));
            }
            (_, _, Some(_)) => {
                return Err(bad(
                    "has ts_hostname, which is only for Docker over Tailscale".into(),
                ));
            }
            (_, _, None) if file.ts_authkey_file.is_some() => {
                return Err(bad("has ts_authkey_file but no ts_hostname".into()));
            }
            (_, _, None) => None,
        };
        if let Some(relative) = tailscale_login
            .as_ref()
            .and_then(|login| login.auth_key_file.as_ref())
            .filter(|path| !path.is_absolute())
        {
            return Err(bad(format!("has a relative path {}", relative.display())));
        }
        let record = match file.record {
            Some(record) => Some(Record::from_file(record, &bad)?),
            None => None,
        };
        Ok(Unfinished {
            purpose,
            tailscale: file.tailscale,
            allow_hosts: file.allow_hosts,
            clients,
            tailscale_login,
            record,
        })
    }

    /// As file text, which [`Unfinished::parse`] reads back.
    pub fn render(&self) -> Result<String, String> {
        let login = self.tailscale_login.as_ref();
        let file = UnfinishedFile {
            mode: self.purpose.name().to_string(),
            tailscale: self.tailscale,
            allow_hosts: self.allow_hosts.clone(),
            clients: self.clients.iter().map(|c| c.name().to_string()).collect(),
            ts_hostname: login.map(|login| login.hostname.clone()),
            ts_authkey_file: login.and_then(|login| login.auth_key_file.clone()),
            record: self.record.as_ref().map(Record::to_file),
        };
        toml::to_string(&file).map_err(|e| format!("could not write the unfinished setup: {e}"))
    }

    /// One sentence saying what was installed, or began to be, and that it
    /// is not recorded, for setup, `kkt status` and `kkt doctor`.
    pub fn summary(&self) -> String {
        match &self.record {
            Some(record) => {
                let how = match &record.mode {
                    Mode::Docker(docker) => format!("in Docker, from {}", docker.folder.display()),
                    Mode::Service { .. } => "as a service".to_string(),
                    Mode::Client => "as a client".to_string(),
                };
                format!(
                    "An earlier `kkt setup` installed a server here {how}, for clients at {}, \
                     but stopped before recording it.",
                    record.url
                )
            }
            None => {
                let how = match self.purpose {
                    Purpose::Docker => "in Docker",
                    _ => "as a service",
                };
                format!(
                    "An earlier `kkt setup` began to install a server here {how}, but stopped \
                     before the install finished; parts of it may be running."
                )
            }
        }
    }
}
