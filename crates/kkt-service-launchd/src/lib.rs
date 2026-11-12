//! kkt's service as a launchd user agent on macOS: `Launchd` implements
//! kkt-service's `Service` over the user's `gui/<uid>` domain, writing the
//! plist and running `launchctl`, `tail` and `id`. Everything it decides is
//! worked through a private system, so its tests use a fake and never start a
//! real service; the real system is behind `real`.

#![cfg(any(feature = "real", test))]

mod agent;
mod system;
#[cfg(test)]
mod tests;

#[cfg(feature = "real")]
pub use real::Launchd;

use std::time::Duration;

/// How long installing the service again waits for launchd to let the new
/// one load. The program passes [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How long to wait for launchd to remove the old service after
    /// unloading it, trying to load the new one meanwhile. `launchctl
    /// bootout` returns before launchd has finished removing the service,
    /// and a `bootstrap` before it has fails ("Operation already in
    /// progress", which launchctl shows as "Input/output error").
    pub removal_wait: Duration,
    /// How long to wait between those tries.
    pub reload_retry: Duration,
}

impl Settings {
    /// launchd gets 35 seconds to remove the old service: the old server has
    /// the 30 seconds of the service's stop timeout to exit, and the rest is
    /// a margin for launchd's own work after that. Loading is tried every
    /// 250 milliseconds meanwhile.
    pub const DEFAULT: Settings = Settings {
        removal_wait: Duration::from_secs(kkt_service::SERVICE_STOP_TIMEOUT.as_secs() + 5),
        reload_retry: Duration::from_millis(250),
    };
}

#[cfg(feature = "real")]
mod real {
    use crate::agent::Agent;
    use crate::system::OsSystem;
    use kkt_clock::Clock;
    use kkt_service::{Pending, Service, ServiceError, ServiceState, Settings, Spec};
    use std::path::Path;

    /// The launchd user agent of the user whose home folder it is given,
    /// waiting for launchd as `settings` say.
    /// Without a home folder there is nowhere to install it: every question
    /// is answered with [`ServiceError::NoHome`].
    pub struct Launchd {
        agent: Option<Agent>,
    }

    impl Launchd {
        pub fn new(home: Option<&Path>, settings: crate::Settings) -> Launchd {
            Launchd {
                agent: home.map(|home| Agent::new(home, settings)),
            }
        }

        fn agent(&self) -> Result<&Agent, ServiceError> {
            self.agent.as_ref().ok_or(ServiceError::NoHome)
        }
    }

    impl Service for Launchd {
        fn ready(&self) -> Result<(), ServiceError> {
            self.agent().map(drop)
        }
        fn install<'a>(
            &'a self,
            clock: &'a dyn Clock,
            _: Settings,
            spec: &'a Spec,
        ) -> Pending<'a, Vec<String>> {
            Box::pin(async move { self.agent()?.install(&OsSystem, clock, spec).await })
        }
        fn dry_run<'a>(
            &'a self,
            clock: &'a dyn Clock,
            _: Settings,
            spec: &'a Spec,
        ) -> Pending<'a, Vec<String>> {
            Box::pin(async move { self.agent()?.dry_run(&OsSystem, clock, spec).await })
        }
        fn check_started<'a>(
            &'a self,
            clock: &'a dyn Clock,
            settings: Settings,
        ) -> Pending<'a, ()> {
            Box::pin(async move {
                self.agent()?
                    .check_started(&OsSystem, clock, settings)
                    .await
            })
        }
        fn installed_args(&self) -> Result<Option<Vec<String>>, ServiceError> {
            self.agent()?.installed_args(&OsSystem)
        }
        fn uninstall(&self) -> Result<Vec<String>, ServiceError> {
            self.agent()?.uninstall(&OsSystem)
        }
        fn start(&self) -> Result<Vec<String>, ServiceError> {
            self.agent()?.start(&OsSystem)
        }
        fn stop(&self) -> Result<Vec<String>, ServiceError> {
            self.agent()?.stop(&OsSystem)
        }
        fn add_key(
            &self,
            key: &Path,
            make: &dyn Fn(&Path) -> Result<(), String>,
        ) -> Result<bool, ServiceError> {
            self.agent()?.add_key(&OsSystem, key, make)
        }
        fn state(&self) -> Result<ServiceState, ServiceError> {
            self.agent()?.state(&OsSystem)
        }
        fn status(&self) -> Result<Vec<String>, ServiceError> {
            self.agent()?.status(&OsSystem)
        }
        fn logs(&self, lines: usize, follow: bool) -> Result<Vec<String>, ServiceError> {
            self.agent()?.logs(&OsSystem, lines, follow)
        }
    }
}
