//! Probing a kkt server and reading its key files: [`Prober`], with what it
//! finds ([`KeyFile`]) and the checks it gives back ([`Check`]), and its one
//! implementation, `HttpProber` (behind `real`), which probes over HTTP with
//! reqwest, whose types stay inside this crate. kkt-diagnostics turns the
//! checks into `kkt doctor`'s and setup's, depending on this crate without
//! `real`, so it sees the contract alone; the program turns `real` on and
//! injects `HttpProber`, with the [`Settings`] it chooses.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

/// What a [`Prober`] found in a key file.
#[derive(Debug, Clone)]
pub struct KeyFile {
    /// The file's permission bits, where the system has them.
    pub mode: Option<u32>,
    /// The file's text, or why it could not be read.
    pub text: Result<String, String>,
}

/// The server checks a [`Prober`] makes, once they are done.
pub type Probing<'a> = Pin<Box<dyn Future<Output = Result<Vec<Check>, String>> + Send + 'a>>;

/// What the checks do to the machine. Neither registers an agent nor
/// changes a server's database.
pub trait Prober: Sync {
    /// What the key file at `path` holds. `Err` says why the file could not
    /// be looked at.
    fn key_file(&self, path: &Path) -> Result<KeyFile, String>;

    /// The checks of the server at `url`, using `key` (`None` for none):
    /// whether it answers, accepts its host name, accepts the key and serves
    /// MCP, and its build. `Err` when the probe cannot be made at all.
    fn probe<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a>;
}

/// One check and its outcome: its name, whether it passed, what was found,
/// and for a problem, how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

/// What the HTTP prober takes from the program; it chooses no defaults.
#[derive(Clone, Copy)]
pub struct Settings {
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub protocol_version: &'static str,
    pub server_check: &'static str,
    pub key_check: &'static str,
}

#[cfg(feature = "real")]
mod http;
#[cfg(feature = "real")]
pub use http::HttpProber;
