//! A Compose project as a general contract: [`Projects`], the operations a
//! container tool offers on a folder of Compose files and the services they
//! describe, with what they are given and give back, and [`ProjectError`].
//! Nothing here knows what runs in the services: the caller names the
//! files, services, commands and variables.

use std::io;
use std::path::{Path, PathBuf};

use crate::Folder;

/// A Compose project: a folder of files and the services they describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Project<'a> {
    /// The folder the project's files are in. Every command runs there.
    pub folder: &'a Folder,
    /// Variables the project's files read, which every command runs
    /// without, so the folder's own settings file wins over the caller's
    /// environment. The implementation adds the tool's own such variables.
    pub shielded: &'a [&'a str],
}

/// A file to write in or for a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectFile<'a> {
    /// Where it goes: relative to the project's folder, or absolute.
    pub path: &'a Path,
    /// Its whole text, replacing any file there.
    pub text: &'a str,
    /// Readable only by its owner, whether or not it was there before.
    pub private: bool,
}

/// Where a command's output goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Kept and given back in [`Ran`].
    Captured,
    /// Straight to the person's terminal, as for a download they are
    /// waiting on. [`Ran`]'s output is then empty.
    Terminal,
}

/// The user a command in a service runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum User {
    /// The user the service's image names.
    Default,
    /// The superuser.
    Root,
}

/// Whether a path is there, and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    /// Nothing is there.
    Missing,
    /// A regular file, following links.
    File,
    /// Something else, such as a folder.
    Other,
}

/// A command that ran, however it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// How a person would type it.
    pub command: String,
    /// The exit status, or `None` if a signal ended it.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    /// Whether it exited with status 0.
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// What it printed, if it exited with status 0, or a
    /// [`ProjectError::Failed`] saying how it ended.
    pub fn succeeded(self) -> Result<String, ProjectError> {
        if self.success() {
            Ok(self.stdout)
        } else {
            Err(ProjectError::Failed {
                command: self.command,
                status: self.status,
                stdout: self.stdout,
                stderr: self.stderr,
            })
        }
    }
}

/// How a running service was started, as the tool reports it. A field is
/// `None` when the report does not give it as a list of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceConfig {
    /// The arguments after the entrypoint.
    pub command: Option<Vec<String>>,
    /// The program the service runs.
    pub entrypoint: Option<Vec<String>>,
    /// Its environment, as `NAME=value` entries.
    pub env: Option<Vec<String>>,
    /// The folder it runs in; `None` when the image names none.
    pub working_dir: Option<String>,
}

/// What was being done to a file when it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    Read,
    Write,
    Make,
}

/// Why an operation on a project failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectError {
    /// The tool could not be started at all.
    #[error("could not run `{command}`: {cause}")]
    Missing { command: String, cause: String },
    /// A command ran and did not succeed.
    #[error("`{command}` failed ({})", failed_detail(*status, stdout, stderr))]
    Failed {
        command: String,
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// A command's answer was not what the operation reads.
    #[error("unexpected output from `{command}`: {detail}")]
    Unexpected { command: String, detail: String },
    /// The tool named no container for a service, or an answer that is
    /// not a container's ID.
    #[error("no container from `{command}`: {detail}")]
    NoContainer { command: String, detail: String },
    /// A file in or for the project could not be read, written or made.
    #[error("could not {} {}: {detail}", action.verb(), path.display())]
    File {
        action: FileAction,
        path: PathBuf,
        cause: io::ErrorKind,
        detail: String,
    },
    /// An argument the implementation will not pass on.
    #[error("{what}")]
    Refused { what: String },
}

/// A failed command's exit status and what it said: its error output, or
/// else its output, trimmed (`exit 1: no such service`, or `ended by a
/// signal: ...`).
fn failed_detail(status: Option<i32>, stdout: &str, stderr: &str) -> String {
    let said = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    match status {
        Some(code) => format!("exit {code}: {}", said.trim()),
        None => format!("ended by a signal: {}", said.trim()),
    }
}

impl FileAction {
    /// The verb a failed file action is said with.
    fn verb(self) -> &'static str {
        match self {
            FileAction::Read => "read",
            FileAction::Write => "write",
            FileAction::Make => "create",
        }
    }
}

