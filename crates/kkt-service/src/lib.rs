//! Running the server as a background service of the user's login: a
//! launchd user agent on macOS, a systemd user unit on Linux.
//!
//! The service files are written here rather than through a library. They
//! are short; the restart policy we want (start again only after a failure)
//! can't be expressed through the cross-platform crates for launchd; and
//! `status` needs code for each platform anyway.
//!
//! Everything that touches the system (files, `launchctl`, `systemctl`,
//! `journalctl`, `tail`, `id`) goes through [`System`], so tests use a fake
//! and never start a real service. The program adapts the real system;
//! [`DryRun`] reports what would change without changing anything.
//!
//! [`plan`] works out what the service runs and which keys to make, from
//! plain inputs.

mod dry_run;
mod launchd;
mod plan;
mod systemd;
#[cfg(test)]
mod tests;

pub use dry_run::DryRun;
pub use plan::{Plan, PlanError, Request, plan};

use kkt_clock::Clock;
use kkt_machine::Env;
use launchd::Launchd;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use systemd::Systemd;

/// How long the service manager waits before starting the server again
/// after it failed.
const SERVICE_RESTART_DELAY: Duration = Duration::from_secs(5);
/// How long the service manager waits for the server to stop before it
/// kills it.
const SERVICE_STOP_TIMEOUT: Duration = Duration::from_secs(30);

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

/// The service manager of one platform.
pub enum Manager {
    Launchd(Launchd),
    Systemd(Systemd),
}

/// What the service needs from the system; the program adapts its real one.
pub trait System {
    /// The user's numeric ID, which names their launchd domain.
    fn user_id(&self) -> io::Result<u32>;
    /// Whether `path` is there: `Ok(false)` only when it is not, and an
    /// error when that can't be told, such as when a folder above it can't
    /// be read.
    fn exists(&self, path: &Path) -> io::Result<bool>;
    fn read_file(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Runs `command` and collects its output.
    fn run(&self, command: &Command) -> io::Result<Finished>;
    /// Runs `command` with its output going straight to the terminal, as
    /// for a log someone is following. The returned output is empty.
    fn run_attached(&self, command: &Command) -> io::Result<Finished>;
}

/// A program to run and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub program: &'static str,
    pub args: Vec<String>,
    /// True if running it only asks a question and changes nothing, so a
    /// dry run may still run it.
    pub query: bool,
}

impl Command {
    /// A command that changes something.
    fn new(program: &'static str, args: &[&str]) -> Command {
        Command {
            program,
            args: args.iter().map(|a| a.to_string()).collect(),
            query: false,
        }
    }

    /// A command that only asks a question.
    fn query(program: &'static str, args: &[&str]) -> Command {
        Command {
            query: true,
            ..Command::new(program, args)
        }
    }
}

/// Shown the way a person would type it, for messages.
impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '"') {
                write!(f, " {arg:?}")?;
            } else {
                write!(f, " {arg}")?;
            }
        }
        Ok(())
    }
}

