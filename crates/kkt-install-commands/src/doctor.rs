//! `kkt doctor`: checks the server this machine's clients use, its keys,
//! and how this machine runs kkt, and prints a line for each with a fix for
//! each problem.

use crate::keys::{KeyKind, local_operator_key, local_web_key};
use crate::record::{read_setup, read_unfinished, read_unrecorded, unfinished_mode};
use kkt_deployment::connection::{self, ConnectionOptions, ConnectionSource};
use kkt_diagnostics as diagnostics;
use kkt_llm_agent_config as clients;
use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_setup as setup;
use setup::{Mode, Record, Unfinished, Unrecorded};

pub async fn run<D: crate::ServiceDefaults>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
    flags: ConnectionOptions,
) -> Result<(), String> {
    // With no record, a server setup installed but did not record stands in
    // for it; with no note of one either, a server kkt installed here anyway.
    let unfinished = read_unfinished(env, parts.setup_files, parts.container, parts.service);
    let recorded = read_setup(env, parts.setup_files)?;
    let unrecorded =
        match kkt_deployment::status_looks_for_unrecorded(recorded.as_ref(), &unfinished) {
            true => read_unrecorded(parts, env, platform),
            false => Ok(None),
        };
    let found = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let record = doctor_record(recorded, &unfinished, found, Build::CURRENT.version);
    let connection = match &record {
        Some(record) if flags.config.is_none() => connection::recorded_connection(
            |path| parts.connection.absolute(path),
            crate::deployment::recorded_server(record).for_clients(),
            flags,
        )?,
        _ => connection::client_connection(parts.connection, flags, false)?,
    };
    let inspector = diagnostics::Inspector::new(parts.prober);
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
    let server_fix = mode
        .as_ref()
        .and_then(|mode| diagnostics::server_fix(&setup_of(&clients_server_mode(switched, mode))));
    let checked_setup = mode.as_ref().map(setup_of);
    let web_key = local_web_key(
        parts,
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
    )?;
    let web_key_to_check = web_key.as_deref().map(|path| diagnostics::KeyToCheck {
        path,
        fix: diagnostics::web_key_fix(checked_setup.as_ref()),
    });
    // Most servers have no admin key; only one that exists is checked. One
    // that cannot be looked at is checked too, and its check says why.
    let admin_key = local_operator_key(
        parts,
        env,
        unfinished.as_ref().ok().and_then(Option::as_ref),
        found,
        KeyKind::Admin,
    )?
    .filter(|path| !matches!(parts.key_files.try_exists(path), Ok(false)));
    let admin_key_to_check = admin_key.as_deref().map(|path| diagnostics::KeyToCheck {
        path,
        fix: diagnostics::admin_key_fix(checked_setup.as_ref()),
    });
    // The lines about this machine, whatever server `--url` points at: how
    // `kkt setup` set it up (or installed it, unfinished), or else its kkt
    // service.
    let machine = match mode {
        // The server is elsewhere; the server lines are about it.
        Some(setup::Mode::Client) => diagnostics::ThisMachine::Elsewhere,
        Some(setup::Mode::Docker(install)) => {
            let folder = kkt_container::Folder(install.folder);
            let state = parts.container.status(
                &folder,
                parts.container_configs,
                &parts.serve.default_name_pack(),
                &kkt_config_file::config_file_in_home(),
            );
            let running = matches!(&state, Ok(state) if state.running);
            let tailscale_login = match &state {
                Ok(state) => state.login.clone(),
                Err(_) => None,
            };
            // A missing or unreadable copy has its own line.
            let web_key_copy = match &web_key {
                Some(path) if running && parts.key_files.is_file(path) => {
                    let compared = parts
                        .container
                        .compare_web_key(&folder, path)
                        .map(|copy| match copy {
                            kkt_containerized_service::KeyCopy::Same => {
                                diagnostics::WebKeyCopy::Same
                            }
                            kkt_containerized_service::KeyCopy::Differs => {
                                diagnostics::WebKeyCopy::Differs
                            }
                        })
                        .map_err(String::from);
                    Some(diagnostics::docker_web_key_check(compared, folder.path()))
                }
                _ => None,
            };
            diagnostics::ThisMachine::Docker {
                kkt: diagnostics::docker_check(
                    state
                        .map(|state| diagnostics::DockerKkt {
                            files_present: state.files_present,
                            running: state.running,
                            version: state.version,
                        })
                        .map_err(String::from),
                    folder.path(),
                ),
                tailscale_login: tailscale_login.map(|login| {
                    diagnostics::tailscale_login_check(&tailscale_login_of(login), folder.path())
                }),
                web_key_copy,
            }
        }
        Some(setup::Mode::Service { .. }) | None => {
            diagnostics::ThisMachine::Service(service_check(parts.service))
        }
    };
    let plan = diagnostics::Plan {
        server,
        server_fix,
        web_key: web_key_to_check,
        admin_key: admin_key_to_check,
        machine,
        unfinished: unfinished.map(|unfinished| unfinished.map(|unfinished| unfinished.summary())),
        unrecorded: diagnostics::unrecorded_check(unrecorded_server(unrecorded)),
    };
    let (report, all_ok) = diagnostics::report(plan.checks(inspector));
    parts.output.output(format_args!("{report}"));
    match clients::codex_password_store_hint(env, parts.clients) {
        Ok(true) => parts.output.output(format_args!(
            "info: Codex: mcp_oauth_credentials_store is unset; a password-store timeout can prevent kkt from connecting. See https://github.com/sam-mfb/kashkeshet/blob/main/docs/clients.md#codex for the optional file-store setting and what it changes.\n"
        )),
        Ok(false) => {}
        Err(error) => parts.output.output(format_args!("info: Codex config could not be checked: {error}\n")),
    }
    if all_ok {
        Ok(())
    } else {
        Err("doctor found problems; see the fixes above".into())
    }
}

