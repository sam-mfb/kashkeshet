//! Probing an MCP server over HTTP, with reqwest, whose types stay inside
//! this crate. [`HttpProber`] makes two requests, each on its own call: a
//! plain GET of the server's address ([`HttpProber::answer`]), and an MCP
//! `initialize` with an optional bearer key, closing the session it opens
//! ([`HttpProber::initialize`]); and, for an https address, whether
//! anything there speaks TLS ([`HttpProber::tls`]). It reports what happened as plain facts:
//! statuses, errors, and the server's name and version. Every value it
//! sends comes from the caller's [`Settings`].

use std::time::Duration;

/// What the prober sends and how long it waits, all chosen by the caller.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// How long a request may take, connecting included.
    pub timeout: Duration,
    /// The most bytes of an `initialize` reply read before giving up.
    pub max_response_bytes: usize,
    /// The MCP protocol version announced and sent.
    pub protocol_version: &'static str,
    /// The client name announced in `initialize`, and the user agent's.
    pub client_name: &'static str,
    /// The client version announced in `initialize`, and the user agent's.
    pub client_version: &'static str,
}

/// An HTTP status: its code, and its standard reason phrase where it has
/// one ("Not Found" for 404).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub code: u16,
    pub reason: Option<String>,
}

/// What a GET of the server's address gave: its status, or the error that
/// kept it from answering.
pub type Answered = Result<Status, String>;

/// What the server said about itself in its `initialize` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The result's `serverInfo`, with its name and version where given.
    ServerInfo {
        name: Option<String>,
        version: Option<String>,
    },
    /// A result with no `serverInfo`.
    NoServerInfo,
}

/// Why no `initialize` result could be read from a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyError {
    /// The reply grew past [`Settings::max_response_bytes`].
    TooLarge,
    /// The reply ended with no `initialize` result or error in it.
    NoResult,
    /// The reply could not be read: the error.
    Read(String),
}

/// What an MCP `initialize` request gave.
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tls {
    /// A server answered in TLS, with this status.
    Answered(Status),
    /// Nothing listens there: the connection was refused.
    NoListener,
    /// Something answered, but not in TLS, as a plain-http server does.
    NotTls,
    /// The request failed some other way, such as a certificate that is not
    /// trusted, a connection closed during the handshake, or a timeout: the
    /// error.
    Failed(String),
}

/// Why no request could be made at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unprobed {
    /// The address is not a URL.
    BadUrl,
    /// The HTTP client could not be built: the error.
    NoClient(String),
}

mod http;
pub use http::HttpProber;
