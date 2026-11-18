//! Adapts kkt-browser's system opener to the browser a sharing setup's
//! login link opens in: kkt-containerized-service's.

use kkt_browser::SystemBrowser;
use std::io;

/// The person's browser, through the system's own opener.
pub struct OpenerBrowser(SystemBrowser);

impl OpenerBrowser {
    pub fn new() -> OpenerBrowser {
        OpenerBrowser(SystemBrowser::new())
    }
}

impl kkt_containerized_service::Browser for OpenerBrowser {
    fn open(&self, url: &str) -> io::Result<()> {
        self.0.open(url)
    }
    fn open_failure(&self) -> Option<String> {
        self.0.open_failure()
    }
}
