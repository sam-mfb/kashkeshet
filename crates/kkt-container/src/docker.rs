//! Docker implementation of kkt-container: the Compose files that come with this program, the folder
//! kkt manages for them, and the commands that set it up, start it, back it
//! up, import and export its lanes, upgrade it and remove it. All of it goes through the private runner, so it is
//! tested against a fake host.
//!
//! This part only runs Docker. It does not decide what a person wants (the
//! setup answers) or remember what was set up (the setup record); the caller
//! passes those in.
//!
//! `Inputs.project` names the Compose project and its containers and volumes.
//! `Inputs.allow_hosts` is written to `.env` as `KKT_ALLOW_HOSTS`;
//! `Inputs.sharing` starts the Tailscale sidecar and waits for its login.

mod compose;
mod files;
mod lanes;
mod lifecycle;
mod name_pack;
mod provision;
mod tailscale;
#[cfg(test)]
mod tests;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(feature = "real")]
use crate::Container;
use crate::{
    ADMIN_KEY_FILE_NAME as DOCKER_ADMIN_KEY_FILE_NAME, NO_ADMIN_KEY_EXIT,
    WEB_KEY_FILE_NAME as DOCKER_WEB_KEY_FILE_NAME,
};
use crate::{
    ConfigReader, Existing, Folder, Inputs, KeyCopy, LanesImport, Login, LoginLink, Provisioned,
    State,
};
#[cfg(feature = "real")]
use futures::future::LocalBoxFuture;
#[cfg(feature = "real")]
use kkt_clock::Clock;
#[cfg(feature = "real")]
use kkt_names::AdminName;
#[cfg(feature = "real")]
mod runner;
#[cfg(test)]
use crate::{ContainerConfig, Sharing as Tailscale};
#[cfg(test)]
use files::env_text;
use files::{ENV_FILE, docker_env, tailscale_auth_key, version_in_env};
use lanes::{export_lanes, import_lanes, lane_limit};
use lifecycle::{backup, compare_web_key, down, existing, status, up, upgrade};
#[cfg(feature = "real")]
use runner::OsHost;

use provision::{LinkAndBrowser, add_admin_key, provision};

/// The exit code of a command given an option it does not know, as a kkt
/// from before admin keys is given `kkt key show --admin`.
const UNKNOWN_OPTION_EXIT: i32 = 2;

/// Where the Compose file mounts the backups folder in the container.
const DOCKER_BACKUPS_MOUNT: &str = "/backups";
/// The environment variables Compose reads, which would take the place of
/// the managed folder's settings file if they were set where kkt runs, so
/// every Docker command runs without them.
const DOCKER_MANAGED_VARIABLES: &[&str] = &[
    "COMPOSE_FILE",
    "COMPOSE_PATH_SEPARATOR",
    "COMPOSE_PROFILES",
    "COMPOSE_PROJECT_NAME",
    "COMPOSE_ENV_FILES",
    "KKT_VERSION",
    "KKT_PORT",
    "TS_HOSTNAME",
    "TS_TAILNET",
    "TS_AUTHKEY",
];
/// What `TS_TAILNET` is set to until the Tailscale container has logged in
/// and the real name is known. The Compose file needs some value to start
/// the Tailscale container alone; kkt does not run with it.
const DOCKER_PENDING_TAILNET: &str = "pending.invalid";

/// How long setup and upgrade wait on the containers, and how often they
/// ask. The wrapper uses [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settings {
    /// How long to wait for someone to log the Tailscale container in.
    pub docker_login_wait: Duration,
    /// How long to wait for the kkt container to start and make its access
    /// key.
    pub docker_start_wait: Duration,
    /// How many checks to wait for a login link from a Tailscale container
    /// that needs a login before restarting it. A node whose key expired, or
    /// that was logged out, is not asking for a login, so it shows no link
    /// until it is restarted; a new one shows its link within a few seconds.
    pub docker_link_wait_polls: u32,
    /// How often to ask while waiting for the Tailscale login or for kkt.
    pub docker_poll_interval: Duration,
}

