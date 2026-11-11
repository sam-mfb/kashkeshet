//! What the watch asks for at a time, and how often: plain values, outside
//! `real`.

use std::time::Duration;

/// How much the watch asks for at a time, and how often it asks the admin
/// endpoint. The program passes [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How many messages, or channels, each call asks for.
    pub watch_batch: u32,
    /// How often `kkt watch --admin` asks the admin endpoint, which does not
    /// wait, for new messages while none come.
    pub watch_admin_poll: Duration,
}

impl Settings {
    /// 100 at a time; the admin endpoint is asked every 2 seconds.
    pub const DEFAULT: Settings = Settings {
        watch_batch: 100,
        watch_admin_poll: Duration::from_secs(2),
    };
}
