//! Running the assembled server: binding its address, then answering on it
//! until told to stop. axum's server and tokio's listener stay in here; the
//! contract is in std types only.
//!
//! Running takes two steps, [`listen`] and [`Listening::serve_until`], so a
//! caller can report what it bound (and log) before the server answers.

use axum::Router;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// The server, assembled by [`crate::build`], ready to [`listen`].
pub struct App {
    pub(crate) router: Router,
}

/// The server bound to its address, not yet answering. Made by [`listen`].
pub struct Listening {
    listener: TcpListener,
    router: Router,
}

/// Binds `app` to `address`. Port 0 asks the system to choose one;
/// [`Listening::address`] then names it. The error is the operating
/// system's, for the caller to report.
pub async fn listen(app: App, address: SocketAddr) -> io::Result<Listening> {
    let listener = TcpListener::bind(address).await?;
    Ok(Listening {
        listener,
        router: app.router,
    })
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
    pub async fn serve_until(
        self,
        stop: impl Future<Output = ()> + Send + 'static,
    ) -> io::Result<()> {
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(stop)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, Parts, build};
    use kkt_admission::Admission;
    use kkt_clock::testing::FixedClock;
    use kkt_config::Limits;
    use kkt_hosts::AllowedHosts;
    use kkt_names::testing::SeqEntropy;
    use kkt_shutdown::Shutdown;
    use kkt_store::Db;
    use kkt_web::testing::start_time;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    fn app(shutdown: &Shutdown) -> App {
        let parts = Parts::new(
            Db::open_in_memory().expect("db"),
            Arc::new(FixedClock::new(start_time())),
            Arc::new(SeqEntropy::new(1)),
            &Limits::DEFAULT,
            1,
        );
        let config = AppConfig {
            admission: Admission::Open,
            web_admission: Admission::Open,
            admin_key: None,
            admin_name: None,
            limits: Limits::DEFAULT,
            allowed_origins: Vec::new(),
            allowed_hosts: AllowedHosts::new(&[]).expect("loopback hosts"),
            instructions: "test instructions".into(),
        };
        build(parts, config, shutdown.child())
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
        let shutdown = Shutdown::new();
        let listening = listen(app(&shutdown), "127.0.0.1:0".parse().expect("address"))
            .await
            .expect("listen");
        let address = listening.address().expect("address");
        assert_ne!(address.port(), 0);
        let stop = shutdown.clone();
        let served = tokio::spawn(listening.serve_until(async move { stop.stopped().await }));
        assert_eq!(status_line(address, "/").await, "HTTP/1.1 200 OK");
        shutdown.stop();
        served.await.expect("join").expect("served");
        assert!(TcpStream::connect(address).await.is_err());
    }

    #[tokio::test]
    async fn listening_on_an_address_in_use_fails() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = taken.local_addr().expect("address");
        let shutdown = Shutdown::new();
        assert!(listen(app(&shutdown), address).await.is_err());
    }
}
