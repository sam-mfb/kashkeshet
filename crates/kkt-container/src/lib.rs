//! kkt in Docker: the Compose files that come with this program, the folder
//! kkt manages for them, and the commands that set it up, start it, back it
//! up, import and export its lanes, upgrade it and remove it. All of it goes through [`Host`], so it is
//! tested against a fake host.
//!
//! This part only runs Docker. It does not decide what a person wants (the
//! setup answers) or remember what was set up (the setup record); the caller
//! passes those in.

mod compose;
mod config;
mod files;
mod lanes;
mod lifecycle;
mod login;
mod name_pack;
mod provision;
mod tailscale;
#[cfg(test)]
mod tests;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use config::{ConfigReader, ContainerConfig};
pub use files::{ENV_FILE, docker_env, env_text, tailscale_auth_key, version_in_env};
pub use lanes::{LanesImport, export_lanes, import_lanes, lane_limit};
pub use lifecycle::{
    Existing, KeyCopy, Login, State, backup, compare_web_key, down, existing, status, up, upgrade,
};
pub use login::LoginLink;
pub use provision::{Provisioned, add_admin_key, provision};

/// Docker setup's host copy of the operator key.
pub const DOCKER_WEB_KEY_FILE_NAME: &str = "docker-web-key";
/// Docker setup's host copy of the admin key.
pub const DOCKER_ADMIN_KEY_FILE_NAME: &str = "docker-admin-key";
/// The exit code of `kkt key show --admin` when there is no admin key.
pub const NO_ADMIN_KEY_EXIT: u8 = 4;
/// The exit code of a command given an option it does not know, as a kkt
/// from before admin keys is given `kkt key show --admin`.
const UNKNOWN_OPTION_EXIT: i32 = 2;
/// The folder inside the managed Docker folder that is mounted at
/// [`DOCKER_BACKUPS_MOUNT`] in the container.
const DOCKER_BACKUPS_DIR: &str = "backups";
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
/// ask. The program passes [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
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

/// What Docker setup needs from the host; the program adapts its real host.
pub trait Host {
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
pub struct Command {
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
pub struct Finished {
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

/// The folder kkt manages for the Compose files, the settings file and the
/// backups folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder(pub PathBuf);

impl Folder {
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The backups folder on the host, mounted in the container.
    pub fn backups(&self) -> PathBuf {
        self.0.join(DOCKER_BACKUPS_DIR)
    }
}

/// What to set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// The kkt version whose image to run, such as `0.4.0`.
    pub version: String,
    /// The Compose project name, which names the containers and volumes.
    /// `kkt` (the program's `DOCKER_PROJECT`) unless testing beside a real
    /// setup.
    pub project: String,
    /// The port on the host's `127.0.0.1` that kkt is published on.
    /// The port of kkt's default bind address, 7420, unless testing beside
    /// a real setup.
    pub port: u16,
    /// Host names kkt answers to besides its own, such as
    /// `host.docker.internal`. Written to `.env` as `KKT_ALLOW_HOSTS`.
    pub allow_hosts: Vec<String>,
    /// Serve over Tailscale, if given.
    pub tailscale: Option<Tailscale>,
}

/// Tailscale settings for [`Inputs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tailscale {
    /// The name asked for on the tailnet.
    pub hostname: String,
    /// An auth key, if there is one. Without it the person logs in with a
    /// link, which setup shows.
    pub auth_key: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
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
