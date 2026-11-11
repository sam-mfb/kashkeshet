//! Output for lanes: a page of the list, and one lane's instructions in
//! parts.
//!
//! A lane's description can be up to 200 characters of four bytes each, so
//! at the smallest response cap it may not fit beside the other details.
//! Then it is cut to fit, and marked so (`description_cut`), so that a list
//! row always fits with its cursor and every part of the instructions has
//! room for `LANE_PART_ROOM_CHARS` of them, or at least one: following
//! `cursor` or `next_offset` always moves on.

use super::agents::{held_lane_json, with_held_lane};
use super::fit::{Row, one_line};
use super::{Format, Formatter};
use kkt_config::LANE_PART_ROOM_CHARS;
use kkt_rules::{Lane, LaneChange, LaneTaking, Page};
use serde_json::{Value, json};

/// The description as one response shows it: whole, or its first
/// characters when the whole does not fit.
struct Description {
    text: String,
    cut: bool,
}

impl Description {
    /// The first `chars` characters of `l`'s description, as one line.
    fn of(l: &Lane, chars: usize) -> Description {
        let whole = one_line(&l.current.description);
        let total = whole.chars().count();
        if chars >= total {
            return Description {
                text: whole,
                cut: false,
            };
        }
        Description {
            text: format!("{}…", whole.chars().take(chars).collect::<String>()),
            cut: true,
        }
    }

    fn line(&self) -> String {
        if self.cut {
            format!("{} (cut to fit)", self.text)
        } else {
            self.text.clone()
        }
    }
}

impl Formatter {
    /// One page of lanes, one line each, without the instructions: as many
    /// as fit, with a cursor for the rest.
    pub fn lanes(&self, f: Format, page: &Page<Lane>) -> String {
        let rows: Vec<Row> = page
            .items
            .iter()
            .map(|l| {
                let row = |chars: usize| {
                    let d = Description::of(l, chars);
                    Row {
                        text: format!(
                            "{} · version {} · {}{} · saved {} by the operator{}",
                            l.name,
                            l.current.version,
                            d.line(),
                            if l.retired.is_some() {
                                " · retired"
                            } else {
                                ""
                            },
                            l.current.saved_at,
                            via(l.current.saved_by_admin.as_deref()),
                        ),
                        json: lane_json(l, Some(&d), None),
                        resume: format!("cursor={}", l.name),
                    }
                };
                let fits = |chars: usize| self.row_fits(f, None, "lanes", &row(chars));
                row(longest_that_fits(description_chars(l), fits))
            })
            .collect();
        let tail = page.next_cursor.as_ref().map(|c| format!("cursor={c}"));
        self.list(f, None, "lanes", &rows, tail)
    }

    /// One page of the lanes an agent may take, one line each: the name,
    /// newest version and description, without the instructions or who
    /// saved them.
    pub fn takeable_lanes(&self, f: Format, page: &Page<Lane>) -> String {
        let rows: Vec<Row> = page
            .items
            .iter()
            .map(|l| {
                let row = |chars: usize| {
                    let d = Description::of(l, chars);
                    Row {
                        text: format!("{} · version {} · {}", l.name, l.current.version, d.line()),
                        json: json!({
                            "lane": l.name,
                            "version": l.current.version,
                            "description": d.text,
                            "description_cut": d.cut,
                        }),
                        resume: format!("cursor={}", l.name),
                    }
                };
                let fits = |chars: usize| self.row_fits(f, None, "lanes", &row(chars));
                row(longest_that_fits(description_chars(l), fits))
            })
            .collect();
        let tail = page.next_cursor.as_ref().map(|c| format!("cursor={c}"));
        self.list(f, None, "lanes", &rows, tail)
    }

    /// What `take_lane` changed, then the lane the agent holds now with its
    /// instructions, as `register` gives them. Like `register`'s, the reply
    /// is whole even when it is longer than the response cap.
    pub fn lane_taking(&self, f: Format, handle: &str, t: &LaneTaking) -> String {
        let now = t.held.as_ref().map_or(String::new(), |h| {
            format!("the {} lane (version {})", h.lane.name, h.lane.version)
        });
        match f {
            Format::Json => {
                let (change, from) = match &t.change {
                    LaneChange::Took => ("took", Value::Null),
                    LaneChange::Switched { from } => ("switched", json!(from)),
                    LaneChange::Updated { from } => ("updated", json!(from)),
                    LaneChange::Dropped { from } => ("dropped", json!(from)),
                    LaneChange::Unchanged => ("unchanged", Value::Null),
                };
                json!({
                    "handle": handle,
                    "change": change,
                    "from": from,
                    "lane": held_lane_json(t.held.as_ref()),
                })
                .to_string()
            }
            Format::Text => {
                let told = "Your channels were told.";
                let first = match (&t.change, &t.held) {
                    (LaneChange::Took, _) => format!("You took {now}. {told}"),
                    (LaneChange::Switched { from }, _) => {
                        format!("You switched from the {from} lane to {now}. {told}")
                    }
                    (LaneChange::Updated { from }, _) => {
                        format!("You moved from version {from} to {now}. {told}")
                    }
                    (LaneChange::Dropped { from }, _) => {
                        format!("You dropped the {from} lane and hold none now. {told}")
                    }
                    (LaneChange::Unchanged, Some(_)) => {
                        format!("You already hold {now}, its newest version. Nothing changed.")
                    }
                    (LaneChange::Unchanged, None) => {
                        "You hold no lane. Nothing changed.".to_string()
                    }
                };
                with_held_lane(format!("handle: {handle}\n{first}"), t.held.as_ref())
            }
        }
    }