/// How a command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Finished {
    /// The exit status, or `None` if a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Finished {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Whether the command said `text`, in either stream.
    fn said(&self, text: &str) -> bool {
        self.stdout.contains(text) || self.stderr.contains(text)
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

impl Manager {
    /// The service manager of the system this binary was built for.
    pub fn for_this_os(env: &Env) -> Result<Manager, ServiceError> {
        if cfg!(target_os = "macos") {
            let home = env.home.as_ref().ok_or(ServiceError::NoHome)?;
            Ok(Manager::launchd(home))
        } else if cfg!(target_os = "linux") {
            let config_home = env.config_home().ok_or(ServiceError::NoHome)?;
            Ok(Manager::systemd(&config_home))
        } else {
            Err(ServiceError::Unsupported)
        }
    }

    /// launchd, for the user whose home directory is `home`.
    pub fn launchd(home: &Path) -> Manager {
        Manager::Launchd(Launchd::new(home))
    }

    /// systemd, for the user whose config base (`$XDG_CONFIG_HOME`) is
    /// `config_home`.
    pub fn systemd(config_home: &Path) -> Manager {
        Manager::Systemd(Systemd::new(config_home))
    }

    /// The service file this manager writes for `spec`.
    pub fn render(&self, spec: &Spec) -> Result<String, ServiceError> {
        match self {
            Manager::Launchd(m) => m.render(spec),
            Manager::Systemd(m) => m.render(spec),
        }
    }

    /// Writes the service file, starts the service, and has it start at
    /// each login. Installing again replaces the file and restarts it. On
    /// launchd, that restart may wait on `clock`: launchd removes the old
    /// service after unloading it, and the new one cannot load until it
    /// has, so loading is tried again while the old service is still there
    /// (#369), as `settings` say; when it still fails, the error is
    /// [`ServiceError::ReloadFailed`]. Returns what to tell the user.
    pub async fn install(
        &self,
        system: &dyn System,
        clock: &dyn Clock,
        settings: Settings,
        spec: &Spec,
    ) -> Result<Vec<String>, ServiceError> {
        for dir in &spec.dirs {
            create_dir(system, dir)?;
        }
        match self {
            Manager::Launchd(m) => m.install(system, clock, settings, spec).await,
            Manager::Systemd(m) => m.install(system, spec),
        }
    }

    /// Checks that the server the service just started is still running,
    /// after waiting `settings.service_start_check` on `clock`. A server that
    /// fails as it starts, such as when another program has its port, has
    /// exited by then, and the service manager would start it again every
    /// [`SERVICE_RESTART_DELAY`], forever. So then the service is stopped,
    /// and the error is [`ServiceError::DidNotStayUp`]. The service file
    /// stays. Call it after `install`, which reports only that the service
    /// manager took the request.
    pub async fn check_started(
        &self,
        system: &dyn System,
        clock: &dyn Clock,
        settings: Settings,
    ) -> Result<(), ServiceError> {
        clock
            .sleep_until(clock.now() + settings.service_start_check)
            .await;
        if self.state(system)?.running {
            return Ok(());
        }
        let stopped = match self {
            Manager::Launchd(m) => m.unload(system),
            Manager::Systemd(m) => m.stop(system).map(drop),
        };
        Err(ServiceError::DidNotStayUp {
            checked_after: settings.service_start_check,
            stop_failed: stopped.err().map(|e| e.to_string()),
        })
    }

    /// The arguments the installed service runs its program with, as they
    /// were given to `install` in [`Spec::args`] (without what the manager
    /// adds itself), read from its service file; `None` if it is not
    /// installed. Changes nothing.
    pub fn installed_args(&self, system: &dyn System) -> Result<Option<Vec<String>>, ServiceError> {
        match self {
            Manager::Launchd(m) => m.installed_args(system),
            Manager::Systemd(m) => m.installed_args(system),
        }
    }

    /// Stops the service and removes its file. The database, config and
    /// logs stay.
    pub fn uninstall(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        match self {
            Manager::Launchd(m) => m.uninstall(system),
            Manager::Systemd(m) => m.uninstall(system),
        }
    }

    pub fn start(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        match self {
            Manager::Launchd(m) => m.start(system),
            Manager::Systemd(m) => m.start(system),
        }
    }

    /// Stops the service until it is started again or the user next logs in.
    pub fn stop(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        match self {
            Manager::Launchd(m) => m.stop(system),
            Manager::Systemd(m) => m.stop(system),
        }
    }

    /// Gives the server the key file `key`, such as the admin key: `make`
    /// writes it unless it is already there, then the service is stopped and
    /// started, since the server reads its keys only when it starts. The
    /// restart runs even for a key already there, so calling this again
    /// finishes an earlier call that made the key but failed to restart.
    /// Returns whether `make` made the key.
    pub fn add_key(
        &self,
        system: &dyn System,
        key: &Path,
        make: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<bool, ServiceError> {
        let made = !exists(system, key)?;
        if made {
            make(key).map_err(ServiceError::Key)?;
        }
        self.stop(system)?;
        self.start(system)?;
        Ok(made)
    }

    /// Whether the service is installed and running, for code that acts on
    /// the answer (such as `kkt doctor`). `status` says the same for people.
    pub fn state(&self, system: &dyn System) -> Result<ServiceState, ServiceError> {
        match self {
            Manager::Launchd(m) => m.state(system),
            Manager::Systemd(m) => m.state(system),
        }
    }

    /// Whether the service is installed and running.
    pub fn status(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        match self {
            Manager::Launchd(m) => m.status(system),
            Manager::Systemd(m) => m.status(system),
        }
    }

    /// Shows the last `lines` lines of the service's log, and keeps showing
    /// new ones if `follow`.
    pub fn logs(
        &self,
        system: &dyn System,
        lines: usize,
        follow: bool,
    ) -> Result<Vec<String>, ServiceError> {
        match self {
            Manager::Launchd(m) => m.logs(system, lines, follow),
            Manager::Systemd(m) => m.logs(system, lines, follow),
        }
    }
}

fn create_dir(system: &dyn System, dir: &Path) -> Result<(), ServiceError> {
    system
        .create_dir_all(dir)
        .map_err(|source| ServiceError::File {
            action: "create",
            path: dir.to_path_buf(),
            source,
        })
}

/// Whether `path` is there. When that can't be told, the error names the
/// path and the cause, rather than taking it as missing.
fn exists(system: &dyn System, path: &Path) -> Result<bool, ServiceError> {
    system.exists(path).map_err(|source| ServiceError::File {
        action: "check",
        path: path.to_path_buf(),
        source,
    })
}

/// The service file at `path`, or `None` if it is not there.
fn read_service_file(system: &dyn System, path: &Path) -> Result<Option<String>, ServiceError> {
    if !exists(system, path)? {
        return Ok(None);
    }
    system
        .read_file(path)
        .map(Some)
        .map_err(|source| ServiceError::File {
            action: "read",
            path: path.to_path_buf(),
            source,
        })
}

fn write_file(system: &dyn System, path: &Path, contents: &str) -> Result<(), ServiceError> {
    system
        .write_file(path, contents)
        .map_err(|source| ServiceError::File {
            action: "write",
            path: path.to_path_buf(),
            source,
        })
}

/// Removes `path` if it is there.
fn remove_file(system: &dyn System, path: &Path) -> Result<bool, ServiceError> {
    if !exists(system, path)? {
        return Ok(false);
    }
    system
        .remove_file(path)
        .map_err(|source| ServiceError::File {
            action: "remove",
            path: path.to_path_buf(),
            source,
        })?;
    Ok(true)
}

/// Runs `command` and returns how it ended, failed or not.
fn run(system: &dyn System, command: &Command) -> Result<Finished, ServiceError> {
    system.run(command).map_err(|source| ServiceError::Run {
        command: command.to_string(),
        source,
    })
}

/// Runs `command`, which must succeed.
fn run_ok(system: &dyn System, command: &Command) -> Result<Finished, ServiceError> {
    let finished = run(system, command)?;
    if finished.success() {
        Ok(finished)
    } else {
        Err(ServiceError::Failed {
            command: command.to_string(),
            detail: failure_detail(&finished),
        })
    }
}

/// Runs `command` attached to the terminal. It must succeed, or exit with
/// one of `answers`: statuses that are answers rather than failures, such
/// as systemctl's for a unit that isn't running.
fn run_attached(
    system: &dyn System,
    command: &Command,
    answers: &[i32],
) -> Result<Finished, ServiceError> {
    let finished = system
        .run_attached(command)
        .map_err(|source| ServiceError::Run {
            command: command.to_string(),
            source,
        })?;
    match finished.code {
        Some(0) => Ok(finished),
        Some(code) if answers.contains(&code) => Ok(finished),
        _ => Err(ServiceError::Failed {
            command: command.to_string(),
            detail: failure_detail(&finished),
        }),
    }
}

/// What a failed command said, for an error message.
fn failure_detail(finished: &Finished) -> String {
    let said = [finished.stderr.trim(), finished.stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty());
    match (said, finished.code) {
        (Some(said), _) => said.to_string(),
        (None, Some(code)) => format!("exit status {code}"),
        (None, None) => "ended by a signal".to_string(),
    }
}

/// A path as service-file text. Paths that aren't valid UTF-8 can't be
/// written into one.
fn path_text(path: &Path) -> Result<&str, ServiceError> {
    path.to_str().ok_or_else(|| ServiceError::BadValue {
        value: path.display().to_string(),
        reason: "it is not valid UTF-8",
    })
}

/// Refuses text with control characters (such as newlines), which neither
/// a plist nor a unit file can hold safely.
fn plain_text(text: &str) -> Result<&str, ServiceError> {
    if text.chars().any(char::is_control) {
        return Err(ServiceError::BadValue {
            value: text.to_string(),
            reason: "it contains a control character such as a newline",
        });
    }
    Ok(text)
}
