//! Probing a kkt server over HTTP and reading its key files, with reqwest,
//! whose types stay inside this crate: `HttpProber` (behind `real`), with
//! the [`Settings`] the program chooses. It words what it finds as
//! [`Finding`]s and [`FileRead`]s, its own records. It declares no
//! contract: the program adapts `HttpProber` to the `Prober` kkt-diagnostics
//! declares.

use std::time::Duration;

/// What [`HttpProber`] found in a key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    /// The file's permission bits, where the system has them.
    pub mode: Option<u32>,
    /// The file's text, or why it could not be read.
    pub text: Result<String, String>,
}

/// One thing a probe found: the check's name, whether it passed, what was
/// found, and for a problem, how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

impl Finding {
    fn ok(name: &'static str, detail: impl Into<String>) -> Finding {
        Finding {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }

    fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Finding {
        Finding {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// What the HTTP prober takes from the program; it chooses no defaults.
#[derive(Clone, Copy)]
pub struct Settings {
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub protocol_version: &'static str,
    pub server_check: &'static str,
    pub key_check: &'static str,
}

mod http;
pub use http::HttpProber;
