//! Why one of the steps setup asks its caller to take failed, and why the
//! check of an https address failed.

/// A step's failure: the caller's own error, passed on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StepError(pub Box<dyn std::error::Error + Send + Sync>);

/// Why the check of an https address failed ([`crate::Steps::check_https_address`]),
/// as its first request found it.
#[derive(Debug, thiserror::Error)]
pub enum HttpsRefused {
    /// There is no https there at all: nothing listens, or what answers
    /// does not speak TLS. The problems found.
    #[error(transparent)]
    Absent(StepError),
    /// Anything else, such as an untrusted certificate, or an https server
    /// that fails the check. The problems found.
    #[error(transparent)]
    Failed(StepError),
}
