//! The doctor's lines about how this machine runs kkt, and the fixes that
//! depend on it, from facts the caller reads and hands over: how the server
//! the clients use is run ([`Setup`]), the kkt service ([`ServiceSeen`]),
//! kkt in Docker ([`DockerKkt`], [`TailscaleLogin`], [`WebKeyCopy`]) and a
//! server found installed with no record ([`UnrecordedServer`]).

use super::{Check, SETUP_CHECK, shell_word};
use std::path::{Path, PathBuf};

/// Why a fact the doctor reports on is not known: the reader's own error,
/// passed on unread, whose words the line gives. It can be cloned with the
/// facts that hold it.
#[derive(Clone, Debug, thiserror::Error)]
#[error(transparent)]
pub struct Unknown(pub std::sync::Arc<dyn std::error::Error + Send + Sync>);

impl Unknown {
    /// The fact is not known because of `error`.
    pub fn new(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Unknown(std::sync::Arc::from(error.into()))
    }
}

/// How the server this machine's clients use is run, as the doctor checks
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setup {
    /// As a service on this machine.
    Service,
    /// In Docker on this machine, from `folder`.
    Docker { folder: PathBuf },
    /// On another machine.
    Client,
}

/// The fix for a server that does not answer, run as `setup`; `None` where
/// starting the kkt service is how kkt runs.
pub fn server_fix(setup: &Setup) -> Option<String> {
    match setup {
        Setup::Client => Some(
            "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)"
                .to_string(),
        ),
        // From inside the folder: Compose reads a relative COMPOSE_FILE in
        // .env against the current folder.
        Setup::Docker { folder } => Some(format!(
            "this machine runs kkt in Docker: run `kkt status`, or start it with `cd {} && docker compose up -d`",
            shell_word(&folder.display().to_string())
        )),
        Setup::Service => None,
    }
}

/// The fix for an operator web key that does not work, on a machine run as
/// `setup` (`None` when not known).
pub fn web_key_fix(setup: Option<&Setup>) -> String {
    match setup {
        Some(Setup::Docker { folder }) => docker_web_key_fix(folder),
        _ => "check the operator web key on the server; `kkt key create --web` creates it if missing, then restart the server".to_string(),
    }
}

/// The fix for an admin key that does not work, on a machine run as `setup`
/// (`None` when not known).
pub fn admin_key_fix(setup: Option<&Setup>) -> String {
    match setup {
        Some(Setup::Docker { folder }) => docker_admin_key_fix(folder),
        _ => "check the admin key on the server; `kkt key create --admin --force` replaces it, then restart the server".to_string(),
    }
}

/// How to record a server found installed with no record.
const ADOPT_FIX: &str = "run `kkt setup` to record it (`kkt setup --adopt` with no terminal), so `kkt upgrade` manages it";

/// A server kkt installed here with no record, or why looking for one
/// failed.
#[derive(Clone, Debug)]
pub enum UnrecordedServer {
    /// One was found. `summary` is one sentence saying what was found.
    Found { in_docker: bool, summary: String },
    /// Looking for one failed, for the `reason` given.
    Unreadable { in_docker: bool, reason: Unknown },
}

/// The doctor's line about a server kkt installed here with no record, if
/// one was looked for (`None` when none was found). kkt in Docker
/// unrecorded is a problem: only setup makes the managed folder, and
/// `kkt upgrade` needs the record. A service may have been installed with
/// `kkt service install`, which never needed a record, so for one it is
/// only said.
pub fn unrecorded_check(unrecorded: Option<UnrecordedServer>) -> Option<Check> {
    Some(match unrecorded? {
        UnrecordedServer::Found {
            in_docker: true,
            summary,
        } => Check::problem(SETUP_CHECK, summary, ADOPT_FIX),
        UnrecordedServer::Found {
            in_docker: false,
            summary,
        } => Check::ok(
            SETUP_CHECK,
            format!("{summary} To have `kkt upgrade` manage it, run `kkt setup` to record it."),
        ),
        UnrecordedServer::Unreadable {
            in_docker: true,
            reason,
        } => Check::problem(
            SETUP_CHECK,
            reason.to_string(),
            "run `kkt setup`, which sets it up again",
        ),
        UnrecordedServer::Unreadable {
            in_docker: false,
            reason,
        } => Check::ok(SETUP_CHECK, reason.to_string()),
    })
}

/// The name of the doctor line about the kkt service on this machine.
const SERVICE_CHECK: &str = "this machine's kkt service";

/// What the service manager says of the kkt service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceSeen {
    pub installed: bool,
    pub running: bool,
}

