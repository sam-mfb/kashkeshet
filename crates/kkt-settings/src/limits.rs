//! The limits a server runs with: every one an operator can set, their
//! defaults, and the checks they must pass together.

use kkt_config::InvalidSetting;
use kkt_format::MIN_RESPONSE_BYTES;
use kkt_web::MAX_WEB_PAGE_MESSAGES;
use std::time::Duration;

/// Every limit a server runs with, as gathered from flags, the config file
/// and the defaults. The program gives each part its own share of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Word pack used for new avatars' handles.
    pub name_pack: kkt_names::NamePack,
    /// Largest response, in bytes, before the server cuts it and says how to get the rest.
    pub max_response_bytes: usize,
    /// Items returned by list and read tools when the agent gives no `limit`.
    pub default_limit: usize,
    /// Largest `limit` an agent may ask for.
    pub max_limit: usize,
    /// Largest message body, in bytes.
    pub max_body_bytes: usize,
    /// Longest channel name, in characters.
    pub max_channel_name_chars: usize,
    /// Longest channel purpose, in characters.
    pub max_purpose_chars: usize,
    /// Longest agent description, in characters.
    pub max_description_chars: usize,
    /// Longest pinned channel note, in characters.
    pub max_note_chars: usize,
    /// Longest server announcement, in characters.
    pub max_server_announcement_chars: usize,
    /// Longest entry an avatar keeps in its memory, in characters.
    pub max_memory_entry_chars: usize,
    /// Most characters an avatar keeps in its memory, all entries together.
    pub max_memory_chars: usize,
    /// Longest lane instructions, in characters.
    pub max_lane_chars: usize,
    /// Messages one agent may send per minute.
    pub sends_per_minute: u32,
    /// Agents the whole server may register per minute.
    pub registrations_per_minute: u32,
    /// Most agents that may exist at once, not counting revoked ones.
    pub max_agents: usize,
    /// How long `wait_for_messages` waits when the agent gives no `timeout_s`.
    pub default_wait: Duration,
    /// Longest wait an agent may ask for.
    pub max_wait: Duration,
    /// How long a normally completed wait remains visible as listening.
    pub waiting_grace: Duration,
    /// Member names shown with each channel in `list_channels`.
    pub member_preview: usize,
    /// Characters of a channel purpose shown in `list_channels`.
    pub purpose_preview_chars: usize,
    /// How long a web page sign-in lasts, when an access key is set.
    pub web_session_lifetime: Duration,
    /// The most messages one web page shows at first: a channel's page, and
    /// the live view, which drops the oldest as new ones arrive. A channel
    /// page's "Show previous" button loads this many earlier ones at a time.
    pub web_page_messages: usize,
    /// Characters of each member's latest message that a channel's web page
    /// shows in its member list, as an excerpt linking to the whole message.
    pub web_member_excerpt_chars: usize,
    /// How many of the backups the server makes before upgrading the
    /// database it keeps; older ones are removed.
    pub upgrade_backups_kept: usize,
}

impl Limits {
    pub const DEFAULT: Limits = Limits {
        name_pack: kkt_names::ANIMALS,
        max_response_bytes: 8 * 1024,
        default_limit: 20,
        max_limit: 100,
        max_body_bytes: 8 * 1024,
        max_channel_name_chars: 64,
        max_purpose_chars: 500,
        max_description_chars: 300,
        max_note_chars: 10_000,
        max_server_announcement_chars: 4000,
        // As long as the handoff note was, so each one fits as an entry.
        max_memory_entry_chars: 20_000,
        max_memory_chars: 100_000,
        max_lane_chars: 8000,
        sends_per_minute: 30,
        registrations_per_minute: 20,
        max_agents: 1000,
        default_wait: Duration::from_secs(30),
        // Common MCP clients give up on a tool call after about 60 seconds.
        max_wait: Duration::from_secs(50),
        waiting_grace: Duration::from_secs(3),
        member_preview: 3,
        purpose_preview_chars: 120,
        web_session_lifetime: Duration::from_secs(12 * 60 * 60),
        web_page_messages: 50,
        web_member_excerpt_chars: 80,
        upgrade_backups_kept: 5,
    };

