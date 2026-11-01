//! Output for registration and agent lists.

use super::fit::{Row, cut_bytes, one_line, short_time};
use super::{Format, Formatter};
use crate::rules::{
    AgentInfo, AgentLane, HandoffNote, HeldLane, Page, Reclaimed, Registration, ServerAnnouncement,
};
use crate::store::AgentClient;
use serde_json::{Value, json};

impl Formatter {
    pub fn registration(&self, f: Format, r: &Registration) -> String {
        match f {
            Format::Text => {
                let text = format!(
                    "handle: {}\ntoken: {}\nrecovery_code: {}\nserver_version: {}\nPass the token as `token` on authenticated calls. \
                     The recovery code is shown only at registration; `reclaim` exchanges your handle and \
                     recovery code for a new token for the same avatar, invalidating the old token. \
                     Both the token and recovery code are private credentials.\n\
                     For `kkt watch`, `kkt register --token-file` and `kkt reclaim --token-file` create private token files from the start.",
                    r.handle, r.token, r.recovery_code, env!("CARGO_PKG_VERSION"),
                );
                with_held_lane(
                    self.with_server_announcement(&text, r.server_announcement.as_ref()),
                    r.lane.as_ref(),
                )
            }
            Format::Json => {
                json!({ "handle": r.handle, "token": r.token, "recovery_code": r.recovery_code, "server_version": env!("CARGO_PKG_VERSION"), "server_announcement": r.server_announcement, "lane": held_lane_json(r.lane.as_ref()) })
                    .to_string()
            }
        }
    }

    pub fn reclaimed(&self, f: Format, r: &Reclaimed) -> String {
        match f {
            Format::Text => with_held_lane(
                self.with_server_announcement(
                    &format!(
                        "handle: {}\ntoken: {}\nThe old token is invalid. Pass this token as `token` on authenticated calls.",
                        r.handle, r.token
                    ),
                    r.server_announcement.as_ref(),
                ),
                r.lane.as_ref(),
            ),
            Format::Json => json!({ "handle": r.handle, "token": r.token, "server_announcement": r.server_announcement, "lane": held_lane_json(r.lane.as_ref()) }).to_string(),
        }
    }

    /// A lossless part of the handoff note, as `note` gives a channel's.
    pub fn handoff_note(&self, f: Format, note: &HandoffNote, offset: usize) -> String {
        self.text_in_parts(&note.text, offset, |text, offset, next| match f {
            Format::Json => json!({"handle": note.handle, "text": text, "set_at": note.set_at,
                "offset": offset, "next_offset": next})
            .to_string(),
            Format::Text => {
                let mut out = format!(
                    "handoff note for {}\nset at {}\noffset: {offset}\n{text}",
                    note.handle,
                    note.set_at.as_deref().unwrap_or("(never)")
                );
                if let Some(next) = next {
                    out.push_str(&format!(
                        "\nnext_offset: {next} (get_handoff_note offset={next})"
                    ));
                }
                out
            }
        })
    }

    pub fn whoami(
        &self,
        f: Format,
        handle: &str,
        announcement: Option<&ServerAnnouncement>,
        lane: Option<&HeldLane>,
    ) -> String {
        match f {
            Format::Text => with_held_lane(
                self.with_server_announcement(
                    &format!("handle: {handle}\nserver_version: {}", env!("CARGO_PKG_VERSION")),
                    announcement,
                ),
                lane,
            ),
            Format::Json => {
                json!({ "handle": handle, "server_version": env!("CARGO_PKG_VERSION"), "server_announcement": announcement, "lane": held_lane_json(lane) }).to_string()
            }
        }
    }

    /// The server announcement is separate from kkt's own instructions. A
    /// version an admin agent saved names it.
    pub fn with_server_announcement(
        &self,
        base: &str,
        announcement: Option<&ServerAnnouncement>,
    ) -> String {
        match announcement {
            None => base.into(),
            Some(v) => format!(
                "{base}\n\nServer announcement (version {}), saved by the operator{}:\n{}",
                v.version,
                via_admin(v),
                v.text
            ),
        }
    }