/// The doctor's line about the kkt service on this machine, or why it could
/// not be asked.
pub fn service_check(state: Result<ServiceSeen, Unknown>) -> Check {
    match state {
        Ok(ServiceSeen {
            installed: false, ..
        }) => Check::ok(
            SERVICE_CHECK,
            "not installed; optional when running `kkt serve` in a terminal",
        ),
        Ok(ServiceSeen { running: true, .. }) => Check::ok(SERVICE_CHECK, "installed and running"),
        Ok(_) => Check::problem(
            SERVICE_CHECK,
            "installed but not running",
            "run `kkt service start`",
        ),
        Err(error) => Check::problem(
            SERVICE_CHECK,
            error.to_string(),
            "check `kkt service status` and your user service manager",
        ),
    }
}

/// The name of the doctor line about kkt in Docker on this machine.
const DOCKER_CHECK: &str = "this machine's kkt in Docker";

/// The name of the doctor line about the Tailscale login of kkt in Docker.
const DOCKER_TAILSCALE_CHECK: &str = "this machine's Tailscale login";

/// The name of the doctor line comparing the web key's host copy with the
/// key of kkt in Docker.
const DOCKER_WEB_KEY_CHECK: &str = "web key host copy";

/// How to save, or read, the web key of kkt in Docker in `folder`.
fn docker_web_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup` (run again with the same answers) or `kkt upgrade` (to a new version) saves the server's web key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --web`",
        shell_word(&folder.display().to_string())
    )
}

/// How to save, or read, the admin key of kkt in Docker in `folder`.
fn docker_admin_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup --admin-key` (run again with the same answers) saves the server's admin key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --admin`",
        shell_word(&folder.display().to_string())
    )
}

/// Whether the web key's host copy is the key kkt in Docker runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebKeyCopy {
    Same,
    /// The server's key changed after the copy was saved.
    Differs,
}

/// The doctor's line about whether the web key's host copy is the key of
/// kkt in Docker in `folder`, or why it could not be compared.
pub fn docker_web_key_check(compared: Result<WebKeyCopy, Unknown>, folder: &Path) -> Check {
    match compared {
        Ok(WebKeyCopy::Same) => Check::ok(DOCKER_WEB_KEY_CHECK, "the same as the server's"),
        Ok(WebKeyCopy::Differs) => Check::problem(
            DOCKER_WEB_KEY_CHECK,
            "differs from the server's web key, which was changed after the copy was saved; it no longer logs in",
            docker_web_key_fix(folder),
        ),
        Err(e) => Check::problem(
            DOCKER_WEB_KEY_CHECK,
            format!("could not compare it with the server's web key: {e}"),
            format!(
                "check kkt with `cd {} && docker compose logs kkt`",
                shell_word(&folder.display().to_string())
            ),
        ),
    }
}

/// The Tailscale login of kkt in Docker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TailscaleLogin {
    LoggedIn,
    /// Not logged in: its key expired, or it was logged out.
    NeedsLogin,
    /// The Tailscale container does not answer.
    Unreachable,
    /// Some other state, as Tailscale words it.
    Other(String),
}

/// The doctor's line about the login of the Tailscale container of kkt in
/// Docker in `folder`.
pub fn tailscale_login_check(login: &TailscaleLogin, folder: &Path) -> Check {
    match login {
        TailscaleLogin::LoggedIn => Check::ok(DOCKER_TAILSCALE_CHECK, "logged in"),
        TailscaleLogin::NeedsLogin => Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "not logged in: the machine's key expired, or it was logged out, so it is off the tailnet and its HTTPS address does not answer",
            "run `kkt setup` again and log in with the link it shows",
        ),
        TailscaleLogin::Unreachable => Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "the Tailscale container does not answer",
            format!(
                "start it with `cd {} && docker compose up -d`",
                shell_word(&folder.display().to_string())
            ),
        ),
        TailscaleLogin::Other(state) => Check::problem(
            DOCKER_TAILSCALE_CHECK,
            format!("Tailscale says: {state}"),
            "check the machine in the Tailscale admin console, or run `kkt setup` again",
        ),
    }
}

/// What was found of kkt in Docker in its managed folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerKkt {
    /// Whether the folder has its settings file.
    pub files_present: bool,
    /// Whether the kkt container is running.
    pub running: bool,
    /// The kkt version the settings file names.
    pub version: Option<String>,
}

