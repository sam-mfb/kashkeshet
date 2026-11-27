//! What the checks need from the machine: reading key files and probing a
//! server, behind [`Prober`], which the program supplies; with what it finds
//! ([`KeyFile`], and a probe's facts: [`Answered`], [`Initialized`], [`Tls`]) and the
//! checks made of them ([`Check`]).

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

/// What a [`Prober`] found in a key file.
#[derive(Debug, Clone)]
pub struct KeyFile {
    /// The file's permission bits, where the system has them.
    pub mode: Option<u32>,
    /// The file's text, or why it could not be read.
    pub text: Result<String, KeyFileUnread>,
}

/// Why a [`Prober`] could not look at a key file or read its text: its own
/// error, passed on unread. Shared, so a [`KeyFile`] can be copied.
#[derive(Debug, Clone, thiserror::Error)]
#[error(transparent)]
pub struct KeyFileUnread(pub std::sync::Arc<dyn std::error::Error + Send + Sync>);

impl KeyFileUnread {
    /// `cause`, as why a key file could not be looked at or read.
    pub fn new(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> KeyFileUnread {
        KeyFileUnread(std::sync::Arc::from(cause.into()))
    }
}

/// An HTTP status: its code, and its standard reason phrase where it has
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub code: u16,
    pub reason: Option<String>,
}

/// What a GET of the server's address gave: its status, or the error that
/// kept it from answering.
pub type Answered = Result<Status, String>;

/// What the server said about itself in its `initialize` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    ServerInfo {
        name: Option<String>,
        version: Option<String>,
    },
    NoServerInfo,
}

/// Why no `initialize` result could be read from a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyError {
    /// The reply grew past the size a probe reads.
    TooLarge,
    /// The reply ended with no `initialize` result or error in it.
    NoResult,
    /// The reply could not be read: the error.
    Read(String),
}

/// What an MCP `initialize` request gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Initialized {
    /// The request could not be sent: the error.
    Failed(String),
    /// The server answered with a status other than 200.
    Refused(Status),
    /// The server answered 200: what its reply held, and, where it opened
    /// a session, what closing that session gave.
    Replied {
        reply: Result<Reply, ReplyError>,
        cleanup: Option<Result<Status, String>>,
    },
}

/// What a GET over https found at an address, as far as TLS goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tls {
    /// A server answered in TLS, with any status.
    Answered,
    /// Nothing listens there: the connection was refused.
    NoListener,
    /// Something answered, but not in TLS, as a plain-http server does.
    NotTls,
    /// The request failed some other way, such as a certificate that is not
    /// trusted, a connection closed during the handshake, or a timeout: the
    /// error.
    Failed(String),
}

/// Why no request could be made at all, worded for the person.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unprobed {
    /// The address is not a URL.
    #[error("invalid diagnostic URL")]
    BadUrl,
    /// The HTTP client could not be built: the error, in the words of the
    /// prober, which keeps its HTTP library's types to itself.
    #[error("could not construct the diagnostic HTTP client: {0}")]
    NoClient(String),
}

/// A probe's answer, once it is in.
pub type Probing<'a, T> = Pin<Box<dyn Future<Output = Result<T, Unprobed>> + Send + 'a>>;

/// What the checks do to the machine. None registers an agent or changes a
/// server's database.
pub trait Prober: Sync {
    /// What the key file at `path` holds. `Err` says why the file could not
    /// be looked at.
    fn key_file(&self, path: &Path) -> Result<KeyFile, KeyFileUnread>;

    /// A plain GET of `url`.
    fn answer<'a>(&'a self, url: &'a str) -> Probing<'a, Answered>;

    /// An MCP `initialize` request to `url`, with `key` (`None` for none),
    /// closing any session it opens.
    fn initialize<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a, Initialized>;

    /// A plain GET of `url`, an https address, with no key: whether
    /// anything there speaks TLS.
    fn tls<'a>(&'a self, url: &'a str) -> Probing<'a, Tls>;
}

/// One check and its outcome: its name, whether it passed, what was found,
/// and for a problem, how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}
