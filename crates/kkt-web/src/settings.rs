//! The page's own settings.

use std::time::Duration;

/// Largest `web_page_messages`. More would make a page slow to load and
/// heavy to keep live.
pub const MAX_WEB_PAGE_MESSAGES: usize = 5000;

/// How much the web page shows at once, how it keeps its event stream
/// open, and how long a sign-in lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The most messages one web page shows at first: a channel's page, and
    /// the live view, which drops the oldest as new ones arrive. A channel
    /// page's "Show previous" button loads this many earlier ones at a time.
    pub page_messages: usize,
    /// Characters of each member's latest message that a channel's web page
    /// shows in its member list, as an excerpt linking to the whole message.
    pub member_excerpt_chars: usize,
    /// How often the web page's event stream sends a keep-alive comment.
    pub keep_alive: Duration,
    /// How long a web page sign-in lasts, when sign-in is required. Read by
    /// whoever makes the page's [`Sessions`](crate::Sessions).
    pub session_lifetime: Duration,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        page_messages: 50,
        member_excerpt_chars: 80,
        keep_alive: Duration::from_secs(15),
        session_lifetime: Duration::from_secs(12 * 60 * 60),
    };
}

/// A setting the server refuses to start with, as the preview checks it: the
/// setting's name and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingError {
    pub name: &'static str,
    pub rule: String,
}

impl SettingError {
    pub fn new(name: &'static str, rule: impl Into<String>) -> SettingError {
        SettingError {
            name,
            rule: rule.into(),
        }
    }
}

impl std::fmt::Display for SettingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.rule)
    }
}

impl std::error::Error for SettingError {}
