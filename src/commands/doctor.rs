//! `kkt doctor`: checks the server this machine's clients use, its keys,
//! and how this machine runs kkt, and prints a line for each with a fix for
//! each problem.

use crate::machine::client_config::ClientHost;
use crate::machine::connection::{ConnectionOptions, client_connection, recorded_connection};
use crate::machine::keys::{KeyKind, local_operator_key, local_web_key};
use crate::machine::record::{read_setup, read_unfinished, read_unrecorded, unfinished_mode};
use crate::machine::service::ServiceSystem;
use crate::machine::settings::ServeArgs;
use kkt_client_config as clients;
use kkt_diagnostics::{self as diagnostics, OsProber, SETUP_CHECK};
use kkt_provenance::Build;
use kkt_service::Manager;
use kkt_settings::Env;
use kkt_setup as setup;
use std::path::Path;

/// The name of the doctor line about the service on this machine.
const SERVICE_CHECK: &str = "this machine's kkt service";

pub async fn run(env: &Env, serve_args: ServeArgs, flags: ConnectionOptions) -> Result<(), String> {
    // With no record, a server setup installed but did not record stands in
    // for it; with no note of one either, a server kkt installed here anyway.
    let unfinished = read_unfinished(env);
    let recorded = read_setup(env)?;
    let unrecorded = match setup::status_looks_for_unrecorded(recorded.as_ref(), &unfinished) {
        true => read_unrecorded(env, serve_args),
        false => Ok(None),
    };
    let found = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let record = setup::doctor_record(recorded, &unfinished, found, Build::CURRENT.version);
    let connection = match &record {
        Some(record) if flags.config.is_none() => recorded_connection(record.for_clients(), flags)?,
        _ => client_connection(env, flags, false)?,
    };
    let inspector = diagnostics::Inspector::new(&OsProber);
    let server = diagnostics::ServerLines::inspect(
        inspector,
        &connection.url,
        connection.key_file.as_deref(),
    )
    .await?;
    // A server machine whose clients were switched checks the other server
    // above, and so gets a client's fix.
    let switched = record.as_ref().is_some_and(|r| r.clients.is_some());
    let mode = match (record, &unfinished) {
        (Some(record), _) => Some(record.mode),
        (None, Ok(Some(unfinished))) => Some(unfinished_mode(env, unfinished)?),
        (None, _) => None,
    };
    let server_fix = setup::clients_server_fix(switched, mode.as_ref()).map(|fix| server_fix(&fix));
    let web_key = local_web_key(
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
    )?;
    let web_key_to_check = web_key.as_deref().map(|path| diagnostics::KeyToCheck {
        path,
        fix: match setup::key_fix(mode.as_ref()) {
            setup::KeyFix::Docker(folder) => docker_web_key_fix(&folder),
            setup::KeyFix::OnServer => "check the operator web key on the server; `kkt key create --web` creates it if missing, then restart the server".to_string(),
        },
    });
    // Most servers have no admin key; only one that exists is checked.
    let admin_key = local_operator_key(
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
        KeyKind::Admin,
    )?
    .filter(|path| path.exists());
    let admin_key_to_check = admin_key.as_deref().map(|path| diagnostics::KeyToCheck {
        path,
        fix: match setup::key_fix(mode.as_ref()) {
            setup::KeyFix::Docker(folder) => docker_admin_key_fix(&folder),
            setup::KeyFix::OnServer => "check the admin key on the server; `kkt key create --admin --force` replaces it, then restart the server".to_string(),
        },
    });
    // The lines about this machine, whatever server `--url` points at: how
    // `kkt setup` set it up (or installed it, unfinished), or else its kkt
    // service.
    let machine = match mode {
        // The server is elsewhere; the server lines are about it.
        Some(setup::Mode::Client) => diagnostics::ThisMachine::Elsewhere,
        Some(setup::Mode::Docker(install)) => {
            let folder = kkt_container::Folder(install.folder);
            let state = kkt_container::status(&kkt_container::OsHost, &folder);
            let running = matches!(&state, Ok(state) if state.running);
            let tailscale_login = match &state {
                Ok(state) => state.login.clone(),
                Err(_) => None,
            };
            // A missing or unreadable copy has its own line.
            let web_key_copy = match &web_key {
                Some(path) if running && path.is_file() => {
                    let compared =
                        kkt_container::compare_web_key(&kkt_container::OsHost, &folder, path);
                    Some(docker_web_key_check(compared, &folder))
                }
                _ => None,
            };
            diagnostics::ThisMachine::Docker {
                kkt: docker_check(state, &folder),
                tailscale_login: tailscale_login
                    .map(|login| tailscale_login_check(&login, &folder)),
                web_key_copy,
            }
        }
        Some(setup::Mode::Service { .. }) | None => {
            diagnostics::ThisMachine::Service(service_check(env))
        }
    };
    let plan = diagnostics::Plan {
        server,
        server_fix,
        web_key: web_key_to_check,
        admin_key: admin_key_to_check,
        machine,
        unfinished: unfinished.map(|unfinished| unfinished.map(|unfinished| unfinished.summary())),
        unrecorded: unrecorded_check(unrecorded),
    };
    let (report, all_ok) = diagnostics::report(plan.checks(inspector));
    print!("{report}");
    match clients::codex_password_store_hint(env, &ClientHost) {
        Ok(true) => println!(
            "info: Codex: mcp_oauth_credentials_store is unset; a password-store timeout can prevent kkt from connecting. See https://github.com/sam-mfb/kashkeshet/blob/main/docs/clients.md#codex for the optional file-store setting and what it changes."
        ),
        Ok(false) => {}
        Err(error) => println!("info: Codex config could not be checked: {error}"),
    }
    if all_ok {
        Ok(())
    } else {
        Err("doctor found problems; see the fixes above".into())
    }
}

