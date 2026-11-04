//! What `kkt status` and `kkt upgrade` do, by how this machine is set up.
//! The program reads the record, the note of an unfinished setup and what
//! it finds installed, and carries out what these functions return.

use super::adopt::Unrecorded;
use super::record::{Mode, Record, Unfinished};
use super::upgrade::{Upgrade, upgrade};
use std::path::PathBuf;

/// What `kkt status` says after the key lines, and what it does next.
#[derive(Debug, PartialEq, Eq)]
pub struct Status {
    /// Lines to print, in order.
    pub lines: Vec<String>,
    /// What to do once the lines are printed. An error is returned after
    /// them, so the lines are still printed.
    pub then: Result<StatusThen, String>,
}

/// What `kkt status` does after its lines.
#[derive(Debug, PartialEq, Eq)]
pub enum StatusThen {
    /// Print the configured name pack, then the kkt service's status.
    Service,
    /// Report on kkt in Docker, from this folder.
    Docker(PathBuf),
    /// Nothing more.
    Done,
}

/// Whether `kkt status` looks for a server kkt installed here with no
/// record: only when there is no record and no note of an unfinished setup,
/// readable or not.
pub fn status_looks_for_unrecorded(
    recorded: Option<&Record>,
    unfinished: &Result<Option<Unfinished>, String>,
) -> bool {
    recorded.is_none() && matches!(unfinished, Ok(None))
}

/// What `kkt status` says about this machine: from its `recorded` setup,
/// the note of an `unfinished` one, or an `unrecorded` install (looked for
/// only as [`status_looks_for_unrecorded`] says; otherwise `Ok(None)`).
/// `docker_folder` is where `kkt setup` keeps kkt's Docker files, needed
/// only for an unfinished Docker install.
pub fn status(
    recorded: Option<&Record>,
    unfinished: &Result<Option<Unfinished>, String>,
    unrecorded: &Result<Option<Unrecorded>, String>,
    docker_folder: Result<PathBuf, String>,
) -> Status {
    let mut lines = Vec::new();
    match unfinished {
        Err(e) => lines.push(format!(
            "Unfinished setup: unknown ({e}). Run `kkt setup` again."
        )),
        Ok(Some(unfinished)) => lines.push(format!(
            "Unfinished setup: {} Run `kkt setup` again to finish it.",
            unfinished.summary()
        )),
        Ok(None) => {}
    }
    match unrecorded {
        Err(e) => lines.push(format!("Unrecorded install: {e}")),
        Ok(Some(found)) => lines.push(format!(
            "Unrecorded install: {} Run `kkt setup` to record it, so `kkt upgrade` manages it.",
            found.summary()
        )),
        Ok(None) => {}
    }
    let unfinished = unfinished.as_ref().ok().and_then(Option::as_ref);
    let unrecorded = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let known = match (recorded, unfinished, unrecorded) {
        (Some(record), _, _) => {
            if let Some(clients) = &record.clients {
                lines.push(format!(
                    "Clients use: {} (switched from this machine's server; \
                     `kkt switch --local` points them back)",
                    clients.url
                ));
            }
            Ok((record.mode.clone(), record.url.clone()))
        }
        (None, Some(unfinished), _) => match &unfinished.record {
            Some(record) => Ok((record.mode.clone(), record.url.clone())),
            None => unfinished
                .mode(docker_folder)
                .map(|mode| (mode, "not known until the install finishes".to_string())),
        },
        (None, None, Some(found)) => Ok((found.mode.clone(), found.url.clone())),
        (None, None, None) => {
            lines.push(
                "No setup record; checking this machine's kkt service. Run `kkt setup` to \
                 configure this machine."
                    .to_string(),
            );
            return Status {
                lines,
                then: Ok(StatusThen::Service),
            };
        }
    };
    let (mode, url) = match known {
        Ok(known) => known,
        Err(e) => {
            return Status {
                lines,
                then: Err(e),
            };
        }
    };
    lines.push(format!("Mode: {}\nServer: {url}", mode.name()));
    let then = match mode {
        Mode::Service { .. } => StatusThen::Service,
        Mode::Docker(install) => StatusThen::Docker(install.folder),
        Mode::Client => {
            lines.push("Client only; the server runs on another machine.".to_string());
            StatusThen::Done
        }
    };
    Status {
        lines,
        then: Ok(then),
    }
}

/// The refusal to upgrade on a client machine.
fn client_upgrade() -> String {
    "this machine is a client; upgrade the server on the machine where it runs".into()
}