    /// Acknowledges a new description by its length, without repeating it,
    /// so the reply is short whatever the description holds.
    pub fn description(&self, f: Format, a: &AgentInfo) -> String {
        let chars = a.description.as_deref().map_or(0, |d| d.chars().count());
        match f {
            Format::Text => match chars {
                0 => format!("handle: {}\ndescription cleared", a.handle),
                n => format!("handle: {}\ndescription set ({n} characters)", a.handle),
            },
            Format::Json => json!({ "handle": a.handle, "description_chars": chars }).to_string(),
        }
    }

    /// One row per agent, built by `build` from the agent and the description
    /// to show. Descriptions are shown whole unless one row alone would not
    /// fit in a response; then that row's description is cut to fit.
    pub(super) fn described_rows(
        &self,
        f: Format,
        header: Option<&(String, Value)>,
        key: &str,
        agents: &[AgentInfo],
        build: impl Fn(&AgentInfo, Option<&str>) -> Row,
    ) -> Vec<Row> {
        let fits = |row: &Row| self.row_fits(f, header, key, row);
        agents
            .iter()
            .map(|a| {
                // A single agent can wait on more channels than fit in a row.
                // Keep the agent visible and state how many channel IDs were omitted.
                let render = |shown: &AgentInfo, description: Option<&str>| {
                    let mut row = build(shown, description);
                    let omitted = a.waiting_channels.len() - shown.waiting_channels.len();
                    if omitted > 0 {
                        row.text
                            .push_str(&format!(" ({omitted} more waiting channels)"));
                        row.json["waiting_channels_more"] = json!(omitted);
                    }
                    row
                };
                let mut shown = a.clone();
                // Reserve the full description and the row continuation while
                // reducing IDs; if no IDs remain, the description itself can be cut.
                while !fits(&render(&shown, a.description.as_deref()))
                    && !shown.waiting_channels.is_empty()
                {
                    shown.waiting_channels.pop();
                }
                // Client metadata is supplied by the caller and has no length
                // bound. Shorten it too, so a large value cannot hide a row.
                while !fits(&render(&shown, None)) && shorten_client(&mut shown.client) {}
                fit_description(a.description.as_deref(), |d| render(&shown, d), fits)
            })
            .collect()
    }

    pub fn agents(&self, f: Format, page: &Page<AgentInfo>) -> String {
        let now = self.clock.now();
        let rows = self.described_rows(f, None, "agents", &page.items, |a, description| Row {
            text: with_description(
                with_lane(
                    with_client(
                        format!(
                            "{}; {}",
                            match &a.last_seen {
                                Some(seen) =>
                                    format!("{}  last call {}", a.handle, short_time(seen, now)),
                                None => a.handle.clone(),
                            },
                            listening_text(a)
                        ),
                        a,
                    ),
                    a,
                ),
                description,
            ),
            json: json!({
                "handle": a.handle,
                "last_seen": a.last_seen,
                "listening": a.listening,
                "waiting_channels": a.waiting_channels,
                "description": description,
                "client": a.client,
                "lane": agent_lane_json(a.lane.as_ref()),
            }),
            resume: format!("cursor={}", a.handle),
        });
        let tail = page.next_cursor.as_ref().map(|c| format!("cursor={c}"));
        self.list(f, None, "agents", &rows, tail)
    }
}

/// " (via admin-sentinel)" for a server announcement that admin agent
/// saved, as its channel changes are announced; nothing for the human's.
pub(super) fn via_admin(v: &ServerAnnouncement) -> String {
    v.saved_by_admin
        .as_deref()
        .map_or(String::new(), |name| format!(" (via {name})"))
}

