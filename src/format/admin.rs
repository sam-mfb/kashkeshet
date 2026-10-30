//! Output for the admin endpoint's own answers: channels with whether they
//! are hidden, and the operator instructions. Its other answers (messages,
//! members, notes, agents) use the agents' formats.

use super::channels::{channel_json, channel_line, owner_line};
use super::fit::{Row, one_line};
use super::{Format, Formatter};
use crate::config::NOTE_PREVIEW_CHARS;
use crate::rules::{AdminChannel, OperatorInstructions, Page};
use serde_json::{Value, json};

impl Formatter {
    /// Every channel on a page, each marked open or closed, and hidden when
    /// it is.
    pub fn admin_channels(
        &self,
        f: Format,
        page: &Page<AdminChannel>,
        purpose_chars: usize,
    ) -> String {
        let rows: Vec<Row> = page
            .items
            .iter()
            .map(|c| {
                let mut note_chars = NOTE_PREVIEW_CHARS;
                loop {
                    let row = Row {
                        text: admin_line(c, purpose_chars, note_chars),
                        json: admin_json(c, Some(purpose_chars), note_chars),
                        resume: format!("cursor={}", crate::rules::channel_cursor(&c.info)),
                    };
                    let bytes = match f {
                        Format::Text => row.text.len() + "\nmore: ".len() + row.resume.len(),
                        Format::Json => json!({"channels": [&row.json], "more": row.resume})
                            .to_string()
                            .len(),
                    };
                    if bytes <= self.max_bytes || note_chars == 0 {
                        break row;
                    }
                    note_chars -= 1;
                }
            })
            .collect();
        let tail = page.next_cursor.as_ref().map(|c| format!("cursor={c}"));
        self.list(f, None, "channels", &rows, tail)
    }

    /// One channel's details. The note is read with `get_note`, the members
    /// with `list_members`.
    pub fn admin_channel(&self, f: Format, c: &AdminChannel) -> String {
        let i = &c.info;
        let out = match f {
            Format::Text => {
                let mut lines = vec![format!(
                    "#{} [{}] {}{}, created by {} at {}",
                    i.name,
                    i.id,
                    if i.open { "open" } else { "closed" },
                    if c.hidden { ", hidden" } else { "" },
                    i.created_by,
                    i.created_at
                )];
                if let Some(p) = &i.purpose {
                    lines.push(format!("purpose: {}", one_line(p)));
                }
                lines.push(format!(
                    "members ({}): {}{}",
                    i.member_count,
                    i.members_preview.join(", "),
                    if (i.members_preview.len() as i64) < i.member_count {
                        ", … (list_members)"
                    } else {
                        ""
                    }
                ));
                lines.push(owner_line(i.owner.as_deref()));
                match &i.note.edited_by {
                    Some(editor) => lines.push(format!(
                        "note: {} characters, last edited by {editor} at {} (get_note)",
                        i.note.text.chars().count(),
                        i.note.edited_at.as_deref().unwrap_or("")
                    )),
                    None => lines.push("note: (never set)".into()),
                }
                lines.join("\n")
            }
            Format::Json => {
                let mut value = admin_json(c, None, NOTE_PREVIEW_CHARS);
                value["created_at"] = json!(i.created_at);
                value.to_string()
            }
        };
        self.clip(out)
    }

    /// The operator instructions agents are given, if any have been saved.
    pub fn operator_instructions(
        &self,
        f: Format,
        instructions: Option<&OperatorInstructions>,
    ) -> String {
        match f {
            Format::Text => match instructions {
                None => "no operator instructions".into(),
                Some(v) => format!(
                    "operator instructions, version {}, saved {}:\n{}",
                    v.version, v.saved_at, v.text
                ),
            },
            Format::Json => json!({ "operator_instructions": instructions }).to_string(),
        }
    }
}

fn admin_line(c: &AdminChannel, purpose_chars: usize, note_chars: usize) -> String {
    let line = channel_line(&c.info, purpose_chars, note_chars);
    if !c.hidden {
        return line;
    }
    // After the open or closed mark: `[id] #name · closed · hidden · ...`.
    let mark = if c.info.open {
        " · open"
    } else {
        " · closed"
    };
    match line.find(mark) {
        Some(at) => {
            let end = at + mark.len();
            format!("{} · hidden{}", &line[..end], &line[end..])
        }
        None => format!("{line} · hidden"),
    }
}

fn admin_json(c: &AdminChannel, purpose_chars: Option<usize>, note_chars: usize) -> Value {
    let mut value = channel_json(&c.info, purpose_chars, note_chars);
    if let Value::Object(object) = &mut value {
        // The admin agent is not a member of anything.
        object.remove("is_member");
        object.insert("hidden".into(), json!(c.hidden));
    }
    value
}
