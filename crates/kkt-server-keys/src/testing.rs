//! What tests outside kkt-server-keys need to know about the keys.

/// Shortest access key, in bytes: a key this long, and otherwise valid, is
/// one [`check_access_key`](crate::check_access_key) accepts.
pub const MIN_ACCESS_KEY_BYTES: usize = crate::MIN_ACCESS_KEY_BYTES;
