//! The operating system's stop signals. [`StopSignals`] implements
//! kkt-shutdown's `Signals`: installed before the server is ready, it stops
//! the shared `Shutdown` value on receipt. Tokio's signal types stay inside
//! this crate.

use kkt_shutdown::{Shutdown, Signal, Signals};

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
}

impl Signals for StopSignals {
    /// Waits for Ctrl-C, or SIGTERM on Unix, and stops the shared signal.
    async fn stop_on_signal(self, shutdown: Shutdown) -> std::io::Result<Signal> {
        #[cfg(unix)]
        let result = {
            let mut signals = self;
            let (received, signal) = tokio::select! {
                received = signals.interrupt.recv() => (received, Signal::Interrupt),
                received = signals.terminate.recv() => (received, Signal::Terminate),
            };
            received
                .map(|()| signal)
                .ok_or_else(|| std::io::Error::other("stop signal stream closed"))
        };
        #[cfg(not(unix))]
        let result = tokio::signal::ctrl_c().await.map(|()| Signal::Interrupt);
        shutdown.stop();
        result
    }
}
