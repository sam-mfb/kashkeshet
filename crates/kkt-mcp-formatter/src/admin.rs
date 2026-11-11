//! Output for the admin endpoint's own answers: channels with whether they
//! are hidden, one agent's details, stale agents and hiding them, and the
//! server announcement. Its other
//! answers (messages, members, notes, agent lists, memory) use the
//! agents' formats.

use super::agents::{client_json, fit_description, shorten_client};
use super::channels::{channel_json, channel_line, owner_line};
use super::fit::{Row, cut_body, one_line, short_time};
use super::{Format, Formatter};
use crate::NOTE_PREVIEW_CHARS;
use kkt_api_agent::{AgentView, ChannelRef, Page, RecordedClient, ServerAnnouncement};
use kkt_rules::{AdminChannel, KeptReason, StaleAgent, StaleHidden};
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
                        resume: format!("cursor={}", kkt_api_agent::channel_cursor(&c.info)),
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
            Format::Json => return self.admin_channel_json(c),
        };
        self.clip(out)
    }

    /// One channel's details as JSON within the cap. Fields are fitted
    /// before serializing, so the answer is always valid JSON: first the
    /// purpose is shortened (`purpose_cut` says so), then the member preview
    /// (`member_count` still gives the total). What is left (IDs, the name,
    /// times) is never cut; with the default name limit it fits in the
    /// smallest cap.
    fn admin_channel_json(&self, c: &AdminChannel) -> String {
        let render = |purpose_chars: Option<usize>, members: usize| {
            let i = &c.info;
            let mut value = admin_json(c, None, NOTE_PREVIEW_CHARS);
            let purpose = i.purpose.as_deref().map(|p| cut_body(p, purpose_chars));
            value["purpose"] = json!(purpose.as_ref().map(|(p, _)| p));
            value["purpose_cut"] = json!(purpose.is_some_and(|(_, cut)| cut));
            value["members_preview"] = json!(&i.members_preview[..members]);
            value["created_at"] = json!(i.created_at);
            value["note_edited_by"] = json!(i.note.edited_by);
            value["note_edited_at"] = json!(i.note.edited_at);
            value.to_string()
        };
        let all_members = c.info.members_preview.len();
        let whole = render(None, all_members);
        if whole.len() <= self.max_bytes {
            return whole;
        }
        let purpose_chars = c.info.purpose.as_deref().map_or(0, |p| p.chars().count());
        // The most purpose characters that fit, found by halving.
        let fits = |chars: usize| render(Some(chars), all_members).len() <= self.max_bytes;
        let (mut lo, mut hi) = (0, purpose_chars);
        if fits(0) {
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                if fits(mid) {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            return render(Some(lo), all_members);
        }
        let mut members = all_members;
        loop {
            let out = render(Some(0), members);
            if out.len() <= self.max_bytes || members == 0 {
                return out;
            }
            members -= 1;
        }
    }

    /// One agent's details, as the web page's agent view shows them: a
    /// header with what `agents` shows of it, how many messages it has sent
    /// and the size of its memory (read with `list_memories` and `recall`), then
    /// its channels from `offset` on, as many as fit. The client and then
    /// the description are shortened if the header alone would not fit.
    pub fn admin_agent(&self, f: Format, v: &AgentView, offset: usize) -> String {
        let a = &v.agent;
        let now = self.clock.now();
        let memory_chars: usize = v.memory.iter().map(|e| e.text.chars().count()).sum();
        let header = |client: Option<&RecordedClient>, description: Option<&str>| {
            let mut lines = vec![
                format!("agent {}, registered {}", a.handle, a.created_at),
                format!(
                    "last call: {}; last sent: {}; {}",
                    a.last_seen
                        .as_deref()
                        .map_or("never".into(), |s| short_time(s, now)),
                    a.last_sent
                        .as_deref()
                        .map_or("never".into(), |s| short_time(s, now)),
                    if a.listening {
                        "listening"
                    } else {
                        "not listening"
                    }
                ),
            ];
            if let Some(c) = client {
                lines.push(format!(
                    "client: {} {}",
                    one_line(&c.name),
                    one_line(&c.version)
                ));
            }
            lines.push(match &a.lane {
                None => "lane: (none)".into(),
                Some(l) => format!(
                    "lane: {}, taken {}",
                    super::agents::lane_line(l),
                    l.taken_at
                ),
            });
            if let Some(d) = description {
                lines.push(format!("description: {}", one_line(d)));
            }
            lines.push(format!("messages sent: {}", v.message_count));
            lines.push(match v.memory.len() {
                0 => "memory: (empty)".into(),
                n => format!(
                    "memory: {n} entries, {memory_chars} characters (list_memories, recall)"
                ),
            });
            lines.push(format!("channels ({}):", v.channels.len()));
            let json = json!({
                "handle": a.handle,
                "created_at": a.created_at,
                "last_seen": a.last_seen,
                "last_sent": a.last_sent,
                "listening": a.listening,
                "client": client_json(client),
                "description": description,
                "lane": super::agents::agent_lane_json(a.lane.as_ref()),
                "message_count": v.message_count,
                "memory_entries": v.memory.len(),
                "memory_chars": memory_chars,
                "channel_count": v.channels.len(),
                "offset": offset,
            });
            (lines.join("\n"), json)
        };
        let rows: Vec<Row> = v
            .channels
            .iter()
            .enumerate()
            .skip(offset)
            .map(|(at, c)| {
                let waiting = a.waiting_channels.contains(&c.id);
                Row {
                    text: format!(
                        "[{}] #{} · {}{}",
                        c.id,
                        c.name,
                        if c.open { "open" } else { "closed" },
                        if waiting { " · waiting" } else { "" }
                    ),
                    json: json!({ "id": c.id, "name": c.name, "open": c.open, "waiting": waiting }),
                    resume: format!("offset={}", at + 1),
                }
            })
            .collect();
        // A header fits if it does with the first channel left, so every
        // response shows at least one channel.
        let fits = |h: &(String, Value)| match rows.first() {
            Some(row) => self.row_fits(f, Some(h), "channels", row),
            None => self.list(f, Some(h.clone()), "channels", &[], None).len() <= self.max_bytes,
        };
        let mut client = a.client.clone();
        while !fits(&header(client.as_ref(), None)) && shorten_client(&mut client) {}
        let header = fit_description(
            a.description.as_deref(),
            |d| header(client.as_ref(), d),
            fits,
        );
        self.list(f, Some(header), "channels", &rows, None)
    }

    /// The agents quiet for longer than `idle_hours`, the longest quiet
    /// first, as many as fit. Owners are listed with what they own: they
    /// cannot be hidden until another member owns it or it has no owner.
    pub fn stale_agents(&self, f: Format, idle_hours: u32, agents: &[StaleAgent]) -> String {
        let now = self.clock.now();
        let ids = |refs: &[ChannelRef]| refs.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        let rows: Vec<Row> = agents
            .iter()
            .map(|s| {
                let a = &s.info;
                let mut text = format!(
                    "{}  last call {}; in {} channels",
                    a.handle,
                    a.last_seen
                        .as_deref()
                        .map_or("never".into(), |seen| short_time(seen, now)),
                    s.channels.len()
                );
                if !s.owns.is_empty() {
                    text.push_str(&format!("; owns {}", ids(&s.owns).join(", ")));
                }
                if s.memory_entries > 0 {
                    text.push_str(&format!("; {} memory entries", s.memory_entries));
                }
                Row {
                    text,
                    json: json!({
                        "handle": a.handle,
                        "last_seen": a.last_seen,
                        "channels": ids(&s.channels),
                        "owns": ids(&s.owns),
                        "memory_entries": s.memory_entries,
                    }),
                    resume: "the longest quiet are shown first; hide some, then list again".into(),
                }
            })
            .collect();
        let header = (
            format!(
                "agents quiet for more than {idle_hours} hours: {}",
                agents.len()
            ),
            json!({ "idle_hours": idle_hours, "count": agents.len() }),
        );
        self.list(f, Some(header), "agents", &rows, None)
    }

    /// Which of the chosen stale agents were hidden, and which were kept
    /// and why.
    pub fn stale_hidden(&self, f: Format, outcome: &StaleHidden) -> String {
        let why = |reason: KeptReason| match reason {
            KeptReason::Gone => "gone",
            KeptReason::Active => "active",
            KeptReason::Owner => "owner",
        };
        let out = match f {
            Format::Text => {
                let mut lines = vec![format!("hidden: {}", outcome.hidden.join(", "))];
                if !outcome.kept.is_empty() {
                    let kept: Vec<String> = outcome
                        .kept
                        .iter()
                        .map(|k| format!("{} ({})", k.handle, why(k.reason)))
                        .collect();
                    lines.push(format!("kept: {}", kept.join(", ")));
                }
                lines.join("\n")
            }
            Format::Json => json!({
                "hidden": outcome.hidden,
                "kept": outcome
                    .kept
                    .iter()
                    .map(|k| json!({ "handle": k.handle,"reason": why(k.reason) }))
                    .collect::<Vec<_>>(),
            })
            .to_string(),
        };
        self.clip(out)
    }

    /// The server announcement agents are given, if one has been saved, and
    /// who saved it.
    pub fn server_announcement(
        &self,
        f: Format,
        announcement: Option<&ServerAnnouncement>,
    ) -> String {
        match f {
            Format::Text => match announcement {
                None => "no server announcement".into(),
                Some(v) => format!(
                    "server announcement, version {}, saved {} by the operator{}:\n{}",
                    v.version,
                    v.saved_at,
                    super::agents::via_admin(v),
                    v.text
                ),
            },
            Format::Json => json!({ "server_announcement": announcement }).to_string(),
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
