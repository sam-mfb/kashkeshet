//! A client of kkt's server, as kkt's commands use it: registering and
//! reclaiming avatars, and watching for an agent's new messages or, with
//! the admin key, for every channel's. In kkt's terms only: nothing here
//! names a transport or a library. Its one implementation, over MCP through
//! rmcp and reqwest (`Mcp`, `Client`, `Remote`, `AdminRemote`, and `Http` for
//! a program's test client), is behind `real`, which only programs turn on;
//! `Settings` are plain values. `testing::Memory` is a stand-in.

use futures::future::BoxFuture;
use kkt_clock::Clock;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// A server, as a client reaches it: its MCP endpoint and the key it is
/// reached with, if it needs one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub url: String,
    pub key: Option<String>,
    /// How a refusal of `key` names it, such as "the access key in
    /// /home/me/key".
    pub key_name: Option<String>,
}

/// Clients of a server.
pub trait Servers {
    /// A client that registers and reclaims avatars at `at`.
    fn avatars(&self, at: Address) -> Box<dyn Avatars>;
    /// The agent endpoint at `at`, watched as the agent `token` names.
    fn agent(&self, at: Address, token: String) -> Box<dyn Watched>;
    /// The admin endpoint at `at`, watched with the key it is reached with;
    /// its waits are timed by `clock`.
    fn admin(&self, at: Address, clock: Arc<dyn Clock>) -> Box<dyn Watched>;
}

/// What a client asks of a server for an avatar.
pub trait Avatars {
    /// A new avatar, with the handle and the lane asked for, if any.
    fn register<'a>(
        &'a self,
        handle: Option<&'a str>,
        lane: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Registered, Failure>>;
    /// A fresh token for the avatar `handle`.
    fn reclaim<'a>(
        &'a self,
        handle: &'a str,
        recovery_code: &'a str,
    ) -> BoxFuture<'a, Result<Reclaimed, Failure>>;
    /// Sets the description of the avatar `token` names.
    fn describe<'a>(&'a self, token: &'a str, text: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
}

/// What watching asks of a server: as an agent, the agent's channels; with
/// the admin key, every channel.
pub trait Watched: Send + Sync {
    /// The watcher's name: the agent's handle, or the admin's. A watch asks
    /// for it first, on every start, so it must reach the server with the
    /// watcher's credentials: a failure here is what reports a wrong
    /// address, key or token at once.
    fn handle(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// All visible channels, including every page of the server's list.
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>>;
    /// The highest message ID in the watched channels, or 0 if none.
    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>>;
    /// One message, with its whole body.
    fn message(&self, id: i64) -> BoxFuture<'_, Result<Message, Failure>>;
    /// Messages newer than `after_id`, oldest first, waiting up to `wait`
    /// for one, and the agent's lane if it has a newer version. `channels`
    /// limits the messages to those channels.
    fn wait<'a>(
        &'a self,
        after_id: i64,
        channels: Option<&'a [String]>,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Waited, Failure>>;
}

/// Why a call to the server failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The server could not be reached, the connection broke, or its answer
    /// made no sense.
    Lost(String),
    /// The server answered with one of its error codes, such as
    /// `unauthenticated`.
    Refused { code: String, message: String },
    /// The server refused the access key: it answered HTTP 401 with a
    /// `WWW-Authenticate` challenge, as kkt's admission does. Trying again
    /// with the same key will not help. `key` names it, such as `the access
    /// key in /home/me/.kkt/key`, when the caller knows where it came from.
    KeyRefused { key: Option<String> },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Lost(reason) => write!(f, "could not reach the server: {reason}"),
            Failure::Refused { code, message } => write!(f, "{code}: {message}"),
            Failure::KeyRefused { key: Some(key) } => write!(f, "the server refused {key}"),
            Failure::KeyRefused { key: None } => write!(f, "the server refused the access key"),
        }
    }
}

/// A new message, as `kkt watch` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
    /// Whether the server cut the body to fit its answer.
    pub cut: bool,
}

/// What a wait gave: new messages, and the agent's lane if it has a newer
/// version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Waited {
    pub messages: Vec<Message>,
    pub newer_lane: Option<NewerLane>,
}

/// The lane the agent holds, which has a newer version than the one it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewerLane {
    pub name: String,
    /// The version the agent took.
    pub version: i64,
    /// The lane's newest version.
    pub current_version: i64,
}

/// A visible channel, for resolving names before watching starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
}

/// Credentials returned once by registration. Never format these in output.
pub struct Registered {
    pub handle: String,
    pub token: String,
    pub recovery_code: String,
}

/// A replacement token for an existing avatar.
pub struct Reclaimed {
    pub handle: String,
    pub token: String,
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

mod settings;
pub use settings::Settings;

#[cfg(feature = "real")]
mod mcp;
#[cfg(feature = "real")]
pub use mcp::{AdminRemote, Client, Http, HttpReply, Mcp, Remote};

#[cfg(test)]
mod tests {
    use super::Failure;

    #[test]
    fn each_failure_is_worded_without_more_than_it_was_given() {
        assert_eq!(
            Failure::Lost("MCP connection failed".into()).to_string(),
            "could not reach the server: MCP connection failed"
        );
        assert_eq!(
            Failure::Refused {
                code: "rate_limited".into(),
                message: "slow down".into()
            }
            .to_string(),
            "rate_limited: slow down"
        );
        assert_eq!(
            Failure::KeyRefused {
                key: Some("the access key in /home/me/key".into())
            }
            .to_string(),
            "the server refused the access key in /home/me/key"
        );
        assert_eq!(
            Failure::KeyRefused { key: None }.to_string(),
            "the server refused the access key"
        );
    }
}
