//! Whether `kkt serve` may run on this machine, given how kkt is deployed
//! here.

use super::{Deployment, Recorded};

/// Whether `kkt serve` may start here. `recorded` is this machine's
/// deployment as its setup record says, if it has one. `service_running` is
/// whether the kkt service is running, when that could be found out; it
/// matters only for a machine deployed as a service. `ignore` is
/// `--ignore-setup`, which the service itself passes.
///
/// The error says where this machine's server really is.
pub fn guard(
    recorded: Option<&Recorded>,
    service_running: Option<bool>,
    ignore: bool,
) -> Result<(), String> {
    let Some(recorded) = recorded.filter(|_| !ignore) else {
        return Ok(());
    };
    match &recorded.deployment {
        Deployment::Service { .. } if service_running == Some(true) => Err(
            "kkt already runs on this machine as a service (see `kkt status`); \
             stop it with `kkt service stop`, or pass --ignore-setup to start another server"
                .to_string(),
        ),
        Deployment::Service { .. } => Ok(()),
        Deployment::Docker { folder, .. } => Err(format!(
            "this machine runs kkt in Docker, from {} (see `kkt status`); \
             pass --ignore-setup to start another server",
            folder.display()
        )),
        Deployment::Client => Err(format!(
            "this machine is set up as a client of the kkt server at {}; \
             run `kkt setup` to change that, or pass --ignore-setup to start a server anyway",
            recorded.url
        )),
    }
}
