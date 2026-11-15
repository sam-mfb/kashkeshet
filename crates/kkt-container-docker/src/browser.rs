//! What this crate needs from the person's browser, to open a sharing
//! setup's login link: start opening it now, and say later, once, if that
//! failed. Starting does not wait for the browser. The program supplies it.

use std::io;

/// The person's browser.
pub trait Browser {
    /// Starts opening `url`. Fails if it cannot be started; whether it then
    /// worked is told by `open_failure`.
    fn open(&self, url: &str) -> io::Result<()>;
    /// Why opening ended without success, if it did since the last time this
    /// was asked. Never repeats the link, which may hold a secret.
    fn open_failure(&self) -> Option<String>;
}
