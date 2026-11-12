//! Running the server as a background service of the user's login, as a
//! contract: [`Service`], what kkt asks of the service manager, in kkt's
//! terms, with the values it takes and gives ([`Spec`], [`ServiceState`],
//! [`Settings`]) and its errors ([`ServiceError`]). A wrapper crate named for
//! each service manager implements it: kkt-service-launchd on macOS and
//! kkt-service-systemd on Linux. The program picks one by platform, or
//! [`Unsupported`] elsewhere.
//!
//! [`plan`] works out what the service runs and which keys to make, from
//! plain inputs. [`check_bind`] says whether the service's server could
//! listen on its address, from the program's answer. Neither uses a service
//! manager.

mod bind;
mod plan;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use bind::check_bind;
pub use plan::{Plan, PlanError, Request, ServiceSettings, plan};

use kkt_clock::Clock;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

/// How long the service manager waits before starting the server again
/// after it failed.
pub const SERVICE_RESTART_DELAY: Duration = Duration::from_secs(5);
/// How long the service manager waits for the server to stop before it
/// kills it.
pub const SERVICE_STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long installing and starting the service wait. The program passes
/// [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How long `kkt service install` waits after starting the service
    /// before it checks that the server is still running. A server that
    /// fails as it starts, such as when another program has its port, has
    /// exited by then. It must be shorter than the service manager's
    /// restart delay (5 seconds), so the service manager has not yet started
    /// such a server again when the check is made.
    pub service_start_check: Duration,
    /// How long `kkt service install` waits for launchd to remove the old
    /// service after unloading it, trying to load the new one meanwhile.
    /// `launchctl bootout` returns before launchd has finished removing the
    /// service, and a `bootstrap` before it has fails ("Operation already in
    /// progress", which launchctl shows as "Input/output error").
    pub launchd_removal_wait: Duration,
    /// How long to wait between those tries.
    pub launchd_reload_retry: Duration,
}

impl Settings {
    /// A start check after 3 seconds. launchd gets 35 seconds to remove the
    /// old service: the old server has the 30 seconds of the service's stop
    /// timeout to exit, and the rest is a margin for launchd's own work
    /// after that. Loading is tried every 250 milliseconds meanwhile.
    pub const DEFAULT: Settings = Settings {
        service_start_check: Duration::from_secs(3),
        launchd_removal_wait: Duration::from_secs(SERVICE_STOP_TIMEOUT.as_secs() + 5),
        launchd_reload_retry: Duration::from_millis(250),
    };
}
const _: () =
    assert!(Settings::DEFAULT.service_start_check.as_millis() < SERVICE_RESTART_DELAY.as_millis());

/// What the service runs, and what it needs in place first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The absolute path of the program.
    pub program: PathBuf,
    /// Its arguments. Paths among them should be absolute: services start
    /// in a working directory of the manager's choosing.
    pub args: Vec<String>,
    /// Directories the server needs that it won't create itself, such as
    /// the database's.
    pub dirs: Vec<PathBuf>,
}

/// Whether the service is installed, and running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceState {
    /// The service file is in place.
    pub installed: bool,
    /// The server is running under the service manager.
    pub running: bool,
}

/// An answer that comes after waiting, such as for the service manager to
/// load a service, on the clock it is given.
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T, ServiceError>> + 'a>>;

/// What kkt asks of this machine's service manager. Each method that
/// returns lines returns what to tell the user.
pub trait Service {
    /// Whether a service can be installed here at all: [`ServiceError::NoHome`]
    /// without the folder the service manager keeps services in, and
    /// [`ServiceError::Unsupported`] on a system with no service manager kkt
    /// works with. Callers ask this first, so those are the errors they see.
    fn ready(&self) -> Result<(), ServiceError>;

