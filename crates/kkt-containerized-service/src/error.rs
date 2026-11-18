//! Why an operation of the containerized service failed: its kinds, each
//! worded for the person running kkt.

use kkt_container::{FileAction, ProjectError, Projects};
use std::fmt;

/// Why a [`ServiceContainer`](crate::ServiceContainer) operation failed. Each kind says
/// what went wrong, and its text is what the person reads.
#[derive(Debug, PartialEq, Eq)]
pub enum ContainerError {
    /// The container tool could not be run at all, or did not answer as a
    /// tool that is there: `{tool} is not available: {error}`.
    NotAvailable { tool: String, error: ProjectError },
    /// Setup's first check found no container tool to run: it says so, and
    /// how to fix it.
    ToolMissing {
        tool: String,
        command: String,
        cause: String,
    },
    /// A command could not be started: `could not run `{command}`: {cause}`.
    CannotRun { command: String, cause: String },
    /// A command ran and failed: `` `{command}` failed ({detail}) ``. When the
    /// command prints a key, `detail` never holds its output.
    Command { command: String, detail: String },
    /// A file or folder could not be read, written or made: `{what}: {detail}`.
    Io { what: String, detail: String },
    /// An answer that is not what was asked for:
    /// `unexpected output from {what}: {detail}`.
    Unexpected { what: String, detail: String },
    /// The tool names no container for a service:
    /// `no container from `{command}`: {detail}`.
    NoContainer { command: String, detail: String },
    /// The tool would not pass on an argument it was given.
    Refused { what: String },
    /// A key copy on this machine could not be written, as kkt-keys'
    /// `KeyFiles` says it.
    KeyFile(String),
    /// The server did not start, or did not make its keys, in time; `logs`
    /// is the command that shows why.
    StartTimeout { logs: String },
    /// The sharing node was not logged in in time.
    LoginTimeout,
    /// The tailnet does not issue HTTPS certificates.
    HttpsDisabled,
    /// A path under the config home that is not plain.
    NotPlain { path: String },
    /// The managed folder's install is not one setup finished, or its
    /// settings file lacks what a finished one has; `reason` says what.
    Incomplete { reason: String },
    /// An upgrade failed, and the previous settings could not be put back.
    NotRestored {
        error: Box<ContainerError>,
        restore: Box<ContainerError>,
    },
}

impl ContainerError {
    /// A failure of the container tool, as the person is told it: a tool
    /// that could not be run at all is said to be not available, by its
    /// name.
    pub(crate) fn from_tool(projects: &dyn Projects, error: ProjectError) -> ContainerError {
        match error {
            ProjectError::Missing { .. } => ContainerError::NotAvailable {
                tool: projects.tool().to_string(),
                error,
            },
            other => other.into(),
        }
    }

    /// An answer that is not what was asked for, from `what`.
    pub(crate) fn unexpected(what: &str, detail: &str) -> ContainerError {
        ContainerError::Unexpected {
            what: what.to_string(),
            detail: detail.to_string(),
        }
    }
}

/// A failure of the container tool, said as the tool says it.
impl From<ProjectError> for ContainerError {
    fn from(error: ProjectError) -> ContainerError {
        let detail = error.detail();
        match error {
            ProjectError::Missing { command, cause } => {
                ContainerError::CannotRun { command, cause }
            }
            ProjectError::Failed { command, .. } => ContainerError::Command { command, detail },
            ProjectError::Unexpected { command, detail } => ContainerError::Unexpected {
                what: format!("`{command}`"),
                detail,
            },
            ProjectError::NoContainer { command, detail } => {
                ContainerError::NoContainer { command, detail }
            }
            ProjectError::File {
                action,
                path,
                detail,
                ..
            } => {
                let verb = match action {
                    FileAction::Read => "read",
                    FileAction::Write => "write",
                    FileAction::Make => "create",
                };
                ContainerError::Io {
                    what: format!("could not {verb} {}", path.display()),
                    detail,
                }
            }
            ProjectError::Refused { what } => ContainerError::Refused { what },
        }
    }
}

/// The text, for callers that word their own failures as text.
impl From<ContainerError> for String {
    fn from(error: ContainerError) -> String {
        error.to_string()
    }
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::NotAvailable { tool, error } => {
                write!(f, "{tool} is not available: {error}")
            }
            ContainerError::ToolMissing {
                tool,
                command,
                cause,
            } => write!(
                f,
                "{tool} is not available: could not run `{command}`: {cause}. Install {tool} with Compose and run setup again"
            ),
            ContainerError::CannotRun { command, cause } => {
                write!(f, "could not run `{command}`: {cause}")
            }
            ContainerError::Command { command, detail } => {
                write!(f, "`{command}` failed ({detail})")
            }
            ContainerError::Io { what, detail } => write!(f, "{what}: {detail}"),
            ContainerError::Unexpected { what, detail } => {
                write!(f, "unexpected output from {what}: {detail}")
            }
            ContainerError::NoContainer { command, detail } => {
                write!(f, "no container from `{command}`: {detail}")
            }
            ContainerError::Refused { what } => f.write_str(what),
            ContainerError::KeyFile(message) => f.write_str(message),
            ContainerError::StartTimeout { logs } => write!(
                f,
                "kkt did not start or did not make its keys in time; `{logs}` shows why"
            ),
            ContainerError::LoginTimeout => f.write_str(
                "Tailscale is not logged in yet. Run setup again to continue; the containers are left as they are",
            ),
            ContainerError::HttpsDisabled => f.write_str(
                "HTTPS certificates are not enabled for your tailnet. Turn them on at https://login.tailscale.com/admin/dns (\"Enable HTTPS\"), then run setup again",
            ),
            ContainerError::NotPlain { path } => {
                write!(f, "{path} is not a plain path under the config home")
            }
            ContainerError::Incomplete { reason } => f.write_str(reason),
            ContainerError::NotRestored { error, restore } => write!(
                f,
                "unexpected output from upgrade recovery: {error}; could not restore the previous settings: {restore}"
            ),
        }
    }
}

impl std::error::Error for ContainerError {}
