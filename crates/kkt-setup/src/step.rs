//! Why one of the steps setup asks its caller to take failed.

/// A step's failure: the caller's own error, passed on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StepError(pub Box<dyn std::error::Error + Send + Sync>);
