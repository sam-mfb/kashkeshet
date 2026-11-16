//! A signal that the server is stopping ([`Shutdown`]), and the contract for
//! a source of stop signals ([`Signals`]), which stops it when one comes.
//!
//! Tokio-util's cancellation token does the work, but stays inside this
//! module: no other part sees it. Parts that need a library's own token make
//! one and cancel it when [`Shutdown::stopped`] finishes.

use std::fmt;
use std::future::Future;
use std::io;
use tokio_util::sync::CancellationToken;

/// Clones share one signal: stopping any clone stops them all.
#[derive(Clone, Debug)]
pub struct Shutdown {
    token: CancellationToken,
}

impl Shutdown {
    pub fn new() -> Shutdown {
        Shutdown {
            token: CancellationToken::new(),
        }
    }

    /// A signal that stops when this one does, and can also be stopped on
    /// its own without stopping this one.
    pub fn child(&self) -> Shutdown {
        Shutdown {
            token: self.token.child_token(),
        }
    }

    /// Tells everything waiting on this signal to stop.
    pub fn stop(&self) {
        self.token.cancel();
    }

    pub fn is_stopped(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Finishes once the signal is stopped.
    pub async fn stopped(&self) {
        self.token.cancelled().await
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Shutdown::new()
    }
}

/// A stop signal that came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Ctrl-C (SIGINT on Unix).
    Interrupt,
    /// SIGTERM, on Unix.
    Terminate,
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Signal::Interrupt => "Ctrl-C",
            Signal::Terminate => "SIGTERM",
        })
    }
}

/// A source of stop signals, already listening when it is made, so a
/// failure to listen is reported before the server starts.
pub trait Signals {
    /// Waits for a stop signal, then stops `shutdown` and says which signal
    /// came. `shutdown` is stopped even when waiting fails.
    fn stop_on_signal(self, shutdown: Shutdown) -> impl Future<Output = io::Result<Signal>> + Send;
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_are_named_as_people_know_them() {
        assert_eq!(Signal::Interrupt.to_string(), "Ctrl-C");
        assert_eq!(Signal::Terminate.to_string(), "SIGTERM");
    }

    #[tokio::test]
    async fn the_stand_in_stops_the_shutdown_when_its_signal_comes() {
        let cue = Shutdown::new();
        let shutdown = Shutdown::new();
        let signals = testing::OnCue {
            cue: cue.clone(),
            signal: Signal::Terminate,
        };
        let waiting = tokio::spawn(signals.stop_on_signal(shutdown.clone()));
        assert!(!shutdown.is_stopped());
        cue.stop();
        assert_eq!(
            waiting.await.expect("join").expect("a signal"),
            Signal::Terminate
        );
        assert!(shutdown.is_stopped());
    }

    #[tokio::test]
    async fn stopping_a_clone_stops_the_original() {
        let shutdown = Shutdown::new();
        let clone = shutdown.clone();
        assert!(!shutdown.is_stopped());
        clone.stop();
        assert!(shutdown.is_stopped());
        shutdown.stopped().await;
    }

    #[tokio::test]
    async fn a_child_stops_with_its_parent_but_not_the_reverse() {
        let parent = Shutdown::new();
        let child = parent.child();
        let other = parent.child();
        other.stop();
        assert!(!parent.is_stopped());
        assert!(!child.is_stopped());
        parent.stop();
        child.stopped().await;
        assert!(child.is_stopped());
    }
}
