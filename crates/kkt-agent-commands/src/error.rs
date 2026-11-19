//! Why an agent command failed, worded for the person who ran it.

use kkt_deployment::connection::ConnectionError;
use kkt_llm_agent_config::ClientConfigError;
use kkt_names::NameError;
use kkt_watch::WatchError;
use std::error::Error;
use std::path::PathBuf;

/// Why `register`, `reclaim`, `watch` or `connect` failed.
#[derive(Debug, thiserror::Error)]
pub enum AgentCommandError {
    /// `connect` was given a client kkt does not know.
    #[error("unknown client")]
    UnknownClient,
    /// The server refused the lane `register` was asked to take, in its own
    /// words, which are not a credential.
    #[error("the server refused the lane: {message}")]
    LaneRefused { message: String },
    /// The server could not be reached, its reply read, or the access key
    /// was refused. A call that carries a credential echoes no details.
    #[error("could not reach the server or read its reply")]
    Unreachable,
    /// The server refused the avatar request with `code`.
    #[error("the server refused the avatar request ({code})")]
    Refused { code: String },
    /// A key file is there but could not be read: the system's reason.
    #[error("could not read key file {}: {reason}", path.display())]
    KeyFile { path: PathBuf, reason: String },
    /// There is no admin key at `path`.
    #[error(
        "there is no admin key at {}; `kkt key create --admin` makes one on the server",
        path.display()
    )]
    NoAdminKey { path: PathBuf },
    /// A handle or lane name breaks its rules.
    #[error(transparent)]
    Name(NameError),
    /// Where the server is, or its admin endpoint, could not be worked out.
    #[error(transparent)]
    Connection(ConnectionError),
    /// The watch stopped without its outcome.
    #[error(transparent)]
    Watch(WatchError),
    /// The client's config could not be read, changed or written.
    #[error(transparent)]
    ClientConfig(ClientConfigError),
    /// The credential files failed.
    #[error(transparent)]
    Credentials(CredentialsError),
    /// The admin key could not be found.
    #[error(transparent)]
    AdminKey(AdminKeyError),
    /// This program's path could not be found.
    #[error(transparent)]
    Program(ProgramError),
}

/// Why [`Credentials`](crate::Credentials) or a
/// [`CredentialSlot`](crate::CredentialSlot) failed: the implementation's
/// own error, passed on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct CredentialsError(pub Box<dyn Error + Send + Sync>);

/// Why [`AdminKey`](crate::AdminKey) could not find the admin key: the
/// implementation's own error, passed on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AdminKeyError(pub Box<dyn Error + Send + Sync>);

/// Why [`Program`](crate::Program) could not give this program's path: the
/// implementation's own error, passed on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ProgramError(pub Box<dyn Error + Send + Sync>);

/// A credential file that another user owns, which kkt neither reads nor
/// tightens.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "credential file {} is owned by another user (uid {owner}); ask its owner or an administrator to run chown {uid} {quoted}, then chmod 600 {quoted}",
    path.display()
)]
pub struct ForeignCredentialFile {
    pub path: PathBuf,
    /// The path as one shell word, for the advice.
    pub quoted: String,
    pub owner: u32,
    /// The caller's user ID.
    pub uid: u32,
}
