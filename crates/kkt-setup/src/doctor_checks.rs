//! The doctor's checks and fixes, from facts read by the program.
use std::path::Path;
const SERVICE_CHECK: &str = "this machine's kkt service";
const SETUP_CHECK: &str = "this machine's setup";

/// Setup's check result, adapted by the program to the diagnostic report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}
impl DoctorCheck {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }
    fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

// One shell word in advice copied by the operator; never executed here.
fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn web_key_fix(mode: Option<&crate::Mode>) -> String {
    match crate::key_fix(mode) {
        crate::KeyFix::Docker(folder) => docker_web_key_fix(&folder),
        crate::KeyFix::OnServer => "check the operator web key on the server; `kkt key create --web` creates it if missing, then restart the server".to_string(),
    }
}
pub fn admin_key_fix(mode: Option<&crate::Mode>) -> String {
    match crate::key_fix(mode) {
        crate::KeyFix::Docker(folder) => docker_admin_key_fix(&folder),
        crate::KeyFix::OnServer => "check the admin key on the server; `kkt key create --admin --force` replaces it, then restart the server".to_string(),
    }
}

/// How to record a server found installed with no record.
const ADOPT_FIX: &str = "run `kkt setup` to record it (`kkt setup --adopt` with no terminal), so `kkt upgrade` manages it";

/// The doctor's line about a server kkt installed here with no record, if
/// one was looked for and found. kkt in Docker unrecorded is a problem:
/// only setup makes the managed folder, and `kkt upgrade` needs the record.
/// A service may have been installed with `kkt service install`, which
/// never needed a record, so for one it is only said.
pub fn unrecorded_check(
    unrecorded: Result<Option<crate::Unrecorded>, (crate::Purpose, String)>,
) -> Option<DoctorCheck> {
    Some(match unrecorded {
        Ok(None) => return None,
        Ok(Some(found)) => match found.mode {
            crate::Mode::Docker(_) => DoctorCheck::problem(SETUP_CHECK, found.summary(), ADOPT_FIX),
            _ => DoctorCheck::ok(
                SETUP_CHECK,
                format!(
                    "{} To have `kkt upgrade` manage it, run `kkt setup` to record it.",
                    found.summary()
                ),
            ),
        },
        Err((crate::Purpose::Docker, e)) => {
            DoctorCheck::problem(SETUP_CHECK, e, "run `kkt setup`, which sets it up again")
        }
        Err((_, e)) => DoctorCheck::ok(SETUP_CHECK, e),
    })
}

/// The text of `fix`, kkt-setup's fix for a server that does not answer.
pub fn server_fix_text(fix: &crate::ServerFix) -> String {
    match fix {
        crate::ServerFix::Elsewhere => "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)"
            .to_string(),
        // From inside the folder: Compose reads a relative COMPOSE_FILE in
        // .env against the current folder.
        crate::ServerFix::Docker(folder) => format!(
            "this machine runs kkt in Docker: run `kkt status`, or start it with `cd {} && docker compose up -d`",
            shell_word(&folder.display().to_string())
        ),
    }
}

/// The doctor's line about the kkt service on this machine.
pub fn service_check(state: Result<(bool, bool), String>) -> DoctorCheck {
    match state {
        Ok((false, _)) => DoctorCheck::ok(
            SERVICE_CHECK,
            "not installed; optional when running `kkt serve` in a terminal",
        ),
        Ok((_, true)) => DoctorCheck::ok(SERVICE_CHECK, "installed and running"),
        Ok(_) => DoctorCheck::problem(
            SERVICE_CHECK,
            "installed but not running",
            "run `kkt service start`",
        ),
        Err(error) => DoctorCheck::problem(
            SERVICE_CHECK,
            error,
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

/// The doctor's line about whether the web key's host copy is the Docker
/// server's key, from what `kkt_container::Container::compare_web_key` found.
pub fn docker_web_key_check(
    compared: Result<kkt_container::KeyCopy, String>,
    folder: &kkt_container::Folder,
) -> DoctorCheck {
    match compared {
        Ok(kkt_container::KeyCopy::Same) => {
            DoctorCheck::ok(DOCKER_WEB_KEY_CHECK, "the same as the server's")
        }
        Ok(kkt_container::KeyCopy::Differs) => DoctorCheck::problem(
            DOCKER_WEB_KEY_CHECK,
            "differs from the server's web key, which was changed after the copy was saved; it no longer logs in",
            docker_web_key_fix(folder.path()),
        ),
        Err(e) => DoctorCheck::problem(
            DOCKER_WEB_KEY_CHECK,
            format!("could not compare it with the server's web key: {e}"),
            format!(
                "check kkt with `cd {} && docker compose logs kkt`",
                shell_word(&folder.path().display().to_string())
            ),
        ),
    }
}

/// The doctor's line about the Tailscale container's login.
pub fn tailscale_login_check(
    login: &kkt_container::Login,
    folder: &kkt_container::Folder,
) -> DoctorCheck {
    match login {
        kkt_container::Login::LoggedIn => DoctorCheck::ok(DOCKER_TAILSCALE_CHECK, "logged in"),
        kkt_container::Login::NeedsLogin => DoctorCheck::problem(
            DOCKER_TAILSCALE_CHECK,
            "not logged in: the machine's key expired, or it was logged out, so it is off the tailnet and its HTTPS address does not answer",
            "run `kkt setup` again and log in with the link it shows",
        ),
        kkt_container::Login::Unreachable => DoctorCheck::problem(
            DOCKER_TAILSCALE_CHECK,
            "the Tailscale container does not answer",
            format!(
                "start it with `cd {} && docker compose up -d`",
                shell_word(&folder.path().display().to_string())
            ),
        ),
        kkt_container::Login::Other(state) => DoctorCheck::problem(
            DOCKER_TAILSCALE_CHECK,
            format!("Tailscale says: {state}"),
            "check the machine in the Tailscale admin console, or run `kkt setup` again",
        ),
    }
}

/// The doctor's line about kkt in Docker, from what `kkt_container::Container::status` found
/// in the managed `folder`.
pub fn docker_check(
    state: Result<kkt_container::State, String>,
    folder: &kkt_container::Folder,
) -> DoctorCheck {
    let shown = folder.path().display().to_string();
    match state {
        Ok(state) if !state.files_present => DoctorCheck::problem(
            DOCKER_CHECK,
            format!("its files are missing from {shown}"),
            "run `kkt setup` again",
        ),
        Ok(state) if state.running => DoctorCheck::ok(
            DOCKER_CHECK,
            format!(
                "running kkt {}",
                state.version.as_deref().unwrap_or("(version unknown)")
            ),
        ),
        Ok(_) => DoctorCheck::problem(
            DOCKER_CHECK,
            "not running",
            format!(
                // From inside the folder: Compose reads a relative
                // COMPOSE_FILE in .env against the current folder.
                "start it with `cd {} && docker compose up -d`",
                shell_word(&shown)
            ),
        ),
        Err(error) => DoctorCheck::problem(
            DOCKER_CHECK,
            error.to_string(),
            "check that Docker is running, then run `kkt status`",
        ),
    }
}