impl ProjectError {
    /// What went wrong, without the command. For a failed command: its exit
    /// status and what it said, its error output or else its output,
    /// trimmed (`exit 1: no such service`, or `ended by a signal: ...`).
    pub fn detail(&self) -> String {
        match self {
            ProjectError::Missing { cause, .. } => cause.clone(),
            ProjectError::Failed {
                status,
                stdout,
                stderr,
                ..
            } => failed_detail(*status, stdout, stderr),
            ProjectError::Unexpected { detail, .. } => detail.clone(),
            ProjectError::NoContainer { detail, .. } => detail.clone(),
            ProjectError::File { detail, .. } => detail.clone(),
            ProjectError::Refused { what } => what.clone(),
        }
    }

    /// A file error from `source`, while doing `action` to `path`.
    pub fn file(action: FileAction, path: &Path, source: &io::Error) -> ProjectError {
        ProjectError::File {
            action,
            path: path.to_path_buf(),
            cause: source.kind(),
            detail: source.to_string(),
        }
    }
}

/// A container tool's operations on Compose projects. Each command runs in
/// the project's folder, without the project's shielded variables.
pub trait Projects {
    /// Whether the tool is installed and answers, asked anywhere but, like
    /// the project's commands, without its shielded variables.
    fn available(&self, project: &Project) -> Result<(), ProjectError>;
    /// Whether the named volume exists, even when the project folder is absent.
    fn has_volume(&self, project: &Project, name: &str) -> Result<bool, ProjectError>;
    /// The tool's name, as a person knows it.
    fn tool(&self) -> &str;
    /// How a person would type the tool's project command with `args`, in
    /// the project's folder.
    fn typed(&self, args: &[&str]) -> String;

    /// Writes each file, in order, stopping at the first that fails.
    fn write(&self, project: &Project, files: &[ProjectFile]) -> Result<(), ProjectError>;
    /// Makes each folder and any missing above it, in order.
    fn make_folders(&self, project: &Project, folders: &[&Path]) -> Result<(), ProjectError>;
    /// The text of `file`, relative to the project's folder or absolute.
    fn read(&self, project: &Project, file: &Path) -> Result<String, ProjectError>;
    /// What is at `file`, relative to the project's folder or absolute.
    fn file(&self, project: &Project, file: &Path) -> FileState;

    /// Starts `services`, or every service when it is empty, in the
    /// background.
    fn up(&self, project: &Project, services: &[&str], out: Output) -> Result<(), ProjectError>;
    /// Stops and removes the services. Their volumes stay unless
    /// `remove_volumes`.
    fn down(
        &self,
        project: &Project,
        remove_volumes: bool,
        out: Output,
    ) -> Result<(), ProjectError>;
    /// Fetches the services' images.
    fn pull(&self, project: &Project, out: Output) -> Result<(), ProjectError>;
    /// Restarts `service`.
    fn restart(&self, project: &Project, service: &str, out: Output) -> Result<(), ProjectError>;
    /// The services that are running.
    fn running(&self, project: &Project) -> Result<Vec<String>, ProjectError>;