/// The doctor's line about kkt in Docker in the managed `folder`, or why it
/// could not be asked.
pub fn docker_check(state: Result<DockerKkt, Unknown>, folder: &Path) -> Check {
    let shown = folder.display().to_string();
    match state {
        Ok(state) if !state.files_present => Check::problem(
            DOCKER_CHECK,
            format!("its files are missing from {shown}"),
            "run `kkt setup` again",
        ),
        Ok(state) if state.running => Check::ok(
            DOCKER_CHECK,
            format!(
                "running kkt {}",
                state.version.as_deref().unwrap_or("(version unknown)")
            ),
        ),
        Ok(_) => Check::problem(
            DOCKER_CHECK,
            "not running",
            format!(
                // From inside the folder: Compose reads a relative
                // COMPOSE_FILE in .env against the current folder.
                "start it with `cd {} && docker compose up -d`",
                shell_word(&shown)
            ),
        ),
        Err(error) => Check::problem(
            DOCKER_CHECK,
            error.to_string(),
            "check that Docker is running, then run `kkt status`",
        ),
    }
}

#[cfg(test)]
mod pins {
    //! Every line the doctor's checks and fixes can give, pinned from the
    //! code before it moved here from kkt-setup (`doctor_lines.txt` is the
    //! file the old code's test read, unchanged).
    use super::*;

    const FOLDER: &str = "/data/it's kkt";

    fn line(check: &Check) -> String {
        format!(
            "{} | {} | {} | {:?}\n",
            check.name, check.ok, check.detail, check.fix
        )
    }

    fn every_line() -> String {
        let mut out = String::new();
        let folder = Path::new(FOLDER);
        let docker = Setup::Docker {
            folder: PathBuf::from(FOLDER),
        };
        for setup in [
            None,
            Some(Setup::Service),
            Some(Setup::Client),
            Some(docker),
        ] {
            out += &format!("web key fix: {}\n", web_key_fix(setup.as_ref()));
            out += &format!("admin key fix: {}\n", admin_key_fix(setup.as_ref()));
        }
        for setup in [
            Setup::Client,
            Setup::Docker {
                folder: PathBuf::from(FOLDER),
            },
        ] {
            out += &format!(
                "server fix: {}\n",
                server_fix(&setup).expect("a fix off the service")
            );
        }
        let summary = |how: &str| {
            format!(
                "Found a kkt server {how}, for clients at http://127.0.0.1:9000/mcp, not recorded."
            )
        };
        for unrecorded in [
            None,
            Some(UnrecordedServer::Found {
                in_docker: true,
                summary: summary(&format!("in Docker from {FOLDER}")),
            }),
            Some(UnrecordedServer::Found {
                in_docker: false,
                summary: summary("as a service"),
            }),
            Some(UnrecordedServer::Unreadable {
                in_docker: true,
                reason: Unknown::new("could not read the Docker folder"),
            }),
            Some(UnrecordedServer::Unreadable {
                in_docker: false,
                reason: Unknown::new("could not ask the service"),
            }),
        ] {
            match unrecorded_check(unrecorded) {
                Some(check) => out += &line(&check),
                None => out += "unrecorded: none\n",
            }
        }
        let seen = |installed, running| Ok(ServiceSeen { installed, running });
        for state in [
            seen(false, false),
            seen(true, true),
            seen(true, false),
            Err(Unknown::new("no session")),
        ] {
            out += &line(&service_check(state));
        }
        for compared in [
            Ok(WebKeyCopy::Same),
            Ok(WebKeyCopy::Differs),
            Err(Unknown::new("docker is not running")),
        ] {
            out += &line(&docker_web_key_check(compared, folder));
        }
        for login in [
            TailscaleLogin::LoggedIn,
            TailscaleLogin::NeedsLogin,
            TailscaleLogin::Unreachable,
            TailscaleLogin::Other("Stopped".into()),
        ] {
            out += &line(&tailscale_login_check(&login, folder));
        }
        let state = |files_present, running, version: Option<&str>| {
            Ok(DockerKkt {
                files_present,
                running,
                version: version.map(str::to_string),
            })
        };
        for state in [
            state(false, false, None),
            state(true, true, Some("0.10.0")),
            state(true, true, None),
            state(true, false, Some("0.10.0")),
            Err(Unknown::new("Docker is not available")),
        ] {
            out += &line(&docker_check(state, folder));
        }
        out
    }

    #[test]
    fn every_doctor_line_is_pinned() {
        assert_eq!(every_line(), EXPECTED);
    }

    /// The lines, one per check or fix, as the old code gave them.
    const EXPECTED: &str = include_str!("doctor_lines.txt");

    #[test]
    fn a_service_has_no_server_fix_of_its_own() {
        assert_eq!(server_fix(&Setup::Service), None);
    }
}
