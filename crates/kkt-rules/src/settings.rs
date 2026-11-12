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

    /// Every setting these settings hold that the rules cannot run with, in
    /// the order the rules check them: first each count that must be at
    /// least 1 (its `floor`), then each bound against another setting
    /// (named by that setting). A setting may be named twice, once for each
    /// check it fails. Empty when the rules can run with them all.
    pub fn problems(&self) -> Vec<Problem> {
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
        ];
        let mut problems: Vec<Problem> = at_least_one
            .into_iter()
            .filter(|(_, value)| *value == 0)
            .map(|(name, _)| Problem::new(name, FLOOR, "must be at least 1"))
            .collect();
        let bounds = [
            (
                self.max_limit < self.default_limit,
                "max_limit",
                "default_limit",
                "must be at least default_limit",
            ),
            (
                self.max_memory_chars < self.max_memory_entry_chars,
                "max_memory_chars",
                "max_memory_entry_chars",
                "must be at least max_memory_entry_chars",
            ),
            (
                self.waiting_grace > self.max_wait,
                "waiting_grace",
                "max_wait",
                "must not exceed max_wait",
            ),
            (
                self.max_wait < self.default_wait,
                "max_wait",
                "default_wait",
                "must be at least default_wait",
            ),
        ];
        problems.extend(
            bounds
                .into_iter()
                .filter(|(broken, ..)| *broken)
                .map(|(_, name, check, rule)| Problem::new(name, check, rule)),
        );
        problems
    }

    /// The run record's lines for these settings, a name and a value each,
    /// in the order the record has them: the name pack by name and the
    /// waits in milliseconds. `max_lane_description_chars` is fixed, not a
    /// setting, so it is not recorded.
    pub fn record_lines(&self) -> Vec<(String, String)> {
        // Listing every field, with no `..`, makes a new setting fail to
        // compile here until it is recorded or said not to be.
        let Settings {
            name_pack,
            default_limit,
            max_limit,
            max_body_bytes,
            max_channel_name_chars,
            max_purpose_chars,
            max_description_chars,
            max_note_chars,
            max_server_announcement_chars,
            max_memory_entry_chars,
            max_memory_chars,
            max_lane_chars,
            max_lane_description_chars: _,
            sends_per_minute,
            registrations_per_minute,
            max_agents,
            default_wait,
            max_wait,
            waiting_grace,
            member_preview,
            purpose_preview_chars,
        } = self;
        vec![
            line("name_pack", name_pack.name()),
            line("default_limit", default_limit),
            line("max_limit", max_limit),
            line("max_body_bytes", max_body_bytes),
            line("max_channel_name_chars", max_channel_name_chars),
            line("max_purpose_chars", max_purpose_chars),
            line("max_description_chars", max_description_chars),
            line("max_note_chars", max_note_chars),
            line(
                "max_server_announcement_chars",
                max_server_announcement_chars,
            ),
            line("max_memory_entry_chars", max_memory_entry_chars),
            line("max_memory_chars", max_memory_chars),
            line("max_lane_chars", max_lane_chars),
            line("sends_per_minute", sends_per_minute),
            line("registrations_per_minute", registrations_per_minute),
            line("max_agents", max_agents),
            line("default_wait_ms", default_wait.as_millis()),
            line("max_wait_ms", max_wait.as_millis()),
            line("waiting_grace_ms", waiting_grace.as_millis()),
            line("member_preview", member_preview),
            line("purpose_preview_chars", purpose_preview_chars),
        ]
    }

    /// Clamps an agent's requested `limit` to `1..=max_limit`, using the default when absent.
    pub fn limit(&self, requested: Option<u32>) -> usize {
        match requested {
            Some(n) => (n as usize).clamp(1, self.max_limit),
            None => self.default_limit,
        }
    }
}

/// The name of the check that a count must be at least 1.
pub const FLOOR: &str = "floor";

/// A setting the rules cannot run with: its name, which check found it (its
/// `floor`, or the other setting it is bounded by), and the rule it breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub name: &'static str,
    pub check: &'static str,
    pub rule: String,
}

impl Problem {
    fn new(name: &'static str, check: &'static str, rule: impl Into<String>) -> Problem {
        Problem {
            name,
            check,
            rule: rule.into(),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.rule)
    }
}