/// Where `kkt upgrade` finds the version of the server it replaces.
#[derive(Debug, PartialEq, Eq)]
pub enum RunningVersion {
    /// In the settings file of kkt in Docker, in this folder.
    DockerSettings(PathBuf),
    /// As the record says.
    Recorded(String),
}

/// Where the version of the server `record` describes is, for `kkt
/// upgrade`. A client machine has no server to upgrade.
pub fn running_version(record: &Record) -> Result<RunningVersion, String> {
    match &record.mode {
        Mode::Client => Err(client_upgrade()),
        Mode::Docker(install) => Ok(RunningVersion::DockerSettings(install.folder.clone())),
        Mode::Service { .. } => Ok(RunningVersion::Recorded(record.written_by.clone())),
    }
}

/// What `kkt upgrade` does.
#[derive(Debug, PartialEq, Eq)]
pub struct UpgradePlan {
    /// How to replace the server, then write `record`; `None` when it is
    /// already at the target version, and nothing is done.
    pub replace: Option<Replace>,
    /// The record to write once the server is replaced.
    pub record: Record,
    /// What to print at the end.
    pub said: String,
}

/// How `kkt upgrade` replaces the server.
#[derive(Debug, PartialEq, Eq)]
pub enum Replace {
    /// Upgrade kkt in Docker in `folder`, saving the host copies of its keys
    /// beside `key_file`.
    Docker {
        folder: PathBuf,
        key_file: Option<PathBuf>,
    },
    /// Back up, then install the kkt service again with this program.
    Service,
}

