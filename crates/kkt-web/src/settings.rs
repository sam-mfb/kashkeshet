//! The page's own settings.

/// How much the web page shows at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The most messages one web page shows at first: a channel's page, and
    /// the live view, which drops the oldest as new ones arrive. A channel
    /// page's "Show previous" button loads this many earlier ones at a time.
    pub page_messages: usize,
    /// Characters of each member's latest message that a channel's web page
    /// shows in its member list, as an excerpt linking to the whole message.
    pub member_excerpt_chars: usize,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        page_messages: 50,
        member_excerpt_chars: 80,
    };
}