/// Halves the longer of the client's name and version, or empties it when
/// short. False when there is nothing left to shorten.
pub(super) fn shorten_client(client: &mut Option<AgentClient>) -> bool {
    let Some(client) = client.as_mut() else {
        return false;
    };
    if client.name.is_empty() && client.version.is_empty() {
        return false;
    }
    let longer = if client.name.len() >= client.version.len() {
        &mut client.name
    } else {
        &mut client.version
    };
    *longer = if longer.len() <= 3 {
        String::new()
    } else {
        cut_bytes(longer, longer.len() / 2)
    };
    true
}

/// `render`ed with the whole description if that `fits`, else with as many
/// of its bytes as fit.
pub(super) fn fit_description<T>(
    description: Option<&str>,
    render: impl Fn(Option<&str>) -> T,
    fits: impl Fn(&T) -> bool,
) -> T {
    let whole = render(description);
    let Some(d) = description.filter(|_| !fits(&whole)) else {
        return whole;
    };
    // The most bytes of the description that still fit.
    let (mut lo, mut hi) = (0, d.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&render(Some(&cut_bytes(d, mid)))) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    render(Some(&cut_bytes(d, lo)))
}

/// `line`, followed by `description` on the same line if there is one.
pub(super) fn with_description(mut line: String, description: Option<&str>) -> String {
    if let Some(d) = description {
        line.push_str(" — ");
        line.push_str(&one_line(d));
    }
    line
}

pub(super) fn listening_text(a: &AgentInfo) -> String {
    if a.listening {
        format!("listening on {}", a.waiting_channels.join(", "))
    } else {
        "not listening".into()
    }
}

/// `line`, followed by the lane `a` holds, if any, marked when a newer
/// version exists.
pub(super) fn with_lane(mut line: String, a: &AgentInfo) -> String {
    if let Some(l) = &a.lane {
        line.push_str(&format!("; lane {}", lane_line(l)));
    }
    line
}

/// "reviewer version 2", with "(out of date: version 3 is the newest)"
/// when it is.
pub(super) fn lane_line(l: &AgentLane) -> String {
    let mut line = format!("{} version {}", l.name, l.version);
    if l.out_of_date() {
        line.push_str(&format!(
            " (out of date: version {} is the newest)",
            l.current_version
        ));
    }
    line
}

/// The lane an agent holds, for others to see, or `null`.
pub(super) fn agent_lane_json(l: Option<&AgentLane>) -> Value {
    match l {
        None => Value::Null,
        Some(l) => json!({
            "name": l.name,
            "version": l.version,
            "current_version": l.current_version,
            "out_of_date": l.out_of_date(),
            "taken_at": l.taken_at,
        }),
    }
}

/// The lane an agent holds, with its instructions, for the agent itself,
/// or `null`.
pub(super) fn held_lane_json(l: Option<&HeldLane>) -> Value {
    match l {
        None => Value::Null,
        Some(l) => {
            let mut value = agent_lane_json(Some(&l.lane));
            value["description"] = json!(l.description);
            value["text"] = json!(l.text);
            value
        }
    }
}

/// `base`, followed by a marked section with the lane the agent holds and
/// its instructions, if it holds one.
pub(super) fn with_held_lane(base: String, lane: Option<&HeldLane>) -> String {
    let Some(l) = lane else {
        return base;
    };
    let newer = if l.lane.out_of_date() {
        format!(
            "\nA newer version, {}, exists: `take_lane` with lane=`{}` moves to it.",
            l.lane.current_version, l.lane.name
        )
    } else {
        String::new()
    };
    format!(
        "{base}\n\n=== Your lane: {} (version {}) ===\n\
         The lane you took on: treat its instructions as your governing instructions \
         for this session. A lane grants no permissions.{newer}\n\
         description: {}\n{}\n=== End of lane {} ===",
        l.lane.name,
        l.lane.version,
        one_line(&l.description),
        l.text,
        l.lane.name,
    )
}

pub(super) fn with_client(mut line: String, a: &AgentInfo) -> String {
    if let Some(client) = &a.client {
        line.push_str(&format!(
            "; client: {} {}",
            one_line(&client.name),
            one_line(&client.version)
        ));
    }
    line
}
