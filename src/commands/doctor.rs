//! `kkt doctor`: checks the server this machine's clients use, its keys,
//! and how this machine runs kkt, and prints a line for each with a fix for
//! each problem.

use crate::machine::client_config::ClientHost;
use crate::machine::connection::{ConnectionOptions, client_connection, recorded_connection};
use crate::machine::container::ContainerHost;
use crate::machine::diagnostics::DiagnosticProber;
use crate::machine::keys::{KeyKind, local_operator_key, local_web_key};
use crate::machine::record::{read_setup, read_unfinished, read_unrecorded, unfinished_mode};
use crate::machine::service::ServiceSystem;
use crate::machine::settings::{ContainerConfigs, ServeArgs};
pub(crate) use checks::server_fix_text as server_fix;
use kkt_client_config as clients;
use kkt_diagnostics as diagnostics;
use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_service::Manager;
use kkt_settings::Limits;
use kkt_setup as setup;
use kkt_terminal_os::Terminal;
use setup::doctor_checks as checks;

pub async fn run(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    flags: ConnectionOptions,
) -> Result<(), String> {
    // With no record, a server setup installed but did not record stands in
    // for it; with no note of one either, a server kkt installed here anyway.
    let unfinished = read_unfinished(env);
    let recorded = read_setup(env)?;
    let unrecorded = match setup::status_looks_for_unrecorded(recorded.as_ref(), &unfinished) {
        true => read_unrecorded(env, platform, serve_args),
        false => Ok(None),
    };
    let found = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let record = setup::doctor_record(recorded, &unfinished, found, Build::CURRENT.version);
    let connection = match &record {
        Some(record) if flags.config.is_none() => recorded_connection(record.for_clients(), flags)?,
        _ => client_connection(env, platform, flags, false)?,
    };
    let inspector = diagnostics::Inspector::new(&DiagnosticProber);
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
        (None, Ok(Some(unfinished))) => Some(unfinished_mode(env, platform, unfinished)?),
        (None, _) => None,
    };
    let server_fix = setup::clients_server_fix(switched, mode.as_ref()).map(|fix| server_fix(&fix));
    let web_key = local_web_key(
        env,
        platform,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
    )?;
    let web_key_to_check = web_key.as_deref().map(|path| diagnostics::KeyToCheck {
        path,
        fix: checks::web_key_fix(mode.as_ref()),
    });
    // Most servers have no admin key; only one that exists is checked.
    let admin_key = local_operator_key(
        env,
        platform,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
        KeyKind::Admin,
    )?
    .filter(|path| kkt_admission_os::KeyFiles::exists(path));
    let admin_key_to_check = admin_key.as_deref().map(|path| diagnostics::KeyToCheck {
        path,
        fix: checks::admin_key_fix(mode.as_ref()),
    });
    // The lines about this machine, whatever server `--url` points at: how
    // `kkt setup` set it up (or installed it, unfinished), or else its kkt
    // service.
    let machine = match mode {
        // The server is elsewhere; the server lines are about it.
        Some(setup::Mode::Client) => diagnostics::ThisMachine::Elsewhere,
        Some(setup::Mode::Docker(install)) => {
            let folder = kkt_container::Folder(install.folder);
            let state = kkt_container::status(
                &ContainerHost,
                &folder,
                &ContainerConfigs,
                Limits::DEFAULT.name_pack.name(),
            );
            let running = matches!(&state, Ok(state) if state.running);
            let tailscale_login = match &state {
                Ok(state) => state.login.clone(),
                Err(_) => None,
            };
            // A missing or unreadable copy has its own line.
            let web_key_copy = match &web_key {
                Some(path) if running && kkt_admission_os::KeyFiles::is_file(path) => {
                    let compared = kkt_container::compare_web_key(&ContainerHost, &folder, path);
                    Some(diagnostic_check(checks::docker_web_key_check(
                        compared, &folder,
                    )))
                }
                _ => None,
            };
            diagnostics::ThisMachine::Docker {
                kkt: diagnostic_check(checks::docker_check(state, &folder)),
                tailscale_login: tailscale_login
                    .map(|login| diagnostic_check(checks::tailscale_login_check(&login, &folder))),
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
        unrecorded: checks::unrecorded_check(unrecorded).map(diagnostic_check),
    };
    let (report, all_ok) = diagnostics::report(plan.checks(inspector));
    Terminal.output(format_args!("{report}"));
    match clients::codex_password_store_hint(env, &ClientHost) {
        Ok(true) => Terminal.output(format_args!(
            "info: Codex: mcp_oauth_credentials_store is unset; a password-store timeout can prevent kkt from connecting. See https://github.com/sam-mfb/kashkeshet/blob/main/docs/clients.md#codex for the optional file-store setting and what it changes.\n"
        )),
        Ok(false) => {}
        Err(error) => Terminal.output(format_args!("info: Codex config could not be checked: {error}\n")),
    }
    if all_ok {
        Ok(())
    } else {
        Err("doctor found problems; see the fixes above".into())
    }
}

fn service_check(env: &Env) -> diagnostics::Check {
    diagnostic_check(checks::service_check(
        Manager::for_this_os(env.home.as_deref(), env.config_home().as_deref())
            .and_then(|manager| manager.state(&ServiceSystem))
            .map(|state| (state.installed, state.running))
            .map_err(|e| e.to_string()),
    ))
}

fn diagnostic_check(check: checks::DoctorCheck) -> diagnostics::Check {
    diagnostics::Check {
        name: check.name,
        ok: check.ok,
        detail: check.detail,
        fix: check.fix,
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
