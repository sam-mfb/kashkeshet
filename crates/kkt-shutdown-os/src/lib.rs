//! The operating system's stop signals. Installed before the server is
//! ready, they stop the shared `kkt_shutdown::Shutdown` value on receipt.
//! Tokio's signal types stay inside this crate.

#![cfg(feature = "real")]

use kkt_shutdown::Shutdown;

/// OS stop signals, installed before the server announces it is ready.
/// Tokio's signal types stay inside this part.
pub struct StopSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}

impl StopSignals {
    pub fn listen() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        })
    }

    /// Waits for Ctrl-C, or SIGTERM on Unix, and stops the shared signal.
    pub async fn stop_on_signal(self, shutdown: Shutdown) -> std::io::Result<&'static str> {
        #[cfg(unix)]
        let result = {
            let mut signals = self;
            let (received, name) = tokio::select! {
                received = signals.interrupt.recv() => (received, "Ctrl-C"),
                received = signals.terminate.recv() => (received, "SIGTERM"),
            };
            received
                .map(|()| name)
                .ok_or_else(|| std::io::Error::other("stop signal stream closed"))
        };
        #[cfg(not(unix))]
        let result = tokio::signal::ctrl_c().await.map(|()| "Ctrl-C");
        shutdown.stop();
        result
    }
}
