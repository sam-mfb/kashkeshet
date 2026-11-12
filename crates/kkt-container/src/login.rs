//! The contract for showing the network login link to the person. Showing
//! it happens at the edge of the program, so setup can be tested without a
//! terminal. Opening it in a browser, when the person asks, is the
//! implementation's (kkt-browser's `Browser`, which it is given).

/// What setup does with the login link. [`Container::provision`](super::Container::provision) shows
/// the link once, then at each check for the login asks `open_requested`, so
/// waiting for a key press never stops it watching for the login.
pub trait LoginLink {
    /// Shows the link to log in with.
    fn show(&self, url: &str);
    /// Whether the person has asked to open the link since the last time this
    /// was asked.
    fn open_requested(&self) -> bool;
}
