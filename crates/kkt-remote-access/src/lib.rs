//! Sharing this machine's server securely with clients on its private network,
//! as a contract: `RemoteAccess`, `Node`, its errors (`SharingError`, with
//! the kinds a caller can act on in `Sharing`), and a stand-in behind
//! `testing`. Implementations keep the network's login and unrelated
//! sharing unchanged.

use std::net::SocketAddr;

/// The network name by which clients reach this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub dns_name: String,
}

/// Why sharing could not be done: the kind, which a caller can act on, and
/// what to tell the person, worded by the implementation, which knows its
/// tool and the advice for it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{said}")]
pub struct SharingError {
    pub kind: Sharing,
    pub said: String,
}

impl SharingError {
    pub fn new(kind: Sharing, said: impl Into<String>) -> SharingError {
        SharingError {
            kind,
            said: said.into(),
        }
    }
}

/// The kinds of sharing failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sharing {
    /// The sharing tool could not be run.
    Unavailable,
    /// The node is not running or not logged in; `state` is its state, as
    /// the tool names it.
    NotLoggedIn { state: String },
    /// The node has no usable name on its network.
    NoName,
    /// Certificates for HTTPS are not enabled for the node.
    NoHttps,
    /// The server's address cannot be shared this way.
    NotLoopback,
    /// What sharing would use already serves something else.
    Occupied,
    /// A sharing command ran and did not succeed.
    Failed,
    /// An answer from the tool could not be read.
    Unreadable,
}

/// Preparing and managing the secure sharing of this machine's server.
pub trait RemoteAccess {
    /// Check that sharing is available and return this machine's network name.
    /// Does not change the login or start sharing.
    fn prepare(&self) -> Result<Node, SharingError>;
    /// Share the server at `bind`, refusing to replace an unrelated sharing rule.
    fn serve(&self, bind: SocketAddr) -> Result<(), SharingError>;
    /// Find the network name currently sharing this server, if any. Read only.
    fn served(&self, bind: SocketAddr) -> Result<Option<Node>, SharingError>;
    /// Stop this server's sharing, preserving unrelated rules and the login.
    fn stop(&self, bind: SocketAddr) -> Result<(), SharingError>;
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
