//! kkt-deployment's facts, read from the setup record: how kkt is deployed
//! here, as the record, the note of an unfinished setup, or a server found
//! installed with no record says.

use kkt_deployment::connection::{RecordedServer, ServerAddress};
use kkt_deployment::{
    Deployment, DeploymentEndpoint, RecordedDeployment, ServerVersion, UnrecordedDeployment,
};
use kkt_machine::{Env, Platform};
use kkt_setup as setup;

/// The deployment a record's `mode` names.
pub fn deployment(mode: &setup::Mode) -> Deployment {
    match mode {
        setup::Mode::Service { tailscale } => Deployment::Service {
            tailscale: *tailscale,
        },
        setup::Mode::Docker(docker) => Deployment::Docker {
            folder: docker.folder.clone(),
            tailscale: docker.tailscale,
            hostname: docker.hostname.clone(),
            tailnet: docker.tailnet.clone(),
        },
        setup::Mode::Client => Deployment::Client,
    }
}

/// The version of kkt a record or a settings file names: not known when it
/// says so ([`setup::version_not_known`]).
pub fn server_version(version: &str) -> ServerVersion {
    match setup::version_not_known(version) {
        true => ServerVersion::NotKnown,
        false => ServerVersion::Known(version.to_string()),
    }
}

/// This machine's deployment, as `record` says.
pub fn recorded(record: &setup::Record) -> RecordedDeployment {
    RecordedDeployment {
        deployment: deployment(&record.mode),
        url: record.url.clone(),
        key_file: record.key_file.clone(),
        clients_url: record.clients.as_ref().map(|clients| clients.url.clone()),
        version: server_version(&record.written_by),
    }
}

/// The servers `record` names, for how a client reaches the server: the
/// one set up here (or used, on a client machine), and the one its clients
/// were switched to, if they were.
pub fn recorded_server(record: &setup::Record) -> RecordedServer {
    let address = |server: setup::Server| ServerAddress {
        url: server.url,
        key_file: server.key_file,
    };
    RecordedServer {
        server: address(record.own_server()),
        switched_to: record.clients.clone().map(address),
    }
}

/// The note of an unfinished setup, for `kkt status`: from its record when
/// the install finished, otherwise from where it puts the server, in the
/// folder `kkt setup` keeps kkt's Docker files in.
pub fn setup_not_finished(
    unfinished: &Result<Option<setup::Unfinished>, String>,
    env: &Env,
    platform: Platform,
) -> Result<Option<UnrecordedDeployment>, String> {
    let Some(unfinished) = unfinished.as_ref().map_err(Clone::clone)? else {
        return Ok(None);
    };
    let found = match &unfinished.record {
        Some(record) => Ok(DeploymentEndpoint {
            deployment: deployment(&record.mode),
            url: Some(record.url.clone()),
        }),
        None => unfinished
            .mode(kkt_deployment::docker_folder(env, platform))
            .map(|mode| DeploymentEndpoint {
                deployment: deployment(&mode),
                url: None,
            }),
    };
    Ok(Some(UnrecordedDeployment {
        summary: unfinished.summary(),
        deployment: found,
    }))
}

/// A server found installed with no record, for `kkt status`.
pub fn found_without_record(
    unrecorded: &Result<Option<setup::Unrecorded>, String>,
) -> Result<Option<UnrecordedDeployment>, String> {
    Ok(unrecorded
        .as_ref()
        .map_err(Clone::clone)?
        .as_ref()
        .map(|found| UnrecordedDeployment {
            summary: found.summary(),
            deployment: Ok(DeploymentEndpoint {
                deployment: deployment(&found.mode),
                url: Some(found.url.clone()),
            }),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn docker() -> setup::Mode {
        setup::Mode::Docker(setup::DockerSetup {
            folder: PathBuf::from("/data/docker"),
            tailscale: true,
            hostname: Some("kkt-1".into()),
            tailnet: Some("tail1234.ts.net".into()),
        })
    }

    /// `kkt status` names a deployment as the record names its mode.
    #[test]
    fn each_mode_is_the_deployment_of_the_same_name() {
        for mode in [
            setup::Mode::Service { tailscale: true },
            docker(),
            setup::Mode::Client,
        ] {
            assert_eq!(deployment(&mode).name(), mode.name());
        }
        assert_eq!(
            deployment(&docker()),
            Deployment::Docker {
                folder: PathBuf::from("/data/docker"),
                tailscale: true,
                hostname: Some("kkt-1".into()),
                tailnet: Some("tail1234.ts.net".into()),
            }
        );
    }

    #[test]
    fn a_version_marked_not_known_is_not_known() {
        assert_eq!(server_version("unknown"), ServerVersion::NotKnown);
        assert_eq!(
            server_version("0.5.0"),
            ServerVersion::Known("0.5.0".into())
        );
    }

    #[test]
    fn a_record_is_its_deployment_server_and_version() {
        let record = setup::Record {
            mode: setup::Mode::Service { tailscale: false },
            written_by: "0.5.0".into(),
            url: "http://127.0.0.1:7420/mcp".into(),
            key_file: Some(PathBuf::from("/k")),
            clients: Some(setup::Server {
                url: "https://other/mcp".into(),
                key_file: None,
            }),
        };
        assert_eq!(
            recorded(&record),
            RecordedDeployment {
                deployment: Deployment::Service { tailscale: false },
                url: "http://127.0.0.1:7420/mcp".into(),
                key_file: Some(PathBuf::from("/k")),
                clients_url: Some("https://other/mcp".into()),
                version: ServerVersion::Known("0.5.0".into()),
            }
        );
    }
}
