//! Output for lanes: the list, and one lane's instructions in parts.

use super::fit::{Row, one_line};
use super::{Format, Formatter};
use crate::rules::Lane;
use serde_json::{Value, json};

impl Formatter {
    /// Every lane, one line each, without the instructions: as many as fit.
    pub fn lanes(&self, f: Format, lanes: &[Lane]) -> String {
        let rows: Vec<Row> = lanes
            .iter()
            .map(|l| Row {
                text: format!(
                    "{} · version {} · {}{} · saved {} by the operator{}",
                    l.name,
                    l.current.version,
                    one_line(&l.current.description),
                    if l.retired.is_some() {
                        " · retired"
                    } else {
                        ""
                    },
                    l.current.saved_at,
                    via(l.current.saved_by_admin.as_deref()),
                ),
                json: lane_json(l, None),
                resume: "the rest did not fit; `get_lane` shows one lane by name".into(),
            })
            .collect();
        let header = (
            format!("lanes: {}", lanes.len()),
            json!({ "count": lanes.len() }),
        );
        self.list(f, Some(header), "lanes", &rows, None)
    }

    /// One lane with its instructions from character `offset` on, as much
    /// as fits; `next_offset` says where the rest starts.
    pub fn lane(&self, f: Format, l: &Lane, offset: usize) -> String {
        self.text_in_parts(&l.current.text, offset, |text, offset, next| match f {
            Format::Json => {
                let mut value = lane_json(l, Some(text));
                value["offset"] = json!(offset);
                value["next_offset"] = json!(next);
                value.to_string()
            }
            Format::Text => {
                let mut lines = vec![
                    format!(
                        "lane {}, version {}, saved {} by the operator{}",
                        l.name,
                        l.current.version,
                        l.current.saved_at,
                        via(l.current.saved_by_admin.as_deref())
                    ),
                    format!("description: {}", one_line(&l.current.description)),
                ];
                if let Some(r) = &l.retired {
                    lines.push(format!(
                        "retired {} by the operator{}",
                        r.at,
                        via(r.by_admin.as_deref())
                    ));
                }
                lines.push(format!("offset: {offset}"));
                lines.push(text.to_string());
                if let Some(next) = next {
                    lines.push(format!(
                        "next_offset: {next} (get_lane {} offset={next})",
                        l.name
                    ));
                }
                lines.join("\n")
            }
        })
    }
}

/// " (via admin-sentinel)" for what that admin agent did, as its channel
/// changes are announced; nothing for the human's.
fn via(admin: Option<&str>) -> String {
    admin.map_or(String::new(), |name| format!(" (via {name})"))
}

/// The lane's fields, and its instructions (or this part of them) when given.
fn lane_json(l: &Lane, text: Option<&str>) -> Value {
    let mut value = json!({
        "lane": l.name,
        "version": l.current.version,
        "description": l.current.description,
        "saved_at": l.current.saved_at,
        "saved_by_admin": l.current.saved_by_admin,
        "created_at": l.created_at,
        "retired": l.retired.is_some(),
        "retired_at": l.retired.as_ref().map(|r| &r.at),
        "retired_by_admin": l.retired.as_ref().map(|r| &r.by_admin),
    });
    if let Some(text) = text {
        value["text"] = json!(text);
    }
    value
}
