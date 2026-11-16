//! Serving HTTP: binding an address, then answering on it with the router
//! it is handed until told to stop. It touches the network, so only
//! programs depend on it. It knows nothing of how the router was built.
//! axum's server and tokio's listener stay in here; apart from the router,
//! the contract is in std types only.
//!
//! Running takes two steps, [`listen`] and [`Listening::serve_until`], so a
//! caller can report what it bound (and log) before the server answers.
//! [`try_bind`] asks only whether an address could be listened on, and
//! lets it go at once.

use axum::Router;
use axum::serve::ListenerExt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// The server bound to its address, not yet answering. Made by [`listen`].
pub struct Listening {
    listener: TcpListener,
    router: Router,
}

/// Binds `router` to `address`. Port 0 asks the system to choose one;
/// [`Listening::address`] then names it. The error is the operating
/// system's, for the caller to report.
pub async fn listen(router: Router, address: SocketAddr) -> io::Result<Listening> {
    let listener = TcpListener::bind(address).await?;
    Ok(Listening { listener, router })
}

/// Listens on `address` and lets it go at once: whether a server could
/// listen there now. The error is the operating system's.
pub fn try_bind(address: SocketAddr) -> io::Result<()> {
    std::net::TcpListener::bind(address).map(drop)
}

impl Listening {
    /// The address as bound, which names the port the system chose when
    /// the port asked for was 0.
    pub fn address(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Answers requests until `stop` finishes. Then it takes no new
    /// connections and returns once the open ones end. Returns an error only
    /// if serving itself fails.
    ///
    /// Every connection is sent without Nagle's algorithm (`TCP_NODELAY`).
    /// An MCP reply goes out as two writes, its headers and then its event
    /// stream; with Nagle, the second waits for the client to acknowledge
    /// the first, and a client delays that by about 40 ms on every call
    /// after a connection's first. If the option cannot be set on a
    /// connection, the connection is still served, more slowly, and
    /// `report` is given the error to log: this crate does not log.
    pub async fn serve_until(
        self,
        stop: impl Future<Output = ()> + Send + 'static,
        report: impl Fn(&io::Error) + Send + Sync + 'static,
    ) -> io::Result<()> {
        let listener = self.listener.tap_io(move |connection| {
            if let Err(e) = connection.set_nodelay(true) {
                report(&e);
            }
        });
        axum::serve(listener, self.router)
            .with_graceful_shutdown(stop)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::oneshot;

    fn router() -> Router {
        Router::new().route("/", get(|| async { "hello" }))
    }

    /// The status line of a plain `GET path` to `address`.
    async fn status_line(address: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(address).await.expect("connect");
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("read");
        let response = String::from_utf8_lossy(&response);
        response.lines().next().unwrap_or("").to_string()
    }

    #[tokio::test]
    async fn listening_on_port_0_reports_the_port_bound_and_serves_there_until_stopped() {
        let listening = listen(router(), "127.0.0.1:0".parse().expect("address"))
            .await
            .expect("listen");
        let address = listening.address().expect("address");
        assert_ne!(address.port(), 0);
        let (stop, stopped) = oneshot::channel::<()>();
        let served = tokio::spawn(listening.serve_until(
            async move { stopped.await.expect("the test sends stop") },
            |e| panic!("could not set TCP_NODELAY: {e}"),
        ));
        assert_eq!(status_line(address, "/").await, "HTTP/1.1 200 OK");
        assert_eq!(
            status_line(address, "/missing").await,
            "HTTP/1.1 404 Not Found"
        );
        stop.send(()).expect("stop");
        served.await.expect("join").expect("served");
        assert!(TcpStream::connect(address).await.is_err());
    }

    #[test]
    fn try_bind_answers_whether_an_address_is_free_and_lets_it_go() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = taken.local_addr().expect("address");
        assert!(try_bind(address).is_err());
        drop(taken);
        try_bind(address).expect("free once let go");
        try_bind(address).expect("try_bind let it go");
    }

    #[tokio::test]
    async fn listening_on_an_address_in_use_fails() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = taken.local_addr().expect("address");
        assert!(listen(router(), address).await.is_err());
    }
}