/// How to record a server found installed with no record.
const ADOPT_FIX: &str = "run `kkt setup` to record it (`kkt setup --adopt` with no terminal), so `kkt upgrade` manages it";

/// The doctor's line about a server kkt installed here with no record, if
/// one was looked for and found. kkt in Docker unrecorded is a problem:
/// only setup makes the managed folder, and `kkt upgrade` needs the record.
/// A service may have been installed with `kkt service install`, which
/// never needed a record, so for one it is only said.
fn unrecorded_check(
    unrecorded: Result<Option<setup::Unrecorded>, (setup::Purpose, String)>,
) -> Option<diagnostics::Check> {
    Some(match unrecorded {
        Ok(None) => return None,
        Ok(Some(found)) => match found.mode {
            setup::Mode::Docker(_) => {
                diagnostics::Check::problem(SETUP_CHECK, found.summary(), ADOPT_FIX)
            }
            _ => diagnostics::Check::ok(
                SETUP_CHECK,
                format!(
                    "{} To have `kkt upgrade` manage it, run `kkt setup` to record it.",
                    found.summary()
                ),
            ),
        },
        Err((setup::Purpose::Docker, e)) => {
            diagnostics::Check::problem(SETUP_CHECK, e, "run `kkt setup`, which sets it up again")
        }
        Err((_, e)) => diagnostics::Check::ok(SETUP_CHECK, e),
    })
}

/// The text of `fix`, kkt-setup's fix for a server that does not answer.
pub(crate) fn server_fix(fix: &setup::ServerFix) -> String {
    match fix {
        setup::ServerFix::Elsewhere => "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)"
            .to_string(),
        // From inside the folder: Compose reads a relative COMPOSE_FILE in
        // .env against the current folder.
        setup::ServerFix::Docker(folder) => format!(
            "this machine runs kkt in Docker: run `kkt status`, or start it with `cd {} && docker compose up -d`",
            diagnostics::shell_word(&folder.display().to_string())
        ),
    }
}

/// The doctor's line about the kkt service on this machine.
fn service_check(env: &Env) -> diagnostics::Check {
    match Manager::for_this_os(env).and_then(|manager| manager.state(&ServiceSystem)) {
        Ok(state) if !state.installed => diagnostics::Check::ok(
            SERVICE_CHECK,
            "not installed; optional when running `kkt serve` in a terminal",
        ),
        Ok(state) if state.running => {
            diagnostics::Check::ok(SERVICE_CHECK, "installed and running")
        }
        Ok(_) => diagnostics::Check::problem(
            SERVICE_CHECK,
            "installed but not running",
            "run `kkt service start`",
        ),
        Err(error) => diagnostics::Check::problem(
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
        diagnostics::shell_word(&folder.display().to_string())
    )
}

/// How to save, or read, the admin key of kkt in Docker in `folder`.
fn docker_admin_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup --admin-key` (run again with the same answers) saves the server's admin key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --admin`",
        diagnostics::shell_word(&folder.display().to_string())
    )
}

