//! Adapting the terminal's half of the login link to Docker setup's
//! login-link contract. Opening the link is kkt-browser's `SystemBrowser`,
//! which the program adapts to the Docker crate's `Browser`.

use kkt_containerized_service::LoginLink;
use kkt_terminal::BrowserLink;

/// What the person is asked to do with the Tailscale login link.
const LOGIN_LEAD: &str = "Log in to Tailscale by opening";

/// The terminal's login link supplied to Docker setup.
pub struct TerminalLink(BrowserLink);

impl TerminalLink {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(BrowserLink::new(LOGIN_LEAD.to_string()))
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_login_link_keeps_its_words() {
        assert_eq!(super::LOGIN_LEAD, "Log in to Tailscale by opening");
    }
}
