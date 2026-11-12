//! A stand-in browser, for tests of the crates that open links: it records
//! each link, can refuse to start, and can report a later failure once.

use super::Browser;
use std::cell::{Cell, RefCell};
use std::io;

#[derive(Default)]
pub struct Recorder {
    /// Each link asked to be opened, in order.
    pub opened: RefCell<Vec<String>>,
    /// Every `open` fails to start.
    pub open_fails: Cell<bool>,
    /// What `open_failure` answers once.
    pub later_failure: RefCell<Option<String>>,
}

impl Browser for Recorder {
    fn open(&self, url: &str) -> io::Result<()> {
        self.opened.borrow_mut().push(url.to_string());
        match self.open_fails.get() {
            true => Err(io::Error::new(io::ErrorKind::NotFound, "no opener")),
            false => Ok(()),
        }
    }
    fn open_failure(&self) -> Option<String> {
        self.later_failure.borrow_mut().take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_records_links_and_reports_a_later_failure_once() {
        let browser = Recorder::default();
        browser.open("https://a").expect("started");
        browser.open_fails.set(true);
        assert!(browser.open("https://b").is_err());
        *browser.later_failure.borrow_mut() = Some("the opener ended with 1".into());
        assert_eq!(
            browser.open_failure().as_deref(),
            Some("the opener ended with 1")
        );
        assert_eq!(browser.open_failure(), None);
        assert_eq!(*browser.opened.borrow(), ["https://a", "https://b"]);
    }
}
