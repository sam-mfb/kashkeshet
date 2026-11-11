//! The machine's key files and diagnostic HTTP probe. The program maps
//! these records to the diagnostics logic's own contract.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct KeyFile {
    pub mode: Option<u32>,
    pub text: Result<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

/// Values supplied by the program; the effect chooses no defaults.
#[derive(Clone, Copy)]
pub struct Settings {
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub protocol_version: &'static str,
    pub server_check: &'static str,
    pub key_check: &'static str,
}

pub type Probing<'a> = Pin<Box<dyn Future<Output = Result<Vec<Check>, String>> + Send + 'a>>;

#[cfg(feature = "real")]
mod probe;
#[cfg(feature = "real")]
pub use probe::OsProber;

#[cfg(feature = "real")]
impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }

    fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}
