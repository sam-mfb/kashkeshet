//! Sharing this machine's server securely with clients on its private network.
//! Implementations keep the network's login and unrelated sharing unchanged.
//! The contract is available without `real`; `Tailscale` implements it behind
//! that feature, with private tool logic also compiled for unit tests.

#[cfg(any(feature = "real", test))]
mod tailscale;
#[cfg(feature = "real")]
pub use tailscale::Tailscale;

use std::net::SocketAddr;

/// The network name by which clients reach this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub dns_name: String,
}

/// Preparing and managing the secure sharing of this machine's server.
pub trait RemoteAccess {
    /// Check that sharing is available and return this machine's network name.
    /// Does not change the login or start sharing.
    fn prepare(&self) -> Result<Node, String>;
    /// Share the server at `bind`, refusing to replace an unrelated sharing rule.
    fn serve(&self, bind: SocketAddr) -> Result<(), String>;
    /// Find the network name currently sharing this server, if any. Read only.
    fn served(&self, bind: SocketAddr) -> Result<Option<Node>, String>;
    /// Stop this server's sharing, preserving unrelated rules and the login.
    fn stop(&self, bind: SocketAddr) -> Result<(), String>;
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