    /// Writes the service file, starts the service, and has it start at
    /// each login. Installing again replaces the file and restarts it, which
    /// may wait on `clock` as `settings` say, such as while launchd removes
    /// the old service; when that still fails, the error is
    /// [`ServiceError::ReloadFailed`].
    fn install<'a>(
        &'a self,
        clock: &'a dyn Clock,
        settings: Settings,
        spec: &'a Spec,
    ) -> Pending<'a, Vec<String>>;

    /// What [`Service::install`] would change, in order, changing nothing.
    /// Questions to the service manager are still asked.
    fn dry_run<'a>(
        &'a self,
        clock: &'a dyn Clock,
        settings: Settings,
        spec: &'a Spec,
    ) -> Pending<'a, Vec<String>>;

    /// Checks that the server the service just started is still running,
    /// after waiting `settings.service_start_check` on `clock`. If it is
    /// not, the service is stopped so it does not start again every
    /// [`SERVICE_RESTART_DELAY`], and the error is
    /// [`ServiceError::DidNotStayUp`]. The service file stays. Call it after
    /// `install`, which reports only that the service manager took the request.
    fn check_started<'a>(&'a self, clock: &'a dyn Clock, settings: Settings) -> Pending<'a, ()>;

    /// The arguments the installed service runs its program with, as they
    /// were given to `install` in [`Spec::args`], read from its service file;
    /// `None` if it is not installed. Changes nothing.
    fn installed_args(&self) -> Result<Option<Vec<String>>, ServiceError>;

    /// Stops the service and removes its file. The database, config and
    /// logs stay.
    fn uninstall(&self) -> Result<Vec<String>, ServiceError>;

    fn start(&self) -> Result<Vec<String>, ServiceError>;

    /// Stops the service until it is started again or the user next logs in.
    fn stop(&self) -> Result<Vec<String>, ServiceError>;

    /// Gives the server the key file `key`, such as the admin key: `make`
    /// writes it unless it is already there, then the service is stopped and
    /// started, since the server reads its keys only when it starts. The
    /// restart runs even for a key already there, so calling this again
    /// finishes an earlier call that made the key but failed to restart.
    /// Returns whether `make` made the key.
    fn add_key(
        &self,
        key: &Path,
        make: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<bool, ServiceError>;

    /// Whether the service is installed and running, for code that acts on
    /// the answer (such as `kkt doctor`). `status` says the same for people.
    fn state(&self) -> Result<ServiceState, ServiceError>;

    /// Whether the service is installed and running.
    fn status(&self) -> Result<Vec<String>, ServiceError>;

    /// Shows the last `lines` lines of the service's log, and keeps showing
    /// new ones if `follow`.
    fn logs(&self, lines: usize, follow: bool) -> Result<Vec<String>, ServiceError>;
}

/// The service on a system with no service manager kkt works with: every
/// question is answered with [`ServiceError::Unsupported`].
pub struct Unsupported;

