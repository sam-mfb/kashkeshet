//! Why an operation of the containerized service failed, as a person reads
//! it.

use std::fmt;

/// Why a [`Container`](crate::Container) operation failed: a message for
/// the person running kkt, which names what failed and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerError(String);

impl ContainerError {
    /// The failure worded as `message`.
    pub fn new(message: impl Into<String>) -> ContainerError {
        ContainerError(message.into())
    }
    /// The message, as the person reads it.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl From<String> for ContainerError {
    fn from(message: String) -> ContainerError {
        ContainerError(message)
    }
}

impl From<&str> for ContainerError {
    fn from(message: &str) -> ContainerError {
        ContainerError(message.to_string())
    }
}

/// The message, for callers that word their own failures as text.
impl From<ContainerError> for String {
    fn from(error: ContainerError) -> String {
        error.0
    }
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContainerError {}
