//! What tests outside kkt-admission need to know about its keys.

/// Shortest access key, in bytes: a key this long, and otherwise valid, is
/// one [`check_access_key`](crate::check_access_key) accepts.
pub const MIN_ACCESS_KEY_BYTES: usize = crate::keys::MIN_ACCESS_KEY_BYTES;