fn service_check(service: &dyn kkt_bare_service::Service) -> kkt_diagnostics::Check {
    diagnostics::service_check(
        service
            .ready()
            .and_then(|()| service.state())
            .map(|state| diagnostics::ServiceSeen {
                installed: state.installed,
                running: state.running,
            })
            .map_err(|e| crate::record::why_state_unknown(&e)),
    )
}

/// The setup `kkt doctor` goes by: the `recorded` one; else the one an
/// `unfinished` setup installed but did not record; else, with no note of
/// one, a server `found` installed here with no record, adopted as kkt
/// `version` would record it. `found` is looked for only when there is no
/// record and no note of an unfinished setup, readable or not.
fn doctor_record(
    recorded: Option<Record>,
    unfinished: &Result<Option<Unfinished>, String>,
    found: Option<&Unrecorded>,
    version: &str,
) -> Option<Record> {
    match (recorded, unfinished) {
        (Some(record), _) => Some(record),
        (None, Ok(Some(unfinished))) => unfinished.record.clone(),
        (None, _) => found.map(|found| found.record(version)),
    }
}

/// How the server this machine's clients use is checked, on a machine set
/// up as `mode`: as a client's when the clients were `switched` to another
/// machine's server, otherwise as `mode` says. Setup's own check of the
/// server it set up goes by it too.
pub(crate) fn clients_server_mode(switched: bool, mode: &Mode) -> Mode {
    match switched {
        true => Mode::Client,
        false => mode.clone(),
    }
}

/// kkt-diagnostics' fact for a machine set up as `mode`.
pub(crate) fn setup_of(mode: &Mode) -> diagnostics::Setup {
    match mode {
        Mode::Service { .. } => diagnostics::Setup::Service,
        Mode::Docker(install) => diagnostics::Setup::Docker {
            folder: install.folder.clone(),
        },
        Mode::Client => diagnostics::Setup::Client,
    }
}

/// kkt-diagnostics' fact for the Tailscale login of kkt in Docker.
fn tailscale_login_of(login: kkt_containerized_service::Login) -> diagnostics::TailscaleLogin {
    match login {
        kkt_containerized_service::Login::LoggedIn => diagnostics::TailscaleLogin::LoggedIn,
        kkt_containerized_service::Login::NeedsLogin => diagnostics::TailscaleLogin::NeedsLogin,
        kkt_containerized_service::Login::Unreachable => diagnostics::TailscaleLogin::Unreachable,
        kkt_containerized_service::Login::Other(state) => diagnostics::TailscaleLogin::Other(state),
    }
}