    /// Checks that these limits make sense together. The server refuses to
    /// start with limits that fail.
    pub fn check(&self) -> Result<(), InvalidSetting> {
        let at_least_one = [
            ("default_limit", self.default_limit),
            ("max_body_bytes", self.max_body_bytes),
            ("max_channel_name_chars", self.max_channel_name_chars),
            ("max_purpose_chars", self.max_purpose_chars),
            ("max_description_chars", self.max_description_chars),
            ("max_note_chars", self.max_note_chars),
            (
                "max_server_announcement_chars",
                self.max_server_announcement_chars,
            ),
            ("max_memory_entry_chars", self.max_memory_entry_chars),
            ("max_memory_chars", self.max_memory_chars),
            ("max_lane_chars", self.max_lane_chars),
            ("sends_per_minute", self.sends_per_minute as usize),
            (
                "registrations_per_minute",
                self.registrations_per_minute as usize,
            ),
            ("max_agents", self.max_agents),
            ("web_page_messages", self.web_page_messages),
            ("web_member_excerpt_chars", self.web_member_excerpt_chars),
            ("upgrade_backups_kept", self.upgrade_backups_kept),
        ];
        if let Some((name, _)) = at_least_one.iter().find(|(_, value)| *value == 0) {
            return Err(InvalidSetting::new(name, "must be at least 1"));
        }
        if self.max_response_bytes < MIN_RESPONSE_BYTES {
            return Err(InvalidSetting::new(
                "max_response_bytes",
                format!("must be at least {MIN_RESPONSE_BYTES}"),
            ));
        }
        if self.max_limit < self.default_limit {
            return Err(InvalidSetting::new(
                "max_limit",
                "must be at least default_limit",
            ));
        }
        if self.max_memory_chars < self.max_memory_entry_chars {
            return Err(InvalidSetting::new(
                "max_memory_chars",
                "must be at least max_memory_entry_chars",
            ));
        }
        if self.web_page_messages > MAX_WEB_PAGE_MESSAGES {
            return Err(InvalidSetting::new(
                "web_page_messages",
                format!("must be at most {MAX_WEB_PAGE_MESSAGES}"),
            ));
        }
        if self.web_session_lifetime < Duration::from_secs(1) {
            return Err(InvalidSetting::new(
                "web_session_lifetime",
                "must be at least 1 second",
            ));
        }
        if self.waiting_grace > self.max_wait {
            return Err(InvalidSetting::new(
                "waiting_grace",
                "must not exceed max_wait",
            ));
        }
        if self.max_wait < self.default_wait {
            return Err(InvalidSetting::new(
                "max_wait",
                "must be at least default_wait",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_name_pack_is_the_one_names_draws_from() {
        // `names::agent_handle` draws from `ANIMALS` without asking config.
        assert_eq!(Limits::DEFAULT.name_pack, kkt_names::ANIMALS);
    }

    #[test]
    fn web_pages_have_a_ceiling() {
        let limits = Limits {
            web_page_messages: MAX_WEB_PAGE_MESSAGES + 1,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check().map_err(|e| e.name), Err("web_page_messages"));
        let limits = Limits {
            web_page_messages: MAX_WEB_PAGE_MESSAGES,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check(), Ok(()));
    }

    #[test]
    fn default_limits_pass() {
        assert_eq!(Limits::DEFAULT.check(), Ok(()));
    }

    #[test]
    fn zero_is_refused_for_every_count() {
        type SetZero = fn(&mut Limits);
        let cases: [(&str, SetZero); 15] = [
            ("default_limit", |l| l.default_limit = 0),
            ("max_body_bytes", |l| l.max_body_bytes = 0),
            ("max_channel_name_chars", |l| l.max_channel_name_chars = 0),
            ("max_purpose_chars", |l| l.max_purpose_chars = 0),
            ("max_description_chars", |l| l.max_description_chars = 0),
            ("max_note_chars", |l| l.max_note_chars = 0),
            ("max_server_announcement_chars", |l| {
                l.max_server_announcement_chars = 0
            }),
            ("max_memory_entry_chars", |l| l.max_memory_entry_chars = 0),
            ("max_memory_chars", |l| l.max_memory_chars = 0),
            ("max_lane_chars", |l| l.max_lane_chars = 0),
            ("sends_per_minute", |l| l.sends_per_minute = 0),
            ("registrations_per_minute", |l| {
                l.registrations_per_minute = 0
            }),
            ("max_agents", |l| l.max_agents = 0),
            ("web_page_messages", |l| l.web_page_messages = 0),
            ("web_member_excerpt_chars", |l| {
                l.web_member_excerpt_chars = 0
            }),
        ];
        for (name, set_zero) in cases {
            let mut limits = Limits::DEFAULT;
            set_zero(&mut limits);
            let err = limits.check().expect_err(name);
            assert_eq!(err.name, name);
        }
    }

    #[test]
    fn an_avatars_memory_holds_at_least_one_whole_entry() {
        let limits = Limits {
            max_memory_entry_chars: 500,
            max_memory_chars: 499,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check().map_err(|e| e.name), Err("max_memory_chars"));
        let limits = Limits {
            max_memory_entry_chars: 500,
            max_memory_chars: 500,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check(), Ok(()));
    }

    #[test]
    fn response_cap_has_a_floor() {
        let limits = Limits {
            max_response_bytes: MIN_RESPONSE_BYTES - 1,
            ..Limits::DEFAULT
        };
        assert_eq!(
            limits.check().map_err(|e| e.name),
            Err("max_response_bytes")
        );
        let limits = Limits {
            max_response_bytes: MIN_RESPONSE_BYTES,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check(), Ok(()));
    }

    #[test]
    fn maximums_may_not_be_below_defaults() {
        let limits = Limits {
            default_limit: 10,
            max_limit: 9,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check().map_err(|e| e.name), Err("max_limit"));
        let limits = Limits {
            default_limit: 10,
            max_limit: 10,
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check(), Ok(()));

        let limits = Limits {
            default_wait: Duration::from_secs(10),
            max_wait: Duration::from_secs(9),
            ..Limits::DEFAULT
        };
        assert_eq!(limits.check().map_err(|e| e.name), Err("max_wait"));
    }

    #[test]
    fn web_sessions_last_at_least_a_second() {
        let limits = Limits {
            web_session_lifetime: Duration::from_millis(999),
            ..Limits::DEFAULT
        };
        assert_eq!(
            limits.check().map_err(|e| e.name),
            Err("web_session_lifetime")
        );
    }
}
