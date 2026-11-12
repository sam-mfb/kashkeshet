//! kkt's service as a systemd user unit on Linux: `Systemd` implements
//! kkt-service's `Service` with the user's systemd, writing the unit file
//! and running `systemctl` and `journalctl`. Everything it decides is
//! worked through a private system, so its tests use a fake and never start a
//! real service; the real system is behind `real`.

#![cfg(any(feature = "real", test))]

mod system;
#[cfg(test)]
mod tests;
mod unit;

#[cfg(feature = "real")]
pub use real::Systemd;

#[cfg(feature = "real")]
mod real {
    use crate::system::OsSystem;
    use crate::unit::Unit;
    use kkt_clock::Clock;
    use kkt_service::{Pending, Service, ServiceError, ServiceState, Settings, Spec};
    use std::path::Path;

    /// The systemd user unit of the user whose config base
    /// (`$XDG_CONFIG_HOME`, or `~/.config`) it is given. Without one there is
    /// nowhere to install it: every question is answered with
    /// [`ServiceError::NoHome`].
    pub struct Systemd {
        unit: Option<Unit>,
    }

    impl Systemd {
        pub fn new(config_home: Option<&Path>) -> Systemd {
            Systemd {
                unit: config_home.map(Unit::new),
            }
        }

        fn unit(&self) -> Result<&Unit, ServiceError> {
            self.unit.as_ref().ok_or(ServiceError::NoHome)
        }
    }

    impl Service for Systemd {
        fn ready(&self) -> Result<(), ServiceError> {
            self.unit().map(drop)
        }
        fn install<'a>(
            &'a self,
            _: &'a dyn Clock,
            _: Settings,
            spec: &'a Spec,
        ) -> Pending<'a, Vec<String>> {
            Box::pin(async move { self.unit()?.install(&OsSystem, spec) })
        }
        fn dry_run<'a>(
            &'a self,
            _: &'a dyn Clock,
            _: Settings,
            spec: &'a Spec,
        ) -> Pending<'a, Vec<String>> {
            Box::pin(async move { self.unit()?.dry_run(&OsSystem, spec) })
        }
        fn check_started<'a>(
            &'a self,
            clock: &'a dyn Clock,
            settings: Settings,
        ) -> Pending<'a, ()> {
            Box::pin(async move { self.unit()?.check_started(&OsSystem, clock, settings).await })
        }
        fn installed_args(&self) -> Result<Option<Vec<String>>, ServiceError> {
            self.unit()?.installed_args(&OsSystem)
        }
        fn uninstall(&self) -> Result<Vec<String>, ServiceError> {
            self.unit()?.uninstall(&OsSystem)
        }
        fn start(&self) -> Result<Vec<String>, ServiceError> {
            self.unit()?.start(&OsSystem)
        }
        fn stop(&self) -> Result<Vec<String>, ServiceError> {
            self.unit()?.stop(&OsSystem)
        }
        fn add_key(
            &self,
            key: &Path,
            make: &dyn Fn(&Path) -> Result<(), String>,
        ) -> Result<bool, ServiceError> {
            self.unit()?.add_key(&OsSystem, key, make)
        }
        fn state(&self) -> Result<ServiceState, ServiceError> {
            self.unit()?.state(&OsSystem)
        }
        fn status(&self) -> Result<Vec<String>, ServiceError> {
            self.unit()?.status(&OsSystem)
        }
        fn logs(&self, lines: usize, follow: bool) -> Result<Vec<String>, ServiceError> {
            self.unit()?.logs(&OsSystem, lines, follow)
        }
    }
}
