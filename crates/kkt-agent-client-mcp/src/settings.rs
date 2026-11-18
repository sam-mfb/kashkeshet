//! What the watch asks for at a time, and how often: plain values.

/// How much the watch asks for at a time. [`Mcp`](crate::Mcp) watches with
/// [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How many messages, or channels, each call asks for.
    pub watch_batch: u32,
}

impl Settings {
    /// 100 at a time.
    pub const DEFAULT: Settings = Settings { watch_batch: 100 };
}
