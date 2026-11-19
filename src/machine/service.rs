//! This machine's service manager, as kkt-bare-service's `Service`: launchd on
//! macOS, systemd on Linux, and none kkt works with elsewhere. Each is its
//! own wrapper crate; the program only picks one and injects it.

use kkt_bare_service::Service;
use kkt_machine::Env;

/// The service manager of the system this binary was built for: launchd
/// under the home folder on macOS, systemd under the config base on Linux.
pub(crate) fn for_this_os(env: &Env) -> Box<dyn Service> {
    if cfg!(target_os = "macos") {
        Box::new(kkt_bare_service_launchd::Launchd::new(
            env.home.as_deref(),
            kkt_bare_service_launchd::Settings::DEFAULT,
        ))
    } else if cfg!(target_os = "linux") {
        Box::new(kkt_bare_service_systemd::Systemd::new(
            env.config_home().as_deref(),
        ))
    } else {
        Box::new(kkt_bare_service::Unsupported::new(UNSUPPORTED.to_string()))
    }
}

/// What `kkt service` says on a system with no service manager it works
/// with.
const UNSUPPORTED: &str =
    "kkt service works with launchd on macOS and systemd on Linux, not on this system";

#[cfg(test)]
mod tests;