    /// One lane with its instructions from character `offset` on, as much
    /// as fits; `next_offset` says where the rest starts. Every part has
    /// the lane's details; the description is cut, and marked, only when
    /// the whole of it would leave no room for the instructions.
    pub fn lane(&self, f: Format, l: &Lane, offset: usize) -> String {
        let text = &l.current.text;
        let total = text.chars().count();
        let offset = offset.min(total);
        // Whether a part with the first `chars` of the description has room
        // for the next `room` characters of the instructions.
        let fits = |room: usize, chars: usize| {
            let room = room.min(total - offset);
            let part: String = text.chars().skip(offset).take(room).collect();
            let next = (offset + room < total).then_some(offset + room);
            render_lane(f, l, &Description::of(l, chars), &part, offset, next).len()
                <= self.max_bytes
        };
        // Room for a fair part if the details allow it, else for the one
        // character that still moves on.
        let room = if fits(LANE_PART_ROOM_CHARS, 0) {
            LANE_PART_ROOM_CHARS
        } else {
            1
        };
        let chars = longest_that_fits(description_chars(l), |chars| fits(room, chars));
        let description = Description::of(l, chars);
        self.text_in_parts(text, offset, |part, offset, next| {
            render_lane(f, l, &description, part, offset, next)
        })
    }
}

fn description_chars(l: &Lane) -> usize {
    one_line(&l.current.description).chars().count()
}

