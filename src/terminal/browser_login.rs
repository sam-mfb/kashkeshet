//! Adapting the terminal's half of the login link to Docker setup's
//! login-link contract. Opening the link is kkt-browser's `SystemBrowser`,
//! which the program adapts to the Docker crate's `Browser`.

use kkt_containerized_service::LoginLink;
use kkt_terminal::BrowserLink;

/// The terminal's login link supplied to Docker setup.
pub struct TerminalLink(BrowserLink);

impl TerminalLink {
    #[expect(
        clippy::new_without_default,
        reason = "a handle on the terminal or the system, made by name where the program wires it; `Default` would let one be made unseen"
    )]
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
