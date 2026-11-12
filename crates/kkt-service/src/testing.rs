//! A service manager in memory, for consumers' tests: it keeps one service's
//! arguments and whether it is installed and running, writes down what it
//! is asked to change, and never touches the machine.

use crate::{Pending, Service, ServiceError, ServiceState, Settings, Spec};
use kkt_clock::Clock;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One service, held in memory.
#[derive(Default)]
pub struct Memory {
    args: RefCell<Option<Vec<String>>>,
    running: RefCell<bool>,
    keys: RefCell<BTreeSet<PathBuf>>,
    done: RefCell<Vec<String>>,
}

impl Memory {
    /// A machine with no service installed.
    pub fn new() -> Self {
        Self::default()
    }

    /// A machine whose service runs its program with `args`, and is running
    /// or not.
    pub fn installed(args: Vec<String>, running: bool) -> Self {
        Self {
            args: RefCell::new(Some(args)),
            running: RefCell::new(running),
            keys: RefCell::new(BTreeSet::new()),
            done: RefCell::new(Vec::new()),
        }
    }

    /// What it was asked to change, in order, such as `"uninstall"`.
    pub fn done(&self) -> Vec<String> {
        self.done.borrow().clone()
    }

    fn note(&self, what: &str) {
        self.done.borrow_mut().push(what.to_string());
    }

    fn is_installed(&self) -> bool {
        self.args.borrow().is_some()
    }
}

impl Service for Memory {
    fn ready(&self) -> Result<(), ServiceError> {
        Ok(())
    }

    fn install<'a>(
        &'a self,
        _: &'a dyn Clock,
        _: Settings,
        spec: &'a Spec,
    ) -> Pending<'a, Vec<String>> {
        Box::pin(async move {
            self.note("install");
            *self.args.borrow_mut() = Some(spec.args.clone());
            *self.running.borrow_mut() = true;
            Ok(vec!["Installed the service.".to_string()])
        })
    }

    fn dry_run<'a>(
        &'a self,
        _: &'a dyn Clock,
        _: Settings,
        spec: &'a Spec,
    ) -> Pending<'a, Vec<String>> {
        Box::pin(async move { Ok(vec![format!("Would install {}", spec.program.display())]) })
    }

    fn check_started<'a>(&'a self, _: &'a dyn Clock, settings: Settings) -> Pending<'a, ()> {
        Box::pin(async move {
            if *self.running.borrow() {
                Ok(())
            } else {
                Err(ServiceError::DidNotStayUp {
                    checked_after: settings.service_start_check,
                    stop_failed: None,
                })
            }
        })
    }

    fn installed_args(&self) -> Result<Option<Vec<String>>, ServiceError> {
        Ok(self.args.borrow().clone())
    }

    fn uninstall(&self) -> Result<Vec<String>, ServiceError> {
        if !self.is_installed() {
            return Ok(vec!["The service was not installed.".to_string()]);
        }
        self.note("uninstall");
        *self.args.borrow_mut() = None;
        *self.running.borrow_mut() = false;
        Ok(vec!["Removed the service.".to_string()])
    }

    fn start(&self) -> Result<Vec<String>, ServiceError> {
        if !self.is_installed() {
            return Err(ServiceError::NotInstalled);
        }
        self.note("start");
        *self.running.borrow_mut() = true;
        Ok(vec!["Started kashkeshet.".to_string()])
    }

    fn stop(&self) -> Result<Vec<String>, ServiceError> {
        if !self.is_installed() {
            return Err(ServiceError::NotInstalled);
        }
        self.note("stop");
        *self.running.borrow_mut() = false;
        Ok(vec!["Stopped kashkeshet.".to_string()])
    }

    fn add_key(
        &self,
        key: &Path,
        make: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<bool, ServiceError> {
        // A key it gave the server before is not made again, but the
        // restart runs, as a real service manager's does.
        let made = !self.keys.borrow().contains(key);
        if made {
            make(key).map_err(ServiceError::Key)?;
            self.keys.borrow_mut().insert(key.to_path_buf());
        }
        self.stop()?;
        self.start()?;
        Ok(made)
    }

    fn state(&self) -> Result<ServiceState, ServiceError> {
        Ok(ServiceState {
            installed: self.is_installed(),
            running: *self.running.borrow(),
        })
    }

    fn status(&self) -> Result<Vec<String>, ServiceError> {
        let state = self.state()?;
        Ok(vec![format!(
            "installed: {}, running: {}",
            state.installed, state.running
        )])
    }

    fn logs(&self, _: usize, _: bool) -> Result<Vec<String>, ServiceError> {
        if !self.is_installed() {
            return Err(ServiceError::NotInstalled);
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_service_is_stopped_started_and_removed() {
        let service = Memory::installed(vec!["serve".into()], true);
        assert_eq!(
            service.state().expect("state"),
            ServiceState {
                installed: true,
                running: true
            }
        );
        assert_eq!(
            service.installed_args().expect("args"),
            Some(vec!["serve".to_string()])
        );
        service.stop().expect("stop");
        assert!(!service.state().expect("state").running);
        service.start().expect("start");
        service.uninstall().expect("uninstall");
        assert_eq!(service.done(), ["stop", "start", "uninstall"]);
        assert!(!service.state().expect("state").installed);
        assert!(matches!(service.start(), Err(ServiceError::NotInstalled)));
    }

    #[test]
    fn a_key_is_made_once_and_the_service_restarted_each_time() {
        let service = Memory::installed(vec!["serve".into()], true);
        let made = std::cell::Cell::new(0);
        let make = |_: &Path| {
            made.set(made.get() + 1);
            Ok(())
        };
        let key = Path::new("/k/admin-key");
        assert!(service.add_key(key, &make).expect("added"));
        assert!(!service.add_key(key, &make).expect("added again"));
        assert_eq!(made.get(), 1);
        assert_eq!(service.done(), ["stop", "start", "stop", "start"]);
    }
}
