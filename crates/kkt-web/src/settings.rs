//! The page's own settings.

use std::time::Duration;

/// Largest `web_page_messages`. More would make a page slow to load and
/// heavy to keep live.
pub const MAX_WEB_PAGE_MESSAGES: usize = 5000;

/// How much the web page shows at once, how it keeps its event stream
/// open, and how long a sign-in lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How much the page shows at once.
    pub page: PageLimits,
    /// How often the web page's event stream sends a keep-alive comment.
    pub keep_alive: Duration,
    /// How long a web page sign-in lasts, when sign-in is required. Read by
    /// whoever makes the page's [`Sessions`](crate::Sessions).
    pub session_lifetime: Duration,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        page: PageLimits::DEFAULT,
        keep_alive: Duration::from_secs(15),
        session_lifetime: Duration::from_secs(12 * 60 * 60),
    };
}

/// The limits only the page applies: how many messages a page shows and
/// how much of a member's latest message. The limits the application
/// enforces, such as the announcement's and lanes' sizes, are its own
/// (`Api::limits`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageLimits {
    /// The most messages one web page shows at first: a channel's page, and
    /// the live view, which drops the oldest as new ones arrive. A channel
    /// page's "Show previous" button loads this many earlier ones at a time.
    pub page_messages: usize,
    /// Characters of each member's latest message that a channel's web page
    /// shows in its member list, as an excerpt linking to the whole message.
    pub member_excerpt_chars: usize,
}

impl PageLimits {
    pub const DEFAULT: PageLimits = PageLimits {
        page_messages: 50,
        member_excerpt_chars: 80,
    };
}
