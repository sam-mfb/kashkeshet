//! Whether `kkt serve` may run on this machine, given how kkt is deployed
//! here.

use super::{Deployment, RecordedDeployment};
use std::path::PathBuf;

/// Why `kkt serve` may not start here: where this machine's server really
/// is.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    /// The kkt service is running on this machine.
    #[error(
        "kkt already runs on this machine as a service (see `kkt status`); stop it with `kkt service stop`, or pass --ignore-setup to start another server"
    )]
    RunningService,
    /// This machine runs kkt in Docker, from `folder`.
    #[error(
        "this machine runs kkt in Docker, from {} (see `kkt status`); pass --ignore-setup to start another server",
        folder.display()
    )]
    InDocker { folder: PathBuf },
    /// This machine is a client of the server at `url`.
    #[error(
        "this machine is set up as a client of the kkt server at {url}; run `kkt setup` to change that, or pass --ignore-setup to start a server anyway"
    )]
    Client { url: String },
}

/// Whether `kkt serve` may start here. `recorded` is this machine's
/// deployment as its setup record says, if it has one. `service_running` is
/// whether the kkt service is running, when that could be found out; it
/// matters only for a machine deployed as a service. `ignore` is
/// `--ignore-setup`, which the service itself passes.
///
/// The error says where this machine's server really is.
pub fn guard(
    recorded: Option<&RecordedDeployment>,
    service_running: Option<bool>,
    ignore: bool,
) -> Result<(), GuardError> {
    let Some(recorded) = recorded.filter(|_| !ignore) else {
        return Ok(());
    };
    match &recorded.deployment {
        Deployment::Service { .. } if service_running == Some(true) => {
            Err(GuardError::RunningService)
        }
        Deployment::Service { .. } => Ok(()),
        Deployment::Docker { folder, .. } => Err(GuardError::InDocker {
            folder: folder.clone(),
        }),
        Deployment::Client => Err(GuardError::Client {
            url: recorded.url.clone(),
        }),
    }
}
