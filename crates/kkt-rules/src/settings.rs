//! The rules' own settings: the limits they enforce and the defaults they
//! fill in.

use std::time::Duration;

/// What the rules enforce and fill in: the sizes of what agents and the
/// operator save, page sizes, rates, waits, and the word pack for new
/// handles. The program fills it from the server's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Word pack used for new avatars' handles.
    pub name_pack: kkt_names::NamePack,
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
    /// The longest lane description, in characters. It is one line.
    pub max_lane_description_chars: usize,
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
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        name_pack: kkt_names::ANIMALS,
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
        max_lane_description_chars: 200,
        sends_per_minute: 30,
        registrations_per_minute: 20,
        max_agents: 1000,
        default_wait: Duration::from_secs(30),
        // Common MCP clients give up on a tool call after about 60 seconds.
        max_wait: Duration::from_secs(50),
        waiting_grace: Duration::from_secs(3),
        member_preview: 3,
        purpose_preview_chars: 120,
    };

    /// Clamps an agent's requested `limit` to `1..=max_limit`, using the default when absent.
    pub fn limit(&self, requested: Option<u32>) -> usize {
        match requested {
            Some(n) => (n as usize).clamp(1, self.max_limit),
            None => self.default_limit,
        }
    }
}