impl Settings {
    /// Ten minutes for the login and two for kkt to start, asking every two
    /// seconds; a Tailscale container with no login link after five checks
    /// is restarted.
    pub const DEFAULT: Settings = Settings {
        docker_login_wait: Duration::from_secs(10 * 60),
        docker_start_wait: Duration::from_secs(2 * 60),
        docker_link_wait_polls: 5,
        docker_poll_interval: Duration::from_secs(2),
    };
}

/// Private machine operations used by Docker logic and its tests.
trait Runner {
    /// Runs `command` and collects its output.
    fn run(&self, command: &Command) -> io::Result<Finished>;
    /// Runs `command` with its output going straight to the terminal, as for
    /// a download someone is waiting on. The returned output is empty.
    fn run_attached(&self, command: &Command) -> io::Result<Finished>;
    fn exists(&self, path: &Path) -> bool;
    /// Whether this path is a regular file, following links.
    fn is_file(&self, path: &Path) -> bool;
    fn read_file(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Writes `contents`, replacing any file there. With `private` only the
    /// owner may read it, whether or not the file existed.
    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()>;
    /// Tells the person one line.
    fn say(&self, line: &str);
    /// The folder the person ran kkt in, which paths they typed are relative
    /// to. Docker commands run elsewhere, in the managed folder.
    fn working_dir(&self) -> io::Result<PathBuf>;
}

/// A program to run and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Command {
    pub program: &'static str,
    pub args: Vec<String>,
    /// Environment variables the command must run without.
    pub unset: Vec<String>,
    /// The folder to run in. Compose finds the files `COMPOSE_FILE` names
    /// relative to where it runs, not to `--project-directory`, so Compose
    /// commands run in the managed folder.
    pub current_dir: Option<PathBuf>,
}

impl Command {
    /// A `docker` command that does not see the variables Compose would let
    /// override the managed folder's settings file.
    fn docker(args: Vec<String>) -> Command {
        Command {
            program: "docker",
            args,
            unset: DOCKER_MANAGED_VARIABLES
                .iter()
                .map(|v| v.to_string())
                .collect(),
            current_dir: None,
        }
    }
}

/// Shown the way a person would type it, for messages.
impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '"') {
                write!(f, " {arg:?}")?;
            } else {
                write!(f, " {arg}")?;
            }
        }
        Ok(())
    }
}

/// How a command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Finished {
    /// The exit status, or `None` if a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Finished {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// What the command said when it failed, for a message.
    fn detail(&self) -> String {
        let said = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        match self.code {
            Some(code) => format!("exit {code}: {}", said.trim()),
            None => format!("ended by a signal: {}", said.trim()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum DockerError {
    #[error("Docker is not available: {0}")]
    NotAvailable(String),
    #[error("`{command}` failed ({detail})")]
    Command { command: String, detail: String },
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "Tailscale is not logged in yet. Run setup again to continue; the containers are left as they are"
    )]
    LoginTimeout,
    #[error(
        "HTTPS certificates are not enabled for your tailnet. Turn them on at https://login.tailscale.com/admin/dns (\"Enable HTTPS\"), then run setup again"
    )]
    HttpsDisabled,
    #[error(
        "kkt did not start or did not make its keys in time; `docker compose logs kkt` shows why"
    )]
    StartTimeout,
    #[error("unexpected output from {what}: {detail}")]
    Unexpected { what: &'static str, detail: String },
    /// The managed folder is there but lacks something setup writes, or has
    /// a setting setup does not write, said as "its ...".
    #[error("{0}")]
    Incomplete(String),
}

impl DockerError {
    fn io(what: impl Into<String>, source: io::Error) -> DockerError {
        DockerError::Io {
            what: what.into(),
            source,
        }
    }
}

/// Docker and Compose implement the container contract directly. A
/// sharing setup's login link opens in `browser` when the person asks.
#[cfg(feature = "real")]
pub struct Docker {
    browser: Box<dyn kkt_browser::Browser>,
}

