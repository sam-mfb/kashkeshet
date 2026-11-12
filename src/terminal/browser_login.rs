//! Adapting the terminal's half of the login link to Docker setup's
//! login-link contract. Opening the link is kkt-browser's `SystemBrowser`,
//! which the program gives the Docker wrapper.

use kkt_container::LoginLink;
use kkt_terminal_os::BrowserLink;

/// The terminal's login link supplied to Docker setup.
pub struct TerminalLink(BrowserLink);

impl TerminalLink {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(BrowserLink::new())
    }
}

impl LoginLink for TerminalLink {
    fn show(&self, url: &str) {
        self.0.show(url);
    }
    fn open_requested(&self) -> bool {
        self.0.open_requested()
    }
}
