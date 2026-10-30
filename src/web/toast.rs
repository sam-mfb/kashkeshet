//! What each live event's toast says. The wording is decided here, so it can
//! be tested; the page's script only draws the parts it is given (`toast`
//! in `base.html`).

use super::source::LiveEvent;
use serde::Serialize;

/// A toast: an icon, and a line of parts, colored for one agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Toast {
    /// The name whose color the toast takes.
    pub hue_of: String,
    pub icon: &'static str,
    pub parts: Vec<Part>,
}

/// One piece of a toast's line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Part {
    /// Plain words.
    Text { text: String },
    /// An agent's name, linked to its page.
    Agent { name: String },
    /// A name with no page to link to: the operator, or an agent that is
    /// gone.
    Name { name: String },
    /// A channel's name, linked to its page.
    Channel { id: String, name: String },
    /// A small label, such as `closed`.
    Badge { class: String, text: String },
}

impl Toast {
    /// The line as plain text, as a reader sees it.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Text { text } => text.clone(),
                Part::Agent { name } | Part::Name { name } => name.clone(),
                Part::Channel { name, .. } => format!("#{name}"),
                Part::Badge { text, .. } => format!("[{text}]"),
            })
            .collect()
    }
}

fn text(t: &str) -> Part {
    Part::Text { text: t.into() }
}

fn agent(name: &str) -> Part {
    Part::Agent { name: name.into() }
}

fn name(name: &str) -> Part {
    Part::Name { name: name.into() }
}

fn channel(id: &str, name: &str) -> Part {
    Part::Channel {
        id: id.into(),
        name: name.into(),
    }
}

/// Who did something: an agent, linked, or the operator, who has no page.
fn actor(by: &str, by_operator: bool) -> Part {
    if by_operator { name(by) } else { agent(by) }
}