/// What `kkt upgrade` does to the server `record` describes, which runs
/// kkt `running`, to bring it to `target`.
pub fn upgrade_plan(
    record: &Record,
    running: &str,
    target: &str,
    allow_downgrade: bool,
) -> Result<UpgradePlan, String> {
    let how = match &record.mode {
        Mode::Client => return Err(client_upgrade()),
        Mode::Docker(install) => Replace::Docker {
            folder: install.folder.clone(),
            key_file: record.key_file.clone(),
        },
        Mode::Service { .. } => Replace::Service,
    };
    if upgrade(running, target, allow_downgrade)? == Upgrade::Already {
        return Ok(UpgradePlan {
            replace: None,
            record: record.clone(),
            said: format!("Server is already at kkt {target}"),
        });
    }
    let mut record = record.clone();
    record.written_by = target.to_string();
    Ok(UpgradePlan {
        replace: Some(how),
        record,
        said: format!("Upgraded the server to kkt {target}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DockerSetup, Purpose, Server};

    fn docker_mode() -> Mode {
        Mode::Docker(DockerSetup {
            folder: PathBuf::from("/data/docker"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        })
    }

    fn record(mode: Mode) -> Record {
        Record {
            mode,
            written_by: "0.5.0".into(),
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: Some(PathBuf::from("/home/k/access-key")),
            clients: None,
        }
    }

    fn unfinished(purpose: Purpose, record: Option<Record>) -> Unfinished {
        Unfinished {
            purpose,
            tailscale: false,
            allow_hosts: Vec::new(),
            clients: Vec::new(),
            tailscale_login: None,
            record,
        }
    }

    fn unrecorded(mode: Mode) -> Unrecorded {
        Unrecorded {
            mode,
            url: "http://127.0.0.1:9000/mcp".into(),
            key_file: None,
        }
    }

    fn folder() -> Result<PathBuf, String> {
        Ok(PathBuf::from("/data/managed"))
    }

    #[test]
    fn status_looks_for_an_unrecorded_install_only_with_nothing_else_known() {
        let service = record(Mode::Service { tailscale: false });
        assert!(status_looks_for_unrecorded(None, &Ok(None)));
        assert!(!status_looks_for_unrecorded(Some(&service), &Ok(None)));
        let noted = Ok(Some(unfinished(Purpose::Service, None)));
        assert!(!status_looks_for_unrecorded(None, &noted));
        assert!(!status_looks_for_unrecorded(None, &Err("bad".into())));
    }

    #[test]
    fn status_with_nothing_found_checks_the_service() {
        assert_eq!(
            status(None, &Ok(None), &Ok(None), folder()),
            Status {
                lines: lines(&[NO_RECORD]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn status_of_each_recorded_mode() {
        let service = status(
            Some(&record(Mode::Service { tailscale: true })),
            &Ok(None),
            &Ok(None),
            folder(),
        );
        assert_eq!(
            service.lines,
            ["Mode: service\nServer: http://127.0.0.1:8080/mcp"]
        );
        assert_eq!(service.then, Ok(StatusThen::Service));

        let docker = status(Some(&record(docker_mode())), &Ok(None), &Ok(None), folder());
        assert_eq!(
            docker.lines,
            ["Mode: docker\nServer: http://127.0.0.1:8080/mcp"]
        );
        assert_eq!(
            docker.then,
            Ok(StatusThen::Docker(PathBuf::from("/data/docker")))
        );

        let client = status(Some(&record(Mode::Client)), &Ok(None), &Ok(None), folder());
        assert_eq!(
            client.lines,
            [
                "Mode: client\nServer: http://127.0.0.1:8080/mcp",
                "Client only; the server runs on another machine."
            ]
        );
        assert_eq!(client.then, Ok(StatusThen::Done));
    }

    #[test]
    fn status_names_the_server_clients_were_switched_to() {
        let mut switched = record(Mode::Service { tailscale: false });
        switched.clients = Some(Server {
            url: "https://other/mcp".into(),
            key_file: None,
        });
        assert_eq!(
            status(Some(&switched), &Ok(None), &Ok(None), folder()),
            Status {
                lines: lines(&[
                    "Clients use: https://other/mcp (switched from this machine's server; \
                     `kkt switch --local` points them back)",
                    "Mode: service\nServer: http://127.0.0.1:8080/mcp"
                ]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    fn lines(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    const NO_RECORD: &str = "No setup record; checking this machine's kkt service. Run `kkt \
                             setup` to configure this machine.";

    const BEGAN_AS_SERVICE: &str = "Unfinished setup: An earlier `kkt setup` began to install \
                                    a server here as a service, but stopped before the install \
                                    finished; parts of it may be running. Run `kkt setup` \
                                    again to finish it.";

    const BEGAN_IN_DOCKER: &str = "Unfinished setup: An earlier `kkt setup` began to install a \
                                   server here in Docker, but stopped before the install \
                                   finished; parts of it may be running. Run `kkt setup` again \
                                   to finish it.";

    #[test]
    fn status_says_an_unfinished_setup_even_beside_a_record() {
        let status = status(
            Some(&record(Mode::Service { tailscale: false })),
            &Ok(Some(unfinished(Purpose::Service, None))),
            &Ok(None),
            folder(),
        );
        assert_eq!(
            status,
            Status {
                lines: lines(&[
                    BEGAN_AS_SERVICE,
                    "Mode: service\nServer: http://127.0.0.1:8080/mcp"
                ]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn status_of_an_unreadable_note_checks_the_service() {
        let status = status(None, &Err("bad note".into()), &Ok(None), folder());
        assert_eq!(
            status,
            Status {
                lines: lines(&[
                    "Unfinished setup: unknown (bad note). Run `kkt setup` again.",
                    NO_RECORD
                ]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn status_of_a_finished_but_unrecorded_install_uses_its_record() {
        let noted = unfinished(Purpose::Docker, Some(record(docker_mode())));
        let status = status(None, &Ok(Some(noted)), &Ok(None), Err("no home".into()));
        assert_eq!(
            status,
            Status {
                lines: lines(&[
                    "Unfinished setup: An earlier `kkt setup` installed a server here in \
                     Docker, from /data/docker, for clients at http://127.0.0.1:8080/mcp, but \
                     stopped before recording it. Run `kkt setup` again to finish it.",
                    "Mode: docker\nServer: http://127.0.0.1:8080/mcp"
                ]),
                then: Ok(StatusThen::Docker(PathBuf::from("/data/docker"))),
            }
        );
    }

    #[test]
    fn status_of_an_install_that_did_not_finish_uses_where_it_goes() {
        let docker = status(
            None,
            &Ok(Some(unfinished(Purpose::Docker, None))),
            &Ok(None),
            folder(),
        );
        assert_eq!(
            docker,
            Status {
                lines: lines(&[
                    BEGAN_IN_DOCKER,
                    "Mode: docker\nServer: not known until the install finishes"
                ]),
                then: Ok(StatusThen::Docker(PathBuf::from("/data/managed"))),
            }
        );

        let service = status(
            None,
            &Ok(Some(unfinished(Purpose::Service, None))),
            &Ok(None),
            Err("no home".into()),
        );
        assert_eq!(
            service,
            Status {
                lines: lines(&[
                    BEGAN_AS_SERVICE,
                    "Mode: service\nServer: not known until the install finishes"
                ]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn status_of_an_unfinished_docker_install_without_a_home_fails_after_its_lines() {
        let status = status(
            None,
            &Ok(Some(unfinished(Purpose::Docker, None))),
            &Ok(None),
            Err("set HOME to keep kkt's Docker files".into()),
        );
        assert_eq!(
            status,
            Status {
                lines: lines(&[BEGAN_IN_DOCKER]),
                then: Err("set HOME to keep kkt's Docker files".to_string()),
            }
        );
    }

    #[test]
    fn status_of_an_unrecorded_install() {
        let found = unrecorded(Mode::Service { tailscale: false });
        let status = status(None, &Ok(None), &Ok(Some(found)), folder());
        assert_eq!(
            status,
            Status {
                lines: lines(&[
                    "Unrecorded install: Found a kkt server as a service, for clients at \
                     http://127.0.0.1:9000/mcp, not recorded. Run `kkt setup` to record it, so \
                     `kkt upgrade` manages it.",
                    "Mode: service\nServer: http://127.0.0.1:9000/mcp"
                ]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn status_of_an_unrecorded_docker_install() {
        let found = unrecorded(docker_mode());
        let status = status(None, &Ok(None), &Ok(Some(found)), folder());
        assert_eq!(
            status,
            Status {
                lines: lines(&[
                    "Unrecorded install: Found a kkt server in Docker from /data/docker, for \
                     clients at http://127.0.0.1:9000/mcp, not recorded. Run `kkt setup` to \
                     record it, so `kkt upgrade` manages it.",
                    "Mode: docker\nServer: http://127.0.0.1:9000/mcp"
                ]),
                then: Ok(StatusThen::Docker(PathBuf::from("/data/docker"))),
            }
        );
    }

    #[test]
    fn status_of_an_unreadable_unrecorded_install_checks_the_service() {
        let status = status(None, &Ok(None), &Err("both found".into()), folder());
        assert_eq!(
            status,
            Status {
                lines: lines(&["Unrecorded install: both found", NO_RECORD]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn the_running_version_by_mode() {
        let client = "this machine is a client; upgrade the server on the machine where it runs";
        assert_eq!(
            running_version(&record(Mode::Client)),
            Err(client.to_string())
        );
        assert_eq!(
            running_version(&record(docker_mode())),
            Ok(RunningVersion::DockerSettings(PathBuf::from(
                "/data/docker"
            )))
        );
        assert_eq!(
            running_version(&record(Mode::Service { tailscale: false })),
            Ok(RunningVersion::Recorded("0.5.0".into()))
        );
    }

    #[test]
    fn an_upgrade_to_the_same_version_does_nothing() {
        let docker = record(docker_mode());
        assert_eq!(
            upgrade_plan(&docker, "0.6.0", "0.6.0", false),
            Ok(UpgradePlan {
                replace: None,
                record: docker,
                said: "Server is already at kkt 0.6.0".into(),
            })
        );
    }

    #[test]
    fn an_upgrade_replaces_the_server_and_records_the_new_version() {
        let docker = record(docker_mode());
        let mut written = docker.clone();
        written.written_by = "0.6.0".into();
        assert_eq!(
            upgrade_plan(&docker, "0.5.0", "0.6.0", false),
            Ok(UpgradePlan {
                replace: Some(Replace::Docker {
                    folder: PathBuf::from("/data/docker"),
                    key_file: Some(PathBuf::from("/home/k/access-key")),
                }),
                record: written,
                said: "Upgraded the server to kkt 0.6.0".into(),
            })
        );
        let service = record(Mode::Service { tailscale: true });
        let mut written = service.clone();
        written.written_by = "0.6.0".into();
        assert_eq!(
            upgrade_plan(&service, "0.5.0", "0.6.0", false),
            Ok(UpgradePlan {
                replace: Some(Replace::Service),
                record: written,
                said: "Upgraded the server to kkt 0.6.0".into(),
            })
        );
    }

    #[test]
    fn an_upgrade_refuses_a_client_and_a_downgrade() {
        assert_eq!(
            upgrade_plan(&record(Mode::Client), "0.5.0", "0.6.0", false),
            Err(
                "this machine is a client; upgrade the server on the machine where it runs"
                    .to_string()
            )
        );
        assert_eq!(
            upgrade_plan(&record(docker_mode()), "0.7.0", "0.6.0", false),
            Err(
                "refusing to downgrade the server from 0.7.0 to 0.6.0; pass --allow-downgrade \
                 to override"
                    .to_string()
            )
        );
    }

    #[test]
    fn an_upgrade_may_downgrade_when_allowed() {
        let mut written = record(docker_mode());
        written.written_by = "0.6.0".into();
        assert_eq!(
            upgrade_plan(&record(docker_mode()), "0.7.0", "0.6.0", true),
            Ok(UpgradePlan {
                replace: Some(Replace::Docker {
                    folder: PathBuf::from("/data/docker"),
                    key_file: Some(PathBuf::from("/home/k/access-key")),
                }),
                record: written,
                said: "Upgraded the server to kkt 0.6.0".into(),
            })
        );
    }
}