/// The largest count from `0..=whole` for which `fits` holds, trying the
/// whole first; zero when none does. `fits` holds for every count below one
/// for which it holds.
fn longest_that_fits(whole: usize, fits: impl Fn(usize) -> bool) -> usize {
    if fits(whole) {
        return whole;
    }
    let (mut low, mut high) = (0, whole);
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if fits(mid) {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

/// One part of a lane: its details, `description`, and `text`, the
/// instructions from `offset` on, up to `next` if they go on.
fn render_lane(
    f: Format,
    l: &Lane,
    description: &Description,
    text: &str,
    offset: usize,
    next: Option<usize>,
) -> String {
    match f {
        Format::Json => {
            let mut value = lane_json(l, Some(description), Some(text));
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
                format!("description: {}", description.line()),
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
    }
}

/// " (via admin-sentinel)" for what that admin agent did, as its channel
/// changes are announced; nothing for the human's.
fn via(admin: Option<&str>) -> String {
    admin.map_or(String::new(), |name| format!(" (via {name})"))
}

/// The lane's fields, with `description` as shown, and its instructions
/// (or this part of them) when given.
fn lane_json(l: &Lane, description: Option<&Description>, text: Option<&str>) -> Value {
    let whole = Description::of(l, usize::MAX);
    let description = description.unwrap_or(&whole);
    let mut value = json!({
        "lane": l.name,
        "version": l.current.version,
        "description": description.text,
        "description_cut": description.cut,
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

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_clock::testing::FixedClock;
    use kkt_config::MIN_RESPONSE_BYTES;
    use kkt_names::MAX_ADMIN_NAME_CHARS;
    use kkt_rules::{LaneRetired, LaneVersion};
    use std::sync::Arc;

    fn formatter(max_bytes: usize) -> Formatter {
        Formatter::new(
            max_bytes,
            Arc::new(FixedClock::new(kkt_clock::Time::UNIX_EPOCH)),
            "1.2.3-test",
        )
    }

    /// A lane with the largest details allowed: the longest name, a
    /// description of the most four-byte characters, saved and retired by
    /// the longest admin name.
    fn heavy_lane(text: &str, retired: bool) -> Lane {
        let admin = format!("admin-{}", "a".repeat(MAX_ADMIN_NAME_CHARS - 6));
        Lane {
            name: format!("{}-b", "a".repeat(38)),
            created_at: "2026-11-01T10:00:00.000000Z".into(),
            run_id: 1,
            retired: retired.then(|| LaneRetired {
                at: "2026-11-01T12:00:00.000000Z".into(),
                run_id: 1,
                by_admin: Some(admin.clone()),
            }),
            current: LaneVersion {
                version: 1_000_000,
                description: "🦊".repeat(kkt_rules::Settings::DEFAULT.max_lane_description_chars),
                text: text.into(),
                saved_at: "2026-11-01T11:00:00.000000Z".into(),
                run_id: 1,
                saved_by_admin: Some(admin),
                source: None,
            },
        }
    }

    /// Reads the whole text by following `next_offset` from zero, checking
    /// each part fits and moves on.
    fn read_all(fmt: &Formatter, f: Format, lane: &Lane) -> String {
        let mut whole = String::new();
        let mut offset = 0;
        for _ in 0..=lane.current.text.chars().count() {
            let out = fmt.lane(f, lane, offset);
            assert!(
                out.len() <= fmt.max_bytes,
                "{} > {}",
                out.len(),
                fmt.max_bytes
            );
            let (part, next) = match f {
                Format::Json => {
                    let v: Value = serde_json::from_str(&out).expect("JSON");
                    assert_eq!(v["offset"], offset as u64);
                    (
                        v["text"].as_str().expect("text").to_string(),
                        v["next_offset"].as_u64().map(|n| n as usize),
                    )
                }
                Format::Text => {
                    let marker = format!("\noffset: {offset}\n");
                    let at = out.find(&marker).expect("offset line") + marker.len();
                    let rest = &out[at..];
                    match rest.rfind("\nnext_offset: ") {
                        Some(end) => {
                            let n = rest[end + "\nnext_offset: ".len()..]
                                .split(' ')
                                .next()
                                .expect("n")
                                .parse()
                                .expect("number");
                            (rest[..end].to_string(), Some(n))
                        }
                        None => (rest.to_string(), None),
                    }
                }
            };
            whole.push_str(&part);
            match next {
                Some(n) => {
                    assert!(n > offset, "next_offset {n} does not move on from {offset}");
                    let remaining = lane.current.text.chars().count() - offset;
                    assert!(
                        n - offset >= remaining.min(LANE_PART_ROOM_CHARS),
                        "a part of {} characters at {offset}",
                        n - offset
                    );
                    assert_eq!(n, offset + part.chars().count());
                    offset = n;
                }
                None => return whole,
            }
        }
        panic!("reading did not end");
    }

    #[test]
    fn heavy_details_still_let_every_part_move_on_at_the_smallest_cap() {
        let text = "Instructions: read the diff 🦊 twice.\n".repeat(20);
        for retired in [false, true] {
            let lane = heavy_lane(&text, retired);
            for f in [Format::Json, Format::Text] {
                let fmt = formatter(MIN_RESPONSE_BYTES);
                assert_eq!(read_all(&fmt, f, &lane), text, "{f:?} retired={retired}");
            }
        }
    }

    #[test]
    fn the_description_is_cut_and_marked_only_when_it_must_be() {
        let lane = heavy_lane("Instructions", true);
        let small: Value =
            serde_json::from_str(&formatter(MIN_RESPONSE_BYTES).lane(Format::Json, &lane, 0))
                .expect("JSON");
        assert_eq!(small["description_cut"], true);
        assert_eq!(small["text"], "Instructions");
        assert!(small["next_offset"].is_null(), "{small}");
        assert!(small["description"].as_str().expect("d").ends_with('…'));
        let text = formatter(MIN_RESPONSE_BYTES).lane(Format::Text, &lane, 0);
        assert!(text.contains("(cut to fit)"), "{text}");
        assert!(
            text.contains("retired 2026-11-01T12:00:00.000000Z"),
            "{text}"
        );

        let roomy: Value =
            serde_json::from_str(&formatter(8192).lane(Format::Json, &lane, 0)).expect("JSON");
        assert_eq!(roomy["description_cut"], false);
        assert_eq!(roomy["description"], lane.current.description);
        assert_eq!(
            roomy["retired_by_admin"],
            lane.current.saved_by_admin.clone().unwrap()
        );
    }

    #[test]
    fn the_reviewers_case_reads_to_the_end() {
        // A 40-character name, 200 emoji, no admin, `Instructions`: this
        // once gave an empty part pointing back at offset 0.
        let mut lane = heavy_lane("Instructions", false);
        lane.current.saved_by_admin = None;
        for f in [Format::Json, Format::Text] {
            let fmt = formatter(MIN_RESPONSE_BYTES);
            assert_eq!(read_all(&fmt, f, &lane), "Instructions", "{f:?}");
        }
    }

    #[test]
    fn every_heavy_lane_row_fits_with_its_cursor() {
        let lanes: Vec<Lane> = (0..5)
            .map(|i| {
                let mut l = heavy_lane("x", i % 2 == 0);
                l.name = format!("lane-{i}");
                l
            })
            .collect();
        let page = Page {
            items: lanes,
            next_cursor: Some("lane-4".into()),
        };
        for f in [Format::Json, Format::Text] {
            let out = formatter(MIN_RESPONSE_BYTES).lanes(f, &page);
            assert!(out.len() <= MIN_RESPONSE_BYTES, "{out}");
            assert!(out.contains("cursor=lane-"), "{out}");
            assert!(
                !out.contains("larger than the response size limit"),
                "{out}"
            );
        }
    }
}
