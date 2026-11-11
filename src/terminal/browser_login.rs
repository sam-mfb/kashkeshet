//! Adapting the native browser link to Docker setup's login-link contract.

use kkt_container::LoginLink;
use kkt_terminal_os::BrowserLink;
use std::io;

/// The browser link supplied to Docker setup.
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
    fn open(&self, url: &str) -> io::Result<()> {
        self.0.open(url)
    }
    fn open_failure(&self) -> Option<String> {
        self.0.open_failure()
    }
}
