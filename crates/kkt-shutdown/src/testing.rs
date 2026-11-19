//! A stand-in [`Signals`]: a signal that comes when the test says.

use crate::{Shutdown, Signal, Signals};
use std::io;

/// Stop signals that come on cue: once `cue` is stopped, `signal` comes.
pub struct OnCue {
    pub cue: Shutdown,
    pub signal: Signal,
}

impl Signals for OnCue {
    async fn stop_on_signal(self, shutdown: Shutdown) -> io::Result<Signal> {
        self.cue.stopped().await;
        shutdown.stop();
        Ok(self.signal)
    }
}
