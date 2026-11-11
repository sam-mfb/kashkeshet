//! What other crates' tests read of the defaults, which are otherwise
//! private.

/// Whether `bind` may be a non-loopback address, unless a setting says so.
pub const DEFAULT_ALLOW_ANY_BIND: bool = super::DEFAULT_ALLOW_ANY_BIND;