/// The doctor's line about whether the web key's host copy is the Docker
/// server's key, from what `kkt_container::compare_web_key` found.
fn docker_web_key_check(
    compared: Result<kkt_container::KeyCopy, kkt_container::DockerError>,
    folder: &kkt_container::Folder,
) -> diagnostics::Check {
    match compared {
        Ok(kkt_container::KeyCopy::Same) => {
            diagnostics::Check::ok(DOCKER_WEB_KEY_CHECK, "the same as the server's")
        }
        Ok(kkt_container::KeyCopy::Differs) => diagnostics::Check::problem(
            DOCKER_WEB_KEY_CHECK,
            "differs from the server's web key, which was changed after the copy was saved; it no longer logs in",
            docker_web_key_fix(folder.path()),
        ),
        Err(e) => diagnostics::Check::problem(
            DOCKER_WEB_KEY_CHECK,
            format!("could not compare it with the server's web key: {e}"),
            format!(
                "check kkt with `cd {} && docker compose logs kkt`",
                diagnostics::shell_word(&folder.path().display().to_string())
            ),
        ),
    }
}

/// The doctor's line about the Tailscale container's login.
fn tailscale_login_check(
    login: &kkt_container::Login,
    folder: &kkt_container::Folder,
) -> diagnostics::Check {
    match login {
        kkt_container::Login::LoggedIn => {
            diagnostics::Check::ok(DOCKER_TAILSCALE_CHECK, "logged in")
        }
        kkt_container::Login::NeedsLogin => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "not logged in: the machine's key expired, or it was logged out, so it is off the tailnet and its HTTPS address does not answer",
            "run `kkt setup` again and log in with the link it shows",
        ),
        kkt_container::Login::Unreachable => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "the Tailscale container does not answer",
            format!(
                "start it with `cd {} && docker compose up -d`",
                diagnostics::shell_word(&folder.path().display().to_string())
            ),
        ),
        kkt_container::Login::Other(state) => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            format!("Tailscale says: {state}"),
            "check the machine in the Tailscale admin console, or run `kkt setup` again",
        ),
    }
}

/// The doctor's line about kkt in Docker, from what `kkt_container::status` found
/// in the managed `folder`.
fn docker_check(
    state: Result<kkt_container::State, kkt_container::DockerError>,
    folder: &kkt_container::Folder,
) -> diagnostics::Check {
    let shown = folder.path().display().to_string();
    match state {
        Ok(state) if !state.files_present => diagnostics::Check::problem(
            DOCKER_CHECK,
            format!("its files are missing from {shown}"),
            "run `kkt setup` again",
        ),
        Ok(state) if state.running => diagnostics::Check::ok(
            DOCKER_CHECK,
            format!(
                "running kkt {}",
                state.version.as_deref().unwrap_or("(version unknown)")
            ),
        ),
        Ok(_) => diagnostics::Check::problem(
            DOCKER_CHECK,
            "not running",
            format!(
                // From inside the folder: Compose reads a relative
                // COMPOSE_FILE in .env against the current folder.
                "start it with `cd {} && docker compose up -d`",
                diagnostics::shell_word(&shown)
            ),
        ),
        Err(error) => diagnostics::Check::problem(
            DOCKER_CHECK,
            error.to_string(),
            "check that Docker is running, then run `kkt status`",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_fix_for_a_silent_server_names_how_this_machine_runs_kkt() {
        let fix = server_fix(&setup::ServerFix::Docker(PathBuf::from("/data/kkt docker")));
        assert!(
            fix.contains("`cd '/data/kkt docker' && docker compose up -d`"),
            "{fix}"
        );
        assert!(!fix.contains("kkt service"), "{fix}");
        assert!(!fix.contains("kkt serve"), "{fix}");
        let fix = server_fix(&setup::ServerFix::Elsewhere);
        assert!(fix.contains("another machine"), "{fix}");
    }
}