    /// Runs `args` in the running `service` as `user`. An error means only
    /// that the command could not be run; how it ended is in [`Ran`].
    fn exec(
        &self,
        project: &Project,
        service: &str,
        user: User,
        args: &[&str],
        out: Output,
    ) -> Result<Ran, ProjectError>;
    /// Runs `entrypoint` with `args` in a new container of `service`, as
    /// `user`, without starting the services it depends on, and removes
    /// the container after. An error means only that the command could not
    /// be run.
    fn run_once(
        &self,
        project: &Project,
        service: &str,
        user: User,
        entrypoint: &str,
        args: &[&str],
        out: Output,
    ) -> Result<Ran, ProjectError>;
    /// Copies `host` into the running `service`, at `inside`.
    fn copy_in(
        &self,
        project: &Project,
        service: &str,
        host: &Path,
        inside: &str,
    ) -> Result<(), ProjectError>;
    /// Copies `inside` the running `service` to `host`.
    fn copy_out(
        &self,
        project: &Project,
        service: &str,
        inside: &str,
        host: &Path,
    ) -> Result<(), ProjectError>;
    /// How the running `service` was started. The tool naming no
    /// container for it is [`ProjectError::NoContainer`].
    fn inspect(&self, project: &Project, service: &str) -> Result<ServiceConfig, ProjectError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_failure_says_what_it_always_said() {
        let file = |action| ProjectError::File {
            action,
            path: PathBuf::from("/p/.env"),
            cause: io::ErrorKind::PermissionDenied,
            detail: "denied".into(),
        };
        let said: Vec<String> = [
            ProjectError::Missing {
                command: "tool version".into(),
                cause: "not found".into(),
            },
            ProjectError::Failed {
                command: "tool up".into(),
                status: Some(2),
                stdout: "out\n".into(),
                stderr: " err \n".into(),
            },
            ProjectError::Failed {
                command: "tool up".into(),
                status: None,
                stdout: "only out".into(),
                stderr: "  ".into(),
            },
            ProjectError::Unexpected {
                command: "tool ps".into(),
                detail: "no list".into(),
            },
            ProjectError::NoContainer {
                command: "tool ps -q web".into(),
                detail: "no ID".into(),
            },
            file(FileAction::Read),
            file(FileAction::Write),
            file(FileAction::Make),
            ProjectError::Refused {
                what: "a path that is not plain".into(),
            },
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            said,
            [
                "could not run `tool version`: not found",
                "`tool up` failed (exit 2: err)",
                "`tool up` failed (ended by a signal: only out)",
                "unexpected output from `tool ps`: no list",
                "no container from `tool ps -q web`: no ID",
                "could not read /p/.env: denied",
                "could not write /p/.env: denied",
                "could not create /p/.env: denied",
                "a path that is not plain",
            ]
        );
    }

    fn failed(status: Option<i32>, stdout: &str, stderr: &str) -> ProjectError {
        Ran {
            command: "tool compose restart web".into(),
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
        .succeeded()
        .unwrap_err()
    }

    #[test]
    fn a_failure_says_its_error_output_or_else_its_output() {
        assert_eq!(
            failed(Some(1), "out\n", " no such service \n").detail(),
            "exit 1: no such service"
        );
        assert_eq!(failed(Some(2), " out \n", "  \n").detail(), "exit 2: out");
        assert_eq!(failed(None, "", "").detail(), "ended by a signal: ");
        assert_eq!(
            failed(Some(1), "", "no such service").to_string(),
            "`tool compose restart web` failed (exit 1: no such service)"
        );
    }

    #[test]
    fn a_command_that_exits_0_gives_its_output() {
        let ran = Ran {
            command: "tool ps".into(),
            status: Some(0),
            stdout: "web\n".into(),
            stderr: "warning".into(),
        };
        assert!(ran.success());
        assert_eq!(ran.succeeded().unwrap(), "web\n");
    }

    #[test]
    fn each_error_says_what_failed() {
        let missing = ProjectError::Missing {
            command: "tool compose version".into(),
            cause: "No such file or directory (os error 2)".into(),
        };
        assert_eq!(
            missing.to_string(),
            "could not run `tool compose version`: No such file or directory (os error 2)"
        );
        let unexpected = ProjectError::Unexpected {
            command: "tool inspect x".into(),
            detail: "not JSON".into(),
        };
        assert_eq!(
            unexpected.to_string(),
            "unexpected output from `tool inspect x`: not JSON"
        );
        let none = ProjectError::NoContainer {
            command: "tool ps web".into(),
            detail: "no container ID for web".into(),
        };
        assert_eq!(
            none.to_string(),
            "no container from `tool ps web`: no container ID for web"
        );
        let source = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let file = ProjectError::file(FileAction::Write, Path::new("/p/.env"), &source);
        assert_eq!(file.to_string(), "could not write /p/.env: denied");
        assert!(matches!(
            file,
            ProjectError::File {
                cause: io::ErrorKind::PermissionDenied,
                ..
            }
        ));
        let refused = ProjectError::Refused {
            what: "a path that is not UTF-8".into(),
        };
        assert_eq!(refused.detail(), "a path that is not UTF-8");
    }
}
