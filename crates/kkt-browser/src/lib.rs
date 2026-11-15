//! Opening a link in the person's browser with the system's own opener:
//! `SystemBrowser`, which starts opening a link now and
//! says later, once, if that failed. Starting does not wait for the
//! browser. It declares no contract: the program adapts `SystemBrowser` to
//! the `Browser` kkt-container-docker declares.

mod system;
pub use system::SystemBrowser;
