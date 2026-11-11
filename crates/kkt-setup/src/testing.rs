//! What tests outside kkt-setup need to know about its questions.

/// The client-only server address Enter accepts during interactive setup.
pub const DEFAULT_SETUP_URL: &str = crate::flow::DEFAULT_SETUP_URL;
/// The client-only access key Enter accepts (no key).
pub const DEFAULT_SETUP_KEY: &str = crate::flow::DEFAULT_SETUP_KEY;