#[cfg(feature = "real")]
impl Docker {
    /// Docker and Compose, opening a login link in `browser`.
    pub fn new(browser: Box<dyn kkt_browser::Browser>) -> Docker {
        Docker { browser }
    }
}

#[cfg(feature = "real")]
impl Container for Docker {
    fn provision<'a>(
        &'a self,
        clock: &'a dyn Clock,
        login: &'a dyn LoginLink,
        folder: &'a Folder,
        inputs: &'a Inputs,
        key_file: &'a Path,
    ) -> LocalBoxFuture<'a, Result<Provisioned, String>> {
        Box::pin(async move {
            provision(
                &OsHost,
                clock,
                Settings::DEFAULT,
                LinkAndBrowser {
                    link: login,
                    browser: &*self.browser,
                },
                folder,
                inputs,
                key_file,
            )
            .await
            .map_err(|e| e.to_string())
        })
    }
    fn add_admin_key<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        key_file: &'a Path,
        name: &'a AdminName,
    ) -> LocalBoxFuture<'a, Result<PathBuf, String>> {
        Box::pin(async move {
            add_admin_key(&OsHost, clock, Settings::DEFAULT, folder, key_file, name)
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn status(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default_name_pack: &str,
    ) -> Result<State, String> {
        status(&OsHost, folder, reader, default_name_pack).map_err(|e| e.to_string())
    }
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, String> {
        existing(&OsHost, folder).map_err(|e| e.to_string())
    }
    fn compare_web_key(&self, folder: &Folder, web_key_file: &Path) -> Result<KeyCopy, String> {
        compare_web_key(&OsHost, folder, web_key_file).map_err(|e| e.to_string())
    }
    fn up(&self, folder: &Folder) -> Result<(), String> {
        up(&OsHost, folder).map_err(|e| e.to_string())
    }
    fn down(&self, folder: &Folder, remove_data: bool) -> Result<(), String> {
        down(&OsHost, folder, remove_data).map_err(|e| e.to_string())
    }
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, String> {
        backup(&OsHost, folder, dest).map_err(|e| e.to_string())
    }
    fn upgrade<'a>(
        &'a self,
        clock: &'a dyn Clock,
        folder: &'a Folder,
        new_version: &'a str,
        key_file: Option<&'a Path>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            upgrade(
                &OsHost,
                clock,
                Settings::DEFAULT,
                folder,
                new_version,
                key_file,
            )
            .await
            .map_err(|e| e.to_string())
        })
    }
    fn import_lanes(&self, folder: &Folder, import: &LanesImport) -> Result<String, String> {
        import_lanes(&OsHost, folder, import).map_err(|e| e.to_string())
    }
    fn export_lanes(&self, folder: &Folder, dest: &Path) -> Result<String, String> {
        export_lanes(&OsHost, folder, dest).map_err(|e| e.to_string())
    }
    fn lane_limit(
        &self,
        folder: &Folder,
        reader: &dyn ConfigReader,
        default: usize,
    ) -> Result<usize, String> {
        lane_limit(&OsHost, folder, reader, default).map_err(|e| e.to_string())
    }
    fn has_settings(&self, folder: &Folder) -> bool {
        has_settings(&OsHost, folder)
    }
    fn settings_file(&self, folder: &Folder) -> bool {
        settings_file(&OsHost, folder)
    }
    fn version(&self, folder: &Folder) -> Result<String, String> {
        installed_version(&OsHost, folder)
    }
    fn sharing_key(&self, path: &Path) -> Result<String, String> {
        tailscale_auth_key(&OsHost, path)
    }
}

fn has_settings(runner: &dyn Runner, folder: &Folder) -> bool {
    runner.exists(&folder.path().join(ENV_FILE))
}
fn settings_file(runner: &dyn Runner, folder: &Folder) -> bool {
    runner.is_file(&folder.path().join(ENV_FILE))
}
fn installed_version(runner: &dyn Runner, folder: &Folder) -> Result<String, String> {
    let path = folder.path().join(ENV_FILE);
    let text = docker_env(runner, folder.path())?;
    version_in_env(&text).ok_or_else(|| format!("{} has no KKT_VERSION", path.display()))
}
