//! Why an agent's credential or state file could not be used, worded for
//! the person.

use std::error::Error;
use std::io;
use std::path::PathBuf;

/// What was being done to a credential file when it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialAction {
    Create,
    Save,
    Inspect,
    Open,
    Read,
}

impl CredentialAction {
    fn verb(self) -> &'static str {
        match self {
            CredentialAction::Create => "create",
            CredentialAction::Save => "save",
            CredentialAction::Inspect => "inspect",
            CredentialAction::Open => "open",
            CredentialAction::Read => "read",
        }
    }
}

/// What was being done to a state file when it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateAction {
    Read,
    Write,
}

impl StateAction {
    fn verb(self) -> &'static str {
        match self {
            StateAction::Read => "read",
            StateAction::Write => "write",
        }
    }
}

/// Why a state file's lock could not be taken, other than another holder.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// The lock file could not be opened or made.
    #[error("could not open lock file {}: {cause}", lock.display())]
    Open {
        lock: PathBuf,
        #[source]
        cause: io::Error,
    },
    /// The lock file could not be locked.
    #[error("could not lock {}: {cause}", lock.display())]
    Lock {
        lock: PathBuf,
        #[source]
        cause: io::Error,
    },
}

/// Why a credential or state file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum AgentFileError {
    /// A step on a credential file failed.
    #[error("could not {} credential file {}: {cause}", action.verb(), path.display())]
    Credential {
        action: CredentialAction,
        path: PathBuf,
        #[source]
        cause: io::Error,
    },
    /// A credential file could not be made private. `advice` is the
    /// caller's rule's own sentence on how to fix it, not an error.
    #[error("could not make credential file {} private: {cause}; {advice}", path.display())]
    NotPrivate {
        path: PathBuf,
        #[source]
        cause: io::Error,
        advice: String,
    },
    /// The caller's credential-file rule refused the file, with its own
    /// error.
    #[error(transparent)]
    Refused(Box<dyn Error + Send + Sync>),
    /// No token file was given and the environment names none.
    #[error("give the agent's token in --token-file or the {variable} environment variable")]
    NoToken { variable: &'static str },
    /// The token is empty.
    #[error("the agent's token is empty")]
    EmptyToken,
    /// Another watch holds the state file.
    #[error(
        "state file {} is in use by another kkt watch; stop that one, or give each watcher its own --state-file",
        path.display()
    )]
    StateInUse { path: PathBuf },
    /// The state file's lock could not be taken.
    #[error("could not claim state file {}: {cause}", path.display())]
    NotClaimed {
        path: PathBuf,
        #[source]
        cause: LockError,
    },
    /// A step on a state file failed.
    #[error("could not {} state file {}: {cause}", action.verb(), path.display())]
    State {
        action: StateAction,
        path: PathBuf,
        #[source]
        cause: io::Error,
    },
    /// The state file holds something other than a message ID.
    #[error(
        "state file {} does not hold a message ID; remove it to start after the latest message",
        path.display()
    )]
    NotAnId { path: PathBuf },
}

impl AgentFileError {
    pub(crate) fn credential(
        action: CredentialAction,
        path: &std::path::Path,
        cause: io::Error,
    ) -> Self {
        AgentFileError::Credential {
            action,
            path: path.to_path_buf(),
            cause,
        }
    }
}