fn line(name: &str, value: impl std::fmt::Display) -> (String, String) {
    (name.to_string(), value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(settings: &Settings) -> Vec<(&'static str, &'static str)> {
        settings
            .problems()
            .iter()
            .map(|p| (p.name, p.check))
            .collect()
    }

    #[test]
    fn the_defaults_have_no_problem() {
        assert_eq!(Settings::DEFAULT.problems(), []);
    }

    #[test]
    fn every_count_must_be_at_least_one() {
        let counts: [(&str, fn(&mut Settings)); 13] = [
            ("default_limit", |s| s.default_limit = 0),
            ("max_body_bytes", |s| s.max_body_bytes = 0),
            ("max_channel_name_chars", |s| s.max_channel_name_chars = 0),
            ("max_purpose_chars", |s| s.max_purpose_chars = 0),
            ("max_description_chars", |s| s.max_description_chars = 0),
            ("max_note_chars", |s| s.max_note_chars = 0),
            ("max_server_announcement_chars", |s| {
                s.max_server_announcement_chars = 0
            }),
            ("max_memory_entry_chars", |s| {
                s.max_memory_entry_chars = 0;
                s.max_memory_chars = Settings::DEFAULT.max_memory_chars;
            }),
            ("max_memory_chars", |s| {
                s.max_memory_chars = 0;
                s.max_memory_entry_chars = 0;
            }),
            ("max_lane_chars", |s| s.max_lane_chars = 0),
            ("sends_per_minute", |s| s.sends_per_minute = 0),
            ("registrations_per_minute", |s| {
                s.registrations_per_minute = 0
            }),
            ("max_agents", |s| s.max_agents = 0),
        ];
        for (name, zero) in counts {
            let mut settings = Settings::DEFAULT;
            zero(&mut settings);
            let problems = settings.problems();
            let mine: Vec<_> = problems.iter().filter(|p| p.name == name).collect();
            assert_eq!(mine.len(), 1, "{name}: {problems:?}");
            assert_eq!(mine[0].check, FLOOR, "{name}");
            assert_eq!(mine[0].to_string(), format!("{name} must be at least 1"));
        }
    }

    #[test]
    fn settings_bounded_by_another_name_it_and_the_rule() {
        let cases: [(fn(&mut Settings), &str, &str, &str); 4] = [
            (
                |s| s.max_limit = s.default_limit - 1,
                "max_limit",
                "default_limit",
                "max_limit must be at least default_limit",
            ),
            (
                |s| s.max_memory_chars = s.max_memory_entry_chars - 1,
                "max_memory_chars",
                "max_memory_entry_chars",
                "max_memory_chars must be at least max_memory_entry_chars",
            ),
            (
                |s| s.waiting_grace = s.max_wait + std::time::Duration::from_millis(1),
                "waiting_grace",
                "max_wait",
                "waiting_grace must not exceed max_wait",
            ),
            (
                |s| {
                    s.max_wait = s.default_wait - std::time::Duration::from_millis(1);
                    s.waiting_grace = std::time::Duration::ZERO;
                },
                "max_wait",
                "default_wait",
                "max_wait must be at least default_wait",
            ),
        ];
        for (make, name, check, text) in cases {
            let mut settings = Settings::DEFAULT;
            make(&mut settings);
            let problems = settings.problems();
            assert_eq!(problems.len(), 1, "{name}: {problems:?}");
            assert_eq!((problems[0].name, problems[0].check), (name, check));
            assert_eq!(problems[0].to_string(), text);
        }
        // Equal is allowed: each bound is "at least", or "not more than".
        let at_the_bounds = Settings {
            max_limit: Settings::DEFAULT.default_limit,
            max_memory_chars: Settings::DEFAULT.max_memory_entry_chars,
            waiting_grace: Settings::DEFAULT.max_wait,
            ..Settings::DEFAULT
        };
        assert_eq!(at_the_bounds.problems(), []);
    }

    #[test]
    fn every_problem_is_listed_floors_first_and_a_setting_may_be_named_twice() {
        let settings = Settings {
            default_limit: 0,
            max_limit: 0,
            max_memory_entry_chars: 5,
            max_memory_chars: 0,
            max_agents: 0,
            ..Settings::DEFAULT
        };
        assert_eq!(
            named(&settings),
            [
                ("default_limit", FLOOR),
                ("max_memory_chars", FLOOR),
                ("max_agents", FLOOR),
                ("max_memory_chars", "max_memory_entry_chars"),
            ]
        );
    }

    #[test]
    fn the_record_lists_each_setting_once_in_its_order() {
        let lines = Settings::DEFAULT.record_lines();
        let names: Vec<&str> = lines.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "name_pack",
                "default_limit",
                "max_limit",
                "max_body_bytes",
                "max_channel_name_chars",
                "max_purpose_chars",
                "max_description_chars",
                "max_note_chars",
                "max_server_announcement_chars",
                "max_memory_entry_chars",
                "max_memory_chars",
                "max_lane_chars",
                "sends_per_minute",
                "registrations_per_minute",
                "max_agents",
                "default_wait_ms",
                "max_wait_ms",
                "waiting_grace_ms",
                "member_preview",
                "purpose_preview_chars",
            ]
        );
        let value = |name: &str| {
            lines
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(value("name_pack"), Some(Settings::DEFAULT.name_pack.name()));
        assert_eq!(value("default_wait_ms"), Some("30000"));
        assert_eq!(value("max_wait_ms"), Some("50000"));
        assert_eq!(value("waiting_grace_ms"), Some("3000"));
        assert_eq!(value("max_lane_description_chars"), None);
    }
}