/// kkt-diagnostics' fact for what looking for a server installed with no
/// record found: `None` when it found none.
fn unrecorded_server(
    unrecorded: Result<Option<Unrecorded>, (setup::Purpose, String)>,
) -> Option<diagnostics::UnrecordedServer> {
    match unrecorded {
        Ok(None) => None,
        Ok(Some(found)) => Some(diagnostics::UnrecordedServer::Found {
            in_docker: matches!(found.mode, Mode::Docker(_)),
            summary: found.summary(),
        }),
        Err((purpose, reason)) => Some(diagnostics::UnrecordedServer::Unreadable {
            in_docker: purpose == setup::Purpose::Docker,
            reason,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use setup::{DockerSetup, Purpose, Server};
    use std::path::PathBuf;

    fn docker(tailscale: bool) -> Mode {
        Mode::Docker(DockerSetup {
            folder: PathBuf::from("/data/kkt docker"),
            tailscale,
            hostname: None,
            tailnet: None,
        })
    }

    fn folder() -> PathBuf {
        PathBuf::from("/data/kkt docker")
    }

    fn service() -> Mode {
        Mode::Service { tailscale: false }
    }

    fn record(mode: Mode) -> Record {
        Record {
            mode,
            written_by: "0.5.0".into(),
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: None,
            clients: None,
        }
    }

    fn switched(mode: Mode) -> Record {
        let mut record = record(mode);
        record.clients = Some(Server {
            url: "https://other/mcp".into(),
            key_file: None,
        });
        record
    }

    fn unfinished(record: Option<Record>) -> Unfinished {
        Unfinished {
            purpose: Purpose::Docker,
            tailscale: false,
            allow_hosts: Vec::new(),
            clients: Vec::new(),
            tailscale_login: None,
            record,
        }
    }

    fn found(mode: Mode) -> Unrecorded {
        Unrecorded {
            mode,
            url: "http://127.0.0.1:9000/mcp".into(),
            key_file: None,
        }
    }

    /// The setup the server the clients use is checked as, on a machine
    /// set up as `mode` whose clients may have been `switched`.
    fn checked_as(switched: bool, mode: Option<&Mode>) -> Option<diagnostics::Setup> {
        mode.map(|mode| setup_of(&clients_server_mode(switched, mode)))
    }

    #[test]
    fn doctor_goes_by_the_record_even_of_a_switched_server() {
        let record = switched(service());
        let found = found(docker(false));
        let chosen = doctor_record(
            Some(record.clone()),
            &Ok(Some(unfinished(None))),
            Some(&found),
            "0.7.0",
        );
        assert_eq!(chosen, Some(record.clone()));
        // Its clients use another machine's server, so that is checked as a
        // client's.
        assert_eq!(
            checked_as(record.clients.is_some(), Some(&record.mode)),
            Some(diagnostics::Setup::Client)
        );
        assert_eq!(clients_server_mode(true, &docker(false)), Mode::Client);
        assert_eq!(
            checked_as(false, Some(&service())),
            Some(diagnostics::Setup::Service)
        );
    }

    #[test]
    fn doctor_goes_by_what_an_unfinished_setup_installed() {
        let installed = record(docker(false));
        let noted = Ok(Some(unfinished(Some(installed.clone()))));
        assert_eq!(doctor_record(None, &noted, None, "0.7.0"), Some(installed));
        // One that stopped before installing has no record to go by.
        assert_eq!(
            doctor_record(None, &Ok(Some(unfinished(None))), None, "0.7.0"),
            None
        );
        // Its mode comes from where it goes, and was never switched.
        assert_eq!(clients_server_mode(false, &docker(false)), docker(false));
        assert_eq!(
            checked_as(false, Some(&docker(false))),
            Some(diagnostics::Setup::Docker { folder: folder() })
        );
    }

    #[test]
    fn an_unreadable_note_leaves_nothing_to_go_by() {
        let unreadable = Err("bad note".to_string());
        assert_eq!(doctor_record(None, &unreadable, None, "0.7.0"), None);
        assert_eq!(checked_as(false, None), None);
    }

    #[test]
    fn doctor_goes_by_a_server_found_installed() {
        let found = found(docker(false));
        let chosen = doctor_record(None, &Ok(None), Some(&found), "0.7.0");
        assert_eq!(chosen, Some(found.record("0.7.0")));
        assert_eq!(
            checked_as(false, chosen.as_ref().map(|record| &record.mode)),
            Some(diagnostics::Setup::Docker { folder: folder() })
        );
    }

    #[test]
    fn with_nothing_found_doctor_has_no_setup() {
        assert_eq!(doctor_record(None, &Ok(None), None, "0.7.0"), None);
    }

    #[test]
    fn each_mode_is_handed_to_the_doctor_as_its_setup() {
        for tailscale in [false, true] {
            assert_eq!(
                setup_of(&docker(tailscale)),
                diagnostics::Setup::Docker { folder: folder() }
            );
            assert_eq!(
                setup_of(&Mode::Service { tailscale }),
                diagnostics::Setup::Service
            );
        }
        assert_eq!(setup_of(&Mode::Client), diagnostics::Setup::Client);
    }

    #[test]
    fn the_fix_for_a_silent_server_names_how_this_machine_runs_kkt() {
        let fix = diagnostics::server_fix(&setup_of(&docker(false))).expect("a Docker fix");
        assert!(
            fix.contains("`cd '/data/kkt docker' && docker compose up -d`"),
            "{fix}"
        );
        assert!(!fix.contains("kkt service"), "{fix}");
        assert!(!fix.contains("kkt serve"), "{fix}");
        let fix = diagnostics::server_fix(&setup_of(&Mode::Client)).expect("a client fix");
        assert!(fix.contains("another machine"), "{fix}");
        assert_eq!(diagnostics::server_fix(&setup_of(&service())), None);
    }

    #[test]
    fn what_was_found_unrecorded_is_handed_to_the_doctor_as_found() {
        assert_eq!(unrecorded_server(Ok(None)), None);
        assert_eq!(
            unrecorded_server(Ok(Some(found(docker(false))))),
            Some(diagnostics::UnrecordedServer::Found {
                in_docker: true,
                summary: found(docker(false)).summary(),
            })
        );
        assert_eq!(
            unrecorded_server(Ok(Some(found(service())))),
            Some(diagnostics::UnrecordedServer::Found {
                in_docker: false,
                summary: found(service()).summary(),
            })
        );
        assert_eq!(
            unrecorded_server(Err((Purpose::Docker, "bad".into()))),
            Some(diagnostics::UnrecordedServer::Unreadable {
                in_docker: true,
                reason: "bad".into(),
            })
        );
        assert_eq!(
            unrecorded_server(Err((Purpose::Service, "bad".into()))),
            Some(diagnostics::UnrecordedServer::Unreadable {
                in_docker: false,
                reason: "bad".into(),
            })
        );
    }
}
