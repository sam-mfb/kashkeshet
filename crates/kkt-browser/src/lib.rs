//! Opening a link in the person's browser with the system's own opener:
//! `SystemBrowser` starts opening a link at once and says later, once, if
//! that failed. The caller goes on while the browser starts.

mod system;
pub use system::SystemBrowser;
