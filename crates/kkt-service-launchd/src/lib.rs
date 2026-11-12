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

#[cfg(feature = "real")]
mod real {
    use crate::agent::Agent;
    use crate::system::OsSystem;
    use kkt_clock::Clock;
    use kkt_service::{Pending, Service, ServiceError, ServiceState, Settings, Spec};
    use std::path::Path;

    /// The launchd user agent of the user whose home folder it is given.
    /// Without a home folder there is nowhere to install it: every question
    /// is answered with [`ServiceError::NoHome`].
    pub struct Launchd {
        agent: Option<Agent>,
    }

    impl Launchd {
        pub fn new(home: Option<&Path>) -> Launchd {
            Launchd {
                agent: home.map(Agent::new),
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
            settings: Settings,
            spec: &'a Spec,
        ) -> Pending<'a, Vec<String>> {
            Box::pin(async move {
                self.agent()?
                    .install(&OsSystem, clock, settings, spec)
                    .await
            })
        }
        fn dry_run<'a>(
            &'a self,
            clock: &'a dyn Clock,
            settings: Settings,
            spec: &'a Spec,
        ) -> Pending<'a, Vec<String>> {
            Box::pin(async move {
                self.agent()?
                    .dry_run(&OsSystem, clock, settings, spec)
                    .await
            })
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
