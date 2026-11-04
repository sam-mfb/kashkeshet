//! `kkt doctor`: checks the server this machine's clients use, its keys,
//! and how this machine runs kkt, and prints a line for each with a fix for
//! each problem.

use crate::machine::connection::{ConnectionOptions, client_connection, recorded_connection};
use crate::machine::keys::{KeyKind, local_operator_key, local_web_key};
use crate::machine::record::{read_setup, read_unfinished, read_unrecorded, unfinished_mode};
use crate::machine::settings::ServeArgs;
use kkt_client_config as clients;
use kkt_container as docker;
use kkt_diagnostics as diagnostics;
use kkt_provenance::Build;
use kkt_service::{Manager, OsSystem};
use kkt_settings::Env;
use kkt_setup as setup;
use std::path::Path;

/// The name of the doctor line about the service on this machine.
const SERVICE_CHECK: &str = "this machine's kkt service";

/// The name of the doctor line about a setup that installed a server but
/// did not record it.
const SETUP_CHECK: &str = "this machine's setup";

pub async fn run(env: &Env, serve_args: ServeArgs, flags: ConnectionOptions) -> Result<(), String> {
    // With no record, a server setup installed but did not record stands in
    // for it; with no note of one either, a server kkt installed here anyway.
    let unfinished = read_unfinished(env);
    let recorded = read_setup(env)?;
    let unrecorded = match (&recorded, &unfinished) {
        (None, Ok(None)) => read_unrecorded(env, serve_args),
        _ => Ok(None),
    };
    let found = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let record = match recorded {
        Some(record) => Some(record),
        None => match &unfinished {
            Ok(Some(unfinished)) => unfinished.record.clone(),
            _ => found.map(|found| found.record(Build::CURRENT.version)),
        },
    };
    let connection = match &record {
        Some(record) if flags.config.is_none() => recorded_connection(record.for_clients(), flags)?,
        _ => client_connection(env, flags, false)?,
    };
    let mut checks = diagnostics::inspect(&connection.url, connection.key_file.as_deref()).await?;
    // A server machine whose clients were switched checks the other server
    // above, and so gets a client's fix.
    let switched = record.as_ref().is_some_and(|r| r.clients.is_some());
    let mode = match (record, &unfinished) {
        (Some(record), _) => Some(record.mode),
        (None, Ok(Some(unfinished))) => Some(unfinished_mode(env, unfinished)?),
        (None, _) => None,
    };
    let fix = match switched {
        true => server_fix(&setup::Mode::Client),
        false => mode.as_ref().and_then(server_fix),
    };
    if let Some(fix) = fix {
        point_server_fix_at(&mut checks, &fix);
    }
    let web_key = local_web_key(
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
    )?;
    if let Some(path) = &web_key {
        let fix = match &mode {
            Some(setup::Mode::Docker(install)) => docker_web_key_fix(&install.folder),
            _ => "check the operator web key on the server; `kkt key create --web` creates it if missing, then restart the server".to_string(),
        };
        checks.extend(diagnostics::inspect_web_key(path, &fix));
    }
    // Most servers have no admin key; only one that exists is checked.
    let admin_key = local_operator_key(
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
        KeyKind::Admin,
    )?
    .filter(|path| path.exists());
    if let Some(path) = &admin_key {
        let fix = match &mode {
            Some(setup::Mode::Docker(install)) => docker_admin_key_fix(&install.folder),
            _ => "check the admin key on the server; `kkt key create --admin --force` replaces it, then restart the server".to_string(),
        };
        checks.extend(diagnostics::inspect_admin_key(path, &fix));
    }
    // The last lines are about this machine, whatever server `--url` points
    // at: how `kkt setup` set it up (or installed it, unfinished), or else
    // its kkt service.
    match mode {
        // The server is elsewhere; the lines above are about it.
        Some(setup::Mode::Client) => {}
        Some(setup::Mode::Docker(install)) => {
            let folder = docker::Folder(install.folder);
            let state = docker::status(&docker::OsHost, &folder);
            let running = matches!(&state, Ok(state) if state.running);
            checks.extend(docker_checks(state, &folder));
            // A missing or unreadable copy has its own line above.
            if running
                && let Some(path) = &web_key
                && path.is_file()
            {
                let compared = docker::compare_web_key(&docker::OsHost, &folder, path);
                checks.push(docker_web_key_check(compared, &folder));
            }
        }
        Some(setup::Mode::Service { .. }) | None => checks.push(service_check(env)),
    }
    match unfinished {
        Ok(None) => {}
        Ok(Some(unfinished)) => checks.push(diagnostics::Check::problem(
            SETUP_CHECK,
            unfinished.summary(),
            "run `kkt setup` again to finish it",
        )),
        Err(e) => checks.push(diagnostics::Check::problem(
            SETUP_CHECK,
            e,
            "run `kkt setup` again",
        )),
    }
    checks.extend(unrecorded_check(unrecorded));
    let (report, all_ok) = doctor_report(checks);
    print!("{report}");
    match clients::codex_password_store_hint(env) {
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

/// The fix for a server that does not answer, on a machine set up as `mode`
/// where starting the kkt service is not how kkt runs; `None` where it is.
pub(crate) fn server_fix(mode: &setup::Mode) -> Option<String> {
    match mode {
        setup::Mode::Client => Some(
            "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)"
                .to_string(),
        ),
        // From inside the folder: Compose reads a relative COMPOSE_FILE in
        // .env against the current folder.
        setup::Mode::Docker(install) => Some(format!(
            "this machine runs kkt in Docker: run `kkt status`, or start it with `cd {} && docker compose up -d`",
            diagnostics::shell_word(&install.folder.display().to_string())
        )),
        setup::Mode::Service { .. } => None,
    }
}

/// Replaces the fix on a failed server check, which tells the person to
/// start the kkt service, with `fix`, for a machine where that is not how kkt
/// runs.
pub(crate) fn point_server_fix_at(checks: &mut [diagnostics::Check], fix: &str) {
    for check in checks.iter_mut() {
        if check.name == diagnostics::SERVER_CHECK && !check.ok {
            check.fix = Some(fix.to_string());
        }
    }
}

/// The doctor's line about the kkt service on this machine.
fn service_check(env: &Env) -> diagnostics::Check {
    match Manager::for_this_os(env).and_then(|manager| manager.state(&OsSystem)) {
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

/// The doctor's lines about kkt in Docker, from what `docker::status` found
/// in the managed `folder`: kkt itself, then, if it is served over Tailscale,
/// the Tailscale login.
fn docker_checks(
    state: Result<docker::State, docker::DockerError>,
    folder: &docker::Folder,
) -> Vec<diagnostics::Check> {
    let login = match &state {
        Ok(state) => state.login.clone(),
        Err(_) => None,
    };
    let mut checks = vec![docker_check(state, folder)];
    checks.extend(login.map(|login| tailscale_login_check(&login, folder)));
    checks
}

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
/// server's key, from what `docker::compare_web_key` found.
fn docker_web_key_check(
    compared: Result<docker::KeyCopy, docker::DockerError>,
    folder: &docker::Folder,
) -> diagnostics::Check {
    match compared {
        Ok(docker::KeyCopy::Same) => {
            diagnostics::Check::ok(DOCKER_WEB_KEY_CHECK, "the same as the server's")
        }
        Ok(docker::KeyCopy::Differs) => diagnostics::Check::problem(
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
fn tailscale_login_check(login: &docker::Login, folder: &docker::Folder) -> diagnostics::Check {
    match login {
        docker::Login::LoggedIn => diagnostics::Check::ok(DOCKER_TAILSCALE_CHECK, "logged in"),
        docker::Login::NeedsLogin => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "not logged in: the machine's key expired, or it was logged out, so it is off the tailnet and its HTTPS address does not answer",
            "run `kkt setup` again and log in with the link it shows",
        ),
        docker::Login::Unreachable => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            "the Tailscale container does not answer",
            format!(
                "start it with `cd {} && docker compose up -d`",
                diagnostics::shell_word(&folder.path().display().to_string())
            ),
        ),
        docker::Login::Other(state) => diagnostics::Check::problem(
            DOCKER_TAILSCALE_CHECK,
            format!("Tailscale says: {state}"),
            "check the machine in the Tailscale admin console, or run `kkt setup` again",
        ),
    }
}

/// The doctor's line about kkt in Docker, from what `docker::status` found
/// in the managed `folder`.
fn docker_check(
    state: Result<docker::State, docker::DockerError>,
    folder: &docker::Folder,
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

/// The doctor's lines for `checks`, and whether all of them are ok.
pub(crate) fn doctor_report(checks: Vec<diagnostics::Check>) -> (String, bool) {
    let mut report = String::new();
    let mut all_ok = true;
    for check in checks {
        all_ok &= check.ok;
        report.push_str(&format!(
            "{}: {}: {}\n",
            if check.ok { "ok" } else { "problem" },
            check.name,
            check.detail
        ));
        if let Some(fix) = check.fix {
            report.push_str(&format!("  fix: {fix}\n"));
        }
    }
    (report, all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn docker(tailscale: bool) -> setup::Mode {
        setup::Mode::Docker(setup::DockerSetup {
            folder: PathBuf::from("/data/kkt docker"),
            tailscale,
            hostname: None,
            tailnet: None,
        })
    }

    #[test]
    fn the_fix_for_a_silent_server_names_how_this_machine_runs_kkt() {
        for tailscale in [false, true] {
            let fix = server_fix(&docker(tailscale)).expect("a Docker fix");
            assert!(
                fix.contains("`cd '/data/kkt docker' && docker compose up -d`"),
                "{fix}"
            );
            assert!(!fix.contains("kkt service"), "{fix}");
            assert!(!fix.contains("kkt serve"), "{fix}");
        }
        let fix = server_fix(&setup::Mode::Client).expect("a client fix");
        assert!(fix.contains("another machine"), "{fix}");
        // A service machine keeps the service fix.
        for tailscale in [false, true] {
            assert_eq!(server_fix(&setup::Mode::Service { tailscale }), None);
        }
    }
}
