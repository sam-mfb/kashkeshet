//! Probing a kkt server over HTTP, and reading its key files: [`HttpProber`]
//! implements kkt-probe's `Prober` with reqwest, whose types stay inside
//! this crate. The program injects it, with its [`Settings`].

use std::time::Duration;

/// Values supplied by the program; the prober chooses no defaults.
#[derive(Clone, Copy)]
pub struct Settings {
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub protocol_version: &'static str,
    pub server_check: &'static str,
    pub key_check: &'static str,
}

#[cfg(feature = "real")]
mod probe;
#[cfg(feature = "real")]
pub use probe::HttpProber;
