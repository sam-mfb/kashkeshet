//! The files and commands this wrapper works the service manager through,
//! private to it: the real system behind `real` (and in its own tests), a
//! dry run that writes changes down, and the ways a command's failure is
//! said. Tests replace the system with a fake and never start a service.

use kkt_service::ServiceError;
use std::cell::RefCell;
use std::fmt;
use std::io;
use std::path::Path;
#[cfg(any(test, feature = "real"))]
use std::process::Stdio;

/// The files and commands the service manager is worked through: the real
/// system ([`OsSystem`]), a dry run ([`DryRun`]), or a test's fake.
pub(crate) trait System {
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
pub(crate) struct Command {
    pub(crate) program: &'static str,
    pub(crate) args: Vec<String>,
    /// True if running it only asks a question and changes nothing, so a
    /// dry run may still run it.
    pub(crate) query: bool,
}

impl Command {
    /// A command that changes something.
    pub(crate) fn new(program: &'static str, args: &[&str]) -> Command {
        Command {
            program,
            args: args.iter().map(|a| a.to_string()).collect(),
            query: false,
        }
    }

    /// A command that only asks a question.
    pub(crate) fn query(program: &'static str, args: &[&str]) -> Command {
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
pub(crate) struct Finished {
    /// The exit status, or `None` if a signal ended it.
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl Finished {
    pub(crate) fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// The real system: files on disk and real commands.
#[cfg(any(test, feature = "real"))]
pub(crate) struct OsSystem;

#[cfg(any(test, feature = "real"))]
impl System for OsSystem {
    fn exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn run(&self, command: &Command) -> io::Result<Finished> {
        let output = std::process::Command::new(command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .output()?;
        Ok(Finished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        let status = std::process::Command::new(command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .status()?;
        Ok(Finished {
            code: status.code(),
            ..Finished::default()
        })
    }
}

/// Looks at the real system but changes nothing: each change, and each
/// command that changes something, is written down instead and taken as
/// succeeding. Queries still run. Files written are not then seen to exist.
pub(crate) struct DryRun<'a> {
    real: &'a dyn System,
    actions: RefCell<Vec<String>>,
}

impl<'a> DryRun<'a> {
    /// A dry run that answers questions from `real`.
    pub(crate) fn new(real: &'a dyn System) -> DryRun<'a> {
        DryRun {
            real,
            actions: RefCell::new(Vec::new()),
        }
    }

    /// What would have been done, in order.
    pub(crate) fn actions(self) -> Vec<String> {
        self.actions.into_inner()
    }

    fn note(&self, action: String) {
        self.actions.borrow_mut().push(action);
    }
}

impl System for DryRun<'_> {
    fn exists(&self, path: &Path) -> io::Result<bool> {
        self.real.exists(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.real.read_file(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        if !self.real.exists(path)? {
            self.note(format!("Would create {}", path.display()));
        }
        Ok(())
    }

    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        self.note(format!("Would write {}:\n{contents}", path.display()));
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.note(format!("Would remove {}", path.display()));
        Ok(())
    }

    fn run(&self, command: &Command) -> io::Result<Finished> {
        if command.query {
            return self.real.run(command);
        }
        self.note(format!("Would run: {command}"));
        Ok(Finished {
            code: Some(0),
            ..Finished::default()
        })
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        self.run(command)
    }
}

pub(crate) fn create_dir(system: &dyn System, dir: &Path) -> Result<(), ServiceError> {
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
pub(crate) fn exists(system: &dyn System, path: &Path) -> Result<bool, ServiceError> {
    system.exists(path).map_err(|source| ServiceError::File {
        action: "check",
        path: path.to_path_buf(),
        source,
    })
}

/// The service file at `path`, or `None` if it is not there.
pub(crate) fn read_service_file(
    system: &dyn System,
    path: &Path,
) -> Result<Option<String>, ServiceError> {
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

pub(crate) fn write_file(
    system: &dyn System,
    path: &Path,
    contents: &str,
) -> Result<(), ServiceError> {
    system
        .write_file(path, contents)
        .map_err(|source| ServiceError::File {
            action: "write",
            path: path.to_path_buf(),
            source,
        })
}

/// Removes `path` if it is there.
pub(crate) fn remove_file(system: &dyn System, path: &Path) -> Result<bool, ServiceError> {
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
pub(crate) fn run(system: &dyn System, command: &Command) -> Result<Finished, ServiceError> {
    system.run(command).map_err(|source| ServiceError::Run {
        command: command.to_string(),
        source,
    })
}

/// Runs `command`, which must succeed.
pub(crate) fn run_ok(system: &dyn System, command: &Command) -> Result<Finished, ServiceError> {
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
pub(crate) fn run_attached(
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
pub(crate) fn failure_detail(finished: &Finished) -> String {
    let said = [finished.stderr.trim(), finished.stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty());
    match (said, finished.code) {
        (Some(said), _) => said.to_string(),
        (None, Some(code)) => format!("exit status {code}"),
        (None, None) => "ended by a signal".to_string(),
    }
}