/// The toast for `event`, or `None` for events that show none (messages,
/// and the feed falling behind).
pub fn of(event: &LiveEvent) -> Option<Toast> {
    let (hue_of, icon, parts) = match event {
        LiveEvent::Message(_) | LiveEvent::Missed(_) => return None,
        LiveEvent::Agent(a) => (&a.name, "👋", vec![agent(&a.name), text(" registered")]),
        LiveEvent::Channel(c) => {
            let mut parts = vec![
                channel(&c.id, &c.name),
                text(" created by "),
                agent(&c.created_by),
            ];
            if !c.open {
                parts.push(text(" "));
                parts.push(Part::Badge {
                    class: "closed".into(),
                    text: "closed".into(),
                });
            }
            (&c.created_by, "✨", parts)
        }
        LiveEvent::Joined(j) => (
            &j.agent,
            "➜",
            vec![
                agent(&j.agent),
                text(" joined "),
                channel(&j.channel_id, &j.channel_name),
            ],
        ),
        // An agent left by itself, or the owner or the operator removed it.
        LiveEvent::Left(l) => {
            let place = channel(&l.channel_id, &l.channel_name);
            let parts = if l.by == l.agent && !l.by_operator {
                vec![agent(&l.agent), text(" left "), place]
            } else {
                vec![
                    actor(&l.by, l.by_operator),
                    text(" removed "),
                    agent(&l.agent),
                    text(" from "),
                    place,
                ]
            };
            (&l.agent, "←", parts)
        }
        LiveEvent::ChannelHidden(h) => (
            &h.by,
            "🚫",
            vec![
                actor(&h.by, h.by_operator),
                text(" hid "),
                channel(&h.channel_id, &h.channel_name),
            ],
        ),
        // The agent unregistered itself, or the operator hid it. Its page is
        // gone, so its name is not a link.
        LiveEvent::AgentHidden(h) => {
            let parts = if h.by == h.agent && !h.by_operator {
                vec![name(&h.agent), text(" unregistered")]
            } else {
                vec![actor(&h.by, h.by_operator), text(" hid "), name(&h.agent)]
            };
            (&h.agent, "🚪", parts)
        }
    };
    Some(Toast {
        hue_of: hue_of.clone(),
        icon,
        parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::source::{
        AgentHidden, ChannelHidden, ChannelMembers, Joined, Left, NewAgent, NewChannel,
    };

    fn left(agent: &str, by: &str, by_operator: bool) -> LiveEvent {
        LiveEvent::Left(Left {
            agent: agent.into(),
            channel_id: "ch_room0000".into(),
            channel_name: "room".into(),
            member_count: 2,
            by: by.into(),
            by_operator,
        })
    }

    fn channel_hidden(by: &str, by_operator: bool) -> LiveEvent {
        LiveEvent::ChannelHidden(ChannelHidden {
            channel_id: "ch_room0000".into(),
            channel_name: "room".into(),
            by: by.into(),
            by_operator,
        })
    }

    fn agent_hidden(agent: &str, by: &str, by_operator: bool) -> LiveEvent {
        LiveEvent::AgentHidden(AgentHidden {
            agent: agent.into(),
            by: by.into(),
            by_operator,
            channels: vec![ChannelMembers {
                channel_id: "ch_room0000".into(),
                channel_name: "room".into(),
                member_count: 1,
            }],
        })
    }

    fn said(event: &LiveEvent) -> (String, &'static str, String) {
        let t = of(event).expect("a toast");
        (t.hue_of.clone(), t.icon, t.text())
    }

    fn said_is(event: LiveEvent, hue_of: &str, icon: &'static str, line: &str) {
        assert_eq!(said(&event), (hue_of.into(), icon, line.into()));
    }

    #[test]
    fn leaving_says_the_agent_left() {
        said_is(
            left("bold-panther", "bold-panther", false),
            "bold-panther",
            "←",
            "bold-panther left #room",
        );
    }

    #[test]
    fn removal_by_the_owner_names_the_owner() {
        said_is(
            left("bold-panther", "amber-falcon", false),
            "bold-panther",
            "←",
            "amber-falcon removed bold-panther from #room",
        );
        let t = of(&left("bold-panther", "amber-falcon", false)).expect("a toast");
        assert_eq!(
            t.parts,
            [
                agent("amber-falcon"),
                text(" removed "),
                agent("bold-panther"),
                text(" from "),
                channel("ch_room0000", "room"),
            ]
        );
    }

    #[test]
    fn removal_by_the_operator_names_the_operator_without_a_link() {
        said_is(
            left("bold-panther", "operator", true),
            "bold-panther",
            "←",
            "operator removed bold-panther from #room",
        );
        let t = of(&left("bold-panther", "operator", true)).expect("a toast");
        assert_eq!(t.parts[0], name("operator"));
    }

    #[test]
    fn an_agent_named_like_the_operator_is_still_an_agent() {
        // Only `by_operator` makes the actor the operator.
        let t = of(&left("bold-panther", "operator", false)).expect("a toast");
        assert_eq!(t.parts[0], agent("operator"));
    }

    #[test]
    fn hiding_a_channel_names_the_owner_or_the_operator() {
        said_is(
            channel_hidden("amber-falcon", false),
            "amber-falcon",
            "🚫",
            "amber-falcon hid #room",
        );
        assert_eq!(
            of(&channel_hidden("amber-falcon", false))
                .expect("a toast")
                .parts,
            [
                agent("amber-falcon"),
                text(" hid "),
                channel("ch_room0000", "room")
            ]
        );
        said_is(
            channel_hidden("operator", true),
            "operator",
            "🚫",
            "operator hid #room",
        );
        assert_eq!(
            of(&channel_hidden("operator", true))
                .expect("a toast")
                .parts[0],
            name("operator")
        );
    }

    #[test]
    fn unregistering_says_so_without_a_link() {
        said_is(
            agent_hidden("bold-panther", "bold-panther", false),
            "bold-panther",
            "🚪",
            "bold-panther unregistered",
        );
        assert_eq!(
            of(&agent_hidden("bold-panther", "bold-panther", false))
                .expect("a toast")
                .parts,
            [name("bold-panther"), text(" unregistered")]
        );
    }

    #[test]
    fn hiding_an_agent_names_the_operator() {
        said_is(
            agent_hidden("bold-panther", "operator", true),
            "bold-panther",
            "🚪",
            "operator hid bold-panther",
        );
        assert_eq!(
            of(&agent_hidden("bold-panther", "operator", true))
                .expect("a toast")
                .parts,
            [name("operator"), text(" hid "), name("bold-panther")]
        );
    }

    #[test]
    fn new_agents_channels_and_joins_say_so() {
        said_is(
            LiveEvent::Agent(NewAgent {
                name: "bold-panther".into(),
                created_at: "2026-10-25T12:00:00Z".into(),
            }),
            "bold-panther",
            "👋",
            "bold-panther registered",
        );
        let created = |open| {
            LiveEvent::Channel(NewChannel {
                id: "ch_room0000".into(),
                name: "room".into(),
                open,
                created_by: "amber-falcon".into(),
                member_count: 1,
            })
        };
        said_is(
            created(true),
            "amber-falcon",
            "✨",
            "#room created by amber-falcon",
        );
        said_is(
            created(false),
            "amber-falcon",
            "✨",
            "#room created by amber-falcon [closed]",
        );
        said_is(
            LiveEvent::Joined(Joined {
                agent: "bold-panther".into(),
                channel_id: "ch_room0000".into(),
                channel_name: "room".into(),
                member_count: 2,
            }),
            "bold-panther",
            "➜",
            "bold-panther joined #room",
        );
    }

    #[test]
    fn messages_and_falling_behind_show_no_toast() {
        assert_eq!(of(&LiveEvent::Missed(3)), None);
    }

    #[test]
    fn parts_reach_the_page_as_tagged_json() {
        let t = of(&left("bold-panther", "operator", true)).expect("a toast");
        assert_eq!(
            serde_json::to_value(&t).expect("json"),
            serde_json::json!({
                "hue_of": "bold-panther",
                "icon": "←",
                "parts": [
                    {"kind": "name", "name": "operator"},
                    {"kind": "text", "text": " removed "},
                    {"kind": "agent", "name": "bold-panther"},
                    {"kind": "text", "text": " from "},
                    {"kind": "channel", "id": "ch_room0000", "name": "room"},
                ]
            })
        );
    }
}
