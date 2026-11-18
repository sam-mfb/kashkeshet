//! Adapts kkt-browser's system opener to the browsers a sharing setup's
//! login link opens in: kkt-containerized-service's, and kkt-container-
//! docker's.

use kkt_browser::SystemBrowser;
use kkt_container_docker::Browser;
use std::io;

/// The person's browser, through the system's own opener.
pub struct OpenerBrowser(SystemBrowser);

impl OpenerBrowser {
    pub fn new() -> OpenerBrowser {
        OpenerBrowser(SystemBrowser::new())
    }
}

impl Browser for OpenerBrowser {
    fn open(&self, url: &str) -> io::Result<()> {
        self.0.open(url)
    }
    fn open_failure(&self) -> Option<String> {
        self.0.open_failure()
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
