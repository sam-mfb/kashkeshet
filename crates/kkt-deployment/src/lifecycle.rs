//! What `kkt status` and `kkt upgrade` do, by how kkt is deployed on this
//! machine. The caller reads the record, the note of an unfinished setup
//! and what it finds installed, hands their facts here, and carries out
//! what these functions return.

use super::upgrade::{Upgrade, upgrade};
use super::{Deployment, Noted, Recorded, ServerVersion};
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
pub fn status_looks_for_unrecorded<R, U>(
    recorded: Option<&R>,
    unfinished: &Result<Option<U>, String>,
) -> bool {
    recorded.is_none() && matches!(unfinished, Ok(None))
}

/// What `kkt status` says about this machine: from its `recorded`
/// deployment, the note of an `unfinished` one, or an `unrecorded` install
/// (looked for only as [`status_looks_for_unrecorded`] says; otherwise
/// `Ok(None)`).
pub fn status(
    recorded: Option<&Recorded>,
    unfinished: &Result<Option<Noted>, String>,
    unrecorded: &Result<Option<Noted>, String>,
) -> Status {
    let mut lines = Vec::new();
    match unfinished {
        Err(e) => lines.push(format!(
            "Unfinished setup: unknown ({e}). Run `kkt setup` again."
        )),
        Ok(Some(unfinished)) => lines.push(format!(
            "Unfinished setup: {} Run `kkt setup` again to finish it.",
            unfinished.summary
        )),
        Ok(None) => {}
    }
    match unrecorded {
        Err(e) => lines.push(format!("Unrecorded install: {e}")),
        Ok(Some(found)) => lines.push(format!(
            "Unrecorded install: {} Run `kkt setup` to record it, so `kkt upgrade` manages it.",
            found.summary
        )),
        Ok(None) => {}
    }
    let unfinished = unfinished.as_ref().ok().and_then(Option::as_ref);
    let unrecorded = unrecorded.as_ref().ok().and_then(Option::as_ref);
    let known = match (recorded, unfinished.or(unrecorded)) {
        (Some(recorded), _) => {
            if let Some(clients_url) = &recorded.clients_url {
                lines.push(format!(
                    "Clients use: {clients_url} (switched from this machine's server; \
                     `kkt switch --local` points them back)"
                ));
            }
            Ok((recorded.deployment.clone(), recorded.url.clone()))
        }
        (None, Some(noted)) => noted.found.clone().map(|found| {
            (
                found.deployment,
                found
                    .url
                    .unwrap_or_else(|| "not known until the install finishes".to_string()),
            )
        }),
        (None, None) => {
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
    let (deployment, url) = match known {
        Ok(known) => known,
        Err(e) => {
            return Status {
                lines,
                then: Err(e),
            };
        }
    };
    lines.push(format!("Mode: {}\nServer: {url}", deployment.name()));
    let then = match deployment {
        Deployment::Service { .. } => StatusThen::Service,
        Deployment::Docker { folder, .. } => StatusThen::Docker(folder),
        Deployment::Client => {
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
    Recorded(ServerVersion),
}

/// Where the version of the server `recorded` describes is, for `kkt
/// upgrade`. A client machine has no server to upgrade.
pub fn running_version(recorded: &Recorded) -> Result<RunningVersion, String> {
    match &recorded.deployment {
        Deployment::Client => Err(client_upgrade()),
        Deployment::Docker { folder, .. } => Ok(RunningVersion::DockerSettings(folder.clone())),
        Deployment::Service { .. } => Ok(RunningVersion::Recorded(recorded.version.clone())),
    }
}

/// What `kkt upgrade` does.
#[derive(Debug, PartialEq, Eq)]
pub struct UpgradePlan {
    /// How to replace the server; `None` when it is already at the target
    /// version, and nothing is done. Once it is replaced, the record names
    /// the target version.
    pub replace: Option<Replace>,
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

/// What `kkt upgrade` does to the server `recorded` describes, which runs
/// kkt `running`, to bring it to `target`.
pub fn upgrade_plan(
    recorded: &Recorded,
    running: &ServerVersion,
    target: &str,
    allow_downgrade: bool,
) -> Result<UpgradePlan, String> {
    let how = match &recorded.deployment {
        Deployment::Client => return Err(client_upgrade()),
        Deployment::Docker { folder, .. } => Replace::Docker {
            folder: folder.clone(),
            key_file: recorded.key_file.clone(),
        },
        Deployment::Service { .. } => Replace::Service,
    };
    if upgrade(running, target, allow_downgrade)? == Upgrade::Already {
        return Ok(UpgradePlan {
            replace: None,
            said: format!("Server is already at kkt {target}"),
        });
    }
    Ok(UpgradePlan {
        replace: Some(how),
        said: format!("Upgraded the server to kkt {target}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Found;

    fn docker_mode() -> Deployment {
        Deployment::Docker {
            folder: PathBuf::from("/data/docker"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        }
    }

    fn record(deployment: Deployment) -> Recorded {
        Recorded {
            deployment,
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: Some(PathBuf::from("/home/k/access-key")),
            clients_url: None,
            version: ServerVersion::Known("0.5.0".into()),
        }
    }

    /// The summaries kkt-setup gives these notes.
    const BEGAN_AS_SERVICE_SUMMARY: &str = "An earlier `kkt setup` began to install a \
                                            server here as a service, but stopped before \
                                            the install finished; parts of it may be \
                                            running.";
    const BEGAN_IN_DOCKER_SUMMARY: &str = "An earlier `kkt setup` began to install a server \
                                           here in Docker, but stopped before the install \
                                           finished; parts of it may be running.";

    /// An unfinished setup that began to install a server deployed as
    /// `deployment`, in a folder that is `found`.
    fn began(summary: &str, found: Result<Deployment, String>) -> Noted {
        Noted {
            summary: summary.into(),
            found: found.map(|deployment| Found {
                deployment,
                url: None,
            }),
        }
    }

    fn unrecorded(deployment: Deployment, summary: &str) -> Noted {
        Noted {
            summary: summary.into(),
            found: Ok(Found {
                deployment,
                url: Some("http://127.0.0.1:9000/mcp".into()),
            }),
        }
    }

    fn managed_docker() -> Deployment {
        Deployment::Docker {
            folder: PathBuf::from("/data/managed"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        }
    }

    fn service() -> Deployment {
        Deployment::Service { tailscale: false }
    }

    #[test]
    fn status_looks_for_an_unrecorded_install_only_with_nothing_else_known() {
        let recorded = record(service());
        let nothing: Result<Option<Noted>, String> = Ok(None);
        assert!(status_looks_for_unrecorded::<Recorded, _>(None, &nothing));
        assert!(!status_looks_for_unrecorded(Some(&recorded), &nothing));
        let noted = Ok(Some(began(BEGAN_AS_SERVICE_SUMMARY, Ok(service()))));
        assert!(!status_looks_for_unrecorded::<Recorded, _>(None, &noted));
        let unreadable: Result<Option<Noted>, String> = Err("bad".into());
        assert!(!status_looks_for_unrecorded::<Recorded, _>(
            None,
            &unreadable
        ));
    }

    #[test]
    fn status_with_nothing_found_checks_the_service() {
        assert_eq!(
            status(None, &Ok(None), &Ok(None)),
            Status {
                lines: lines(&[NO_RECORD]),
                then: Ok(StatusThen::Service),
            }
        );
    }

    #[test]
    fn status_of_each_recorded_mode() {
        let service = status(
            Some(&record(Deployment::Service { tailscale: true })),
            &Ok(None),
            &Ok(None),
        );
        assert_eq!(
            service.lines,
            ["Mode: service\nServer: http://127.0.0.1:8080/mcp"]
        );
        assert_eq!(service.then, Ok(StatusThen::Service));

        let docker = status(Some(&record(docker_mode())), &Ok(None), &Ok(None));
        assert_eq!(
            docker.lines,
            ["Mode: docker\nServer: http://127.0.0.1:8080/mcp"]
        );
        assert_eq!(
            docker.then,
            Ok(StatusThen::Docker(PathBuf::from("/data/docker")))
        );

        let client = status(Some(&record(Deployment::Client)), &Ok(None), &Ok(None));
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
        let mut switched = record(service());
        switched.clients_url = Some("https://other/mcp".into());
        assert_eq!(
            status(Some(&switched), &Ok(None), &Ok(None)),
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

    fn known(version: &str) -> ServerVersion {
        ServerVersion::Known(version.into())
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
            Some(&record(service())),
            &Ok(Some(began(BEGAN_AS_SERVICE_SUMMARY, Ok(service())))),
            &Ok(None),
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
        let status = status(None, &Err("bad note".into()), &Ok(None));
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
        let noted = Noted {
            summary: "An earlier `kkt setup` installed a server here in Docker, from \
                      /data/docker, for clients at http://127.0.0.1:8080/mcp, but stopped \
                      before recording it."
                .into(),
            found: Ok(Found {
                deployment: docker_mode(),
                url: Some("http://127.0.0.1:8080/mcp".into()),
            }),
        };
        let status = status(None, &Ok(Some(noted)), &Ok(None));
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
            &Ok(Some(began(BEGAN_IN_DOCKER_SUMMARY, Ok(managed_docker())))),
            &Ok(None),
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
            &Ok(Some(began(BEGAN_AS_SERVICE_SUMMARY, Ok(service())))),
            &Ok(None),
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
            &Ok(Some(began(
                BEGAN_IN_DOCKER_SUMMARY,
                Err("set HOME to keep kkt's Docker files".into()),
            ))),
            &Ok(None),
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
        let found = unrecorded(
            service(),
            "Found a kkt server as a service, for clients at http://127.0.0.1:9000/mcp, not \
             recorded.",
        );
        let status = status(None, &Ok(None), &Ok(Some(found)));
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
        let found = unrecorded(
            docker_mode(),
            "Found a kkt server in Docker from /data/docker, for clients at \
             http://127.0.0.1:9000/mcp, not recorded.",
        );
        let status = status(None, &Ok(None), &Ok(Some(found)));
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
        let status = status(None, &Ok(None), &Err("both found".into()));
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
            running_version(&record(Deployment::Client)),
            Err(client.to_string())
        );
        assert_eq!(
            running_version(&record(docker_mode())),
            Ok(RunningVersion::DockerSettings(PathBuf::from(
                "/data/docker"
            )))
        );
        assert_eq!(
            running_version(&record(service())),
            Ok(RunningVersion::Recorded(ServerVersion::Known(
                "0.5.0".into()
            )))
        );
    }

    #[test]
    fn an_upgrade_to_the_same_version_does_nothing() {
        let docker = record(docker_mode());
        assert_eq!(
            upgrade_plan(&docker, &known("0.6.0"), "0.6.0", false),
            Ok(UpgradePlan {
                replace: None,
                said: "Server is already at kkt 0.6.0".into(),
            })
        );
    }

    #[test]
    fn an_upgrade_replaces_the_server_and_records_the_new_version() {
        let docker = record(docker_mode());
        assert_eq!(
            upgrade_plan(&docker, &known("0.5.0"), "0.6.0", false),
            Ok(UpgradePlan {
                replace: Some(Replace::Docker {
                    folder: PathBuf::from("/data/docker"),
                    key_file: Some(PathBuf::from("/home/k/access-key")),
                }),
                said: "Upgraded the server to kkt 0.6.0".into(),
            })
        );
        let service = record(Deployment::Service { tailscale: true });
        assert_eq!(
            upgrade_plan(&service, &known("0.5.0"), "0.6.0", false),
            Ok(UpgradePlan {
                replace: Some(Replace::Service),
                said: "Upgraded the server to kkt 0.6.0".into(),
            })
        );
    }

    #[test]
    fn an_upgrade_refuses_a_client_and_a_downgrade() {
        assert_eq!(
            upgrade_plan(&record(Deployment::Client), &known("0.5.0"), "0.6.0", false),
            Err(
                "this machine is a client; upgrade the server on the machine where it runs"
                    .to_string()
            )
        );
        assert_eq!(
            upgrade_plan(&record(docker_mode()), &known("0.7.0"), "0.6.0", false),
            Err(
                "refusing to downgrade the server from 0.7.0 to 0.6.0; pass --allow-downgrade \
                 to override"
                    .to_string()
            )
        );
    }

    #[test]
    fn an_upgrade_may_downgrade_when_allowed() {
        assert_eq!(
            upgrade_plan(&record(docker_mode()), &known("0.7.0"), "0.6.0", true),
            Ok(UpgradePlan {
                replace: Some(Replace::Docker {
                    folder: PathBuf::from("/data/docker"),
                    key_file: Some(PathBuf::from("/home/k/access-key")),
                }),
                said: "Upgraded the server to kkt 0.6.0".into(),
            })
        );
    }
}