impl Service for Unsupported {
    fn ready(&self) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn install<'a>(
        &'a self,
        _: &'a dyn Clock,
        _: Settings,
        _: &'a Spec,
    ) -> Pending<'a, Vec<String>> {
        Box::pin(async { Err(ServiceError::Unsupported) })
    }
    fn dry_run<'a>(
        &'a self,
        _: &'a dyn Clock,
        _: Settings,
        _: &'a Spec,
    ) -> Pending<'a, Vec<String>> {
        Box::pin(async { Err(ServiceError::Unsupported) })
    }
    fn check_started<'a>(&'a self, _: &'a dyn Clock, _: Settings) -> Pending<'a, ()> {
        Box::pin(async { Err(ServiceError::Unsupported) })
    }
    fn installed_args(&self) -> Result<Option<Vec<String>>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn uninstall(&self) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn start(&self) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn stop(&self) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn add_key(
        &self,
        _: &Path,
        _: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<bool, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn state(&self) -> Result<ServiceState, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn status(&self) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
    fn logs(&self, _: usize, _: bool) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("kkt service works with launchd on macOS and systemd on Linux, not on this system")]
    Unsupported,
    #[error("no home directory to install the service in: set HOME")]
    NoHome,
    #[error("the service is not installed; run `kkt service install` first")]
    NotInstalled,
    #[error("{value:?} can't go in a service file: {reason}")]
    BadValue { value: String, reason: &'static str },
    #[error("could not {action} {}: {source}", path.display())]
    File {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("{} is not a service file kkt writes: {reason}", path.display())]
    Unreadable { path: PathBuf, reason: &'static str },
    #[error("could not run `{command}`: {source}")]
    Run { command: String, source: io::Error },
    #[error("`{command}` failed: {detail}")]
    Failed { command: String, detail: String },
    /// Loading the new launchd service failed after the old one was
    /// unloaded, so the server is stopped (see `Manager::install`). With
    /// `still_removing_for`, launchd was still removing the old service
    /// when the tries over that long ran out; without it, the old service
    /// was gone, and the failure is the new service's own. Either way the
    /// new service file is in place, and `kkt service start` loads it.
    #[error("{}", reload_failed(command, detail, *tries, *still_removing_for))]
    ReloadFailed {
        command: String,
        detail: String,
        tries: u32,
        still_removing_for: Option<Duration>,
    },
    /// Making a key file failed; the text says how.
    #[error("{0}")]
    Key(String),
    /// The server was not running `checked_after` (the settings'
    /// [`Settings::service_start_check`]) after the service started it (see
    /// `Manager::check_started`). The service was then stopped, unless
    /// `stop_failed` says how stopping it failed.
    #[error("{}", did_not_stay_up(*checked_after, stop_failed.as_deref()))]
    DidNotStayUp {
        checked_after: Duration,
        stop_failed: Option<String>,
    },
}

/// [`ServiceError::DidNotStayUp`]'s text.
fn did_not_stay_up(checked_after: Duration, stop_failed: Option<&str>) -> String {
    let check = checked_after.as_secs();
    let restart = SERVICE_RESTART_DELAY.as_secs();
    let stopped = match stop_failed {
        None => format!(
            "kkt stopped the service so it does not keep starting again every {restart} seconds."
        ),
        Some(e) => format!(
            "Stopping the service failed too: {}. It may keep starting again every {restart} seconds until `kkt service stop` works.",
            e.trim_end_matches('.')
        ),
    };
    format!(
        "kashkeshet started but was no longer running {check} seconds later. {stopped} \
         `kkt service logs` shows why; a common cause is another program using the address \
         in the `bind` setting. The service is still installed and starts again at the next \
         login; once the cause is fixed, run `kkt service start`."
    )
}

/// [`ServiceError::ReloadFailed`]'s text.
fn reload_failed(
    command: &str,
    detail: &str,
    tries: u32,
    still_removing_for: Option<Duration>,
) -> String {
    const STOPPED: &str = "kashkeshet is stopped: the old service was unloaded, and the new one is installed but not loaded.";
    match still_removing_for {
        Some(waited) => format!(
            "`{command}` failed on each of {tries} tries over {} seconds, while launchd was still removing the old service: {detail}. {STOPPED} Once the old service is gone, run `kkt service start` to load the new one, and `kkt service status` to check.",
            waited.as_secs()
        ),
        None => format!(
            "`{command}` failed after launchd removed the old service: {detail}. {STOPPED} Run `kkt service start` to load it, and `kkt service status` to check."
        ),
    }
}

/// A path as service-file text. Paths that aren't valid UTF-8 can't be
/// written into one.
pub fn path_text(path: &Path) -> Result<&str, ServiceError> {
    path.to_str().ok_or_else(|| ServiceError::BadValue {
        value: path.display().to_string(),
        reason: "it is not valid UTF-8",
    })
}

/// Refuses text with control characters (such as newlines), which neither
/// a plist nor a unit file can hold safely.
pub fn plain_text(text: &str) -> Result<&str, ServiceError> {
    if text.chars().any(char::is_control) {
        return Err(ServiceError::BadValue {
            value: text.to_string(),
            reason: "it contains a control character such as a newline",
        });
    }
    Ok(text)
}
