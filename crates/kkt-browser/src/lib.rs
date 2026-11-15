//! Opening a link in the person's browser with the system's own opener:
//! `SystemBrowser` (behind `real`), which starts opening a link now and
//! says later, once, if that failed. Starting does not wait for the
//! browser. It declares no contract: the program adapts `SystemBrowser` to
//! the `Browser` kkt-container-docker declares.

#[cfg(any(feature = "real", test))]
mod system;
#[cfg(feature = "real")]
pub use system::SystemBrowser;
