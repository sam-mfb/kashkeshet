//! Whether the service's server could listen on its address, tried on this
//! machine: [`try_bind`] listens and lets the address go at once. kkt-service's
//! `check_bind` decides what the answer means; the program passes this in.

/// Listens on the address and lets it go at once, as the service bind check requires.
#[cfg(any(test, feature = "real"))]
pub fn try_bind(address: std::net::SocketAddr) -> std::io::Result<()> {
    std::net::TcpListener::bind(address).map(drop)
}
