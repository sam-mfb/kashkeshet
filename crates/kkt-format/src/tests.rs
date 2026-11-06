//! Output tests: message formatting, the size cap, and the shared helpers.

use super::fit::{cut_body, cut_bytes, short_time};
use super::*;
use kkt_clock::Time;
use kkt_clock::testing::FixedClock;
use kkt_config::Limits;
use kkt_rules::{
    AgentInfo, Members, Message, MessagePage, Page, Position, RecordedClient, WaitResult,
};
use serde_json::Value;

/// The server version the test formatter is given, unlike any real one.
const SERVER_VERSION: &str = "1.2.3-test";

fn formatter(max: usize) -> Formatter {
    let now = Time::from_parts(2026, 10, 23, 15, 0, 0).unwrap();
    Formatter::new(max, Arc::new(FixedClock::new(now)), SERVER_VERSION)
}

fn msg(id: i64, body: &str) -> Message {
    Message {
        id,
        channel_id: "ch_7k2m9q4x".into(),
        channel_name: "general".into(),
        agent: "amber-falcon".into(),
        body: body.into(),
        created_at: "2026-10-23T14:02:00Z".into(),
        run_id: None,
    }
}

#[test]
fn text_messages_look_like_the_plan() {
    let page = MessagePage {
        messages: vec![
            msg(1201, "can someone take the parser?"),
            msg(1207, &"x".repeat(600)),
        ],
        more: Some(Position::Before(1201)),
    };
    let out = formatter(8192).read(Format::Text, &page, Position::Latest, Some(500));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "#general [ch_7k2m9q4x]");
    assert_eq!(
        lines[1],
        "[1201 14:02 amber-falcon] can someone take the parser?"
    );
    assert!(lines[2].ends_with("…  (cut; get_message 1207)"));
    assert_eq!(lines[3], "more: before_id=1201");
}

#[test]
fn size_cap_keeps_newest_and_points_back() {
    let msgs: Vec<Message> = (1..=50).map(|i| msg(i, &"y".repeat(300))).collect();
    let page = MessagePage {
        messages: msgs,
        more: None,
    };
    let out = formatter(2000).read(Format::Text, &page, Position::Latest, None);
    assert!(out.len() <= 2000);
    let first_id: i64 = out.lines().nth(1).unwrap()[1..]
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(out.contains(&format!("more: before_id={first_id}")));
    assert!(out.contains("[50 "));
}

#[test]
fn size_cap_keeps_oldest_when_reading_forward() {
    let msgs: Vec<Message> = (1..=50).map(|i| msg(i, &"y".repeat(300))).collect();
    let page = MessagePage {
        messages: msgs,
        more: None,
    };
    let out = formatter(2000).read(Format::Json, &page, Position::After(0), None);
    assert!(out.len() <= 2000);
    let v: Value = serde_json::from_str(&out).unwrap();
    let shown = v["messages"].as_array().unwrap();
    let last = shown.last().unwrap()["id"].as_i64().unwrap();
    assert_eq!(shown[0]["id"], 1);
    assert_eq!(v["more"], format!("after_id={last}"));
}

#[test]
fn a_message_too_big_for_the_cap_is_shortened_to_fit() {
    let page = MessagePage {
        messages: vec![msg(9, &"z".repeat(5000))],
        more: None,
    };
    for requested in [None, Some(4000)] {
        let out = formatter(1000).read(Format::Text, &page, Position::Latest, requested);
        assert!(out.len() <= 1000);
        assert!(out.contains("zzzzzzzzzz"));
        assert!(out.contains("(cut; get_message 9)"));
        let out = formatter(1000).read(Format::Json, &page, Position::Latest, requested);
        assert!(out.len() <= 1000);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["messages"][0]["id"], 9);
        assert_eq!(v["messages"][0]["cut"], true);
    }
}

#[test]
fn a_waited_message_too_big_for_the_cap_is_shortened_to_fit() {
    let waited = WaitResult {
        messages: vec![msg(9, &"z".repeat(5000)), msg(10, "next")],
        more_waiting: 0,
    };
    for f in [Format::Text, Format::Json] {
        let out = formatter(1000).waited(f, &waited, None);
        assert!(out.len() <= 1000);
        assert!(out.contains("zzzzzzzzzz"));
        assert!(out.contains("after_id=9"));
    }
}

#[test]
fn an_explicit_body_cut_wins_when_it_is_shorter_than_the_cap_allows() {
    let page = MessagePage {
        messages: vec![msg(9, &"z".repeat(5000))],
        more: None,
    };
    let out = formatter(1000).read(Format::Text, &page, Position::Latest, Some(10));
    assert!(out.contains("zzzzzzzzzz…  (cut; get_message 9)"));
    assert!(!out.contains("zzzzzzzzzzz"));
}

#[test]
fn lists_cut_at_cap_with_cursor() {
    let page = Page {
        items: (0..100)
            .map(|i| AgentInfo {
                handle: format!("agent-{i:03}"),
                created_at: "2026-10-23T14:00:00Z".into(),
                last_seen: None,
                last_sent: None,
                listening: false,
                waiting_channels: Vec::new(),
                run_id: None,
                description: None,
                client: None,
                lane: None,
            })
            .collect(),
        next_cursor: None,
    };
    let out = formatter(200).agents(Format::Text, &page);
    assert!(out.len() <= 200);
    let last_shown = out.lines().rev().nth(1).unwrap();
    assert_eq!(
        out.lines().last().unwrap(),
        format!("more: cursor={}", last_shown.split(';').next().unwrap())
    );
}

fn described(name: &str, description: Option<&str>) -> AgentInfo {
    AgentInfo {
        handle: name.into(),
        created_at: "2026-10-23T14:00:00Z".into(),
        last_seen: Some("2026-10-23T14:05:00Z".into()),
        last_sent: Some("2026-10-23T14:03:00Z".into()),
        listening: false,
        waiting_channels: Vec::new(),
        run_id: None,
        description: description.map(str::to_string),
        client: None,
        lane: None,
    }
}

#[test]
fn stale_agents_show_what_they_own_and_hiding_says_who_was_kept_and_why() {
    use kkt_rules::{ChannelRef, KeptReason, StaleAgent, StaleHidden};
    let room = ChannelRef {
        id: "ch_room".into(),
        name: "room".into(),
    };
    let agents = [
        StaleAgent {
            info: described("amber-falcon", None),
            channels: vec![room.clone()],
            memory_entries: 2,
            owns: vec![room],
        },
        StaleAgent {
            info: described("brave-otter", None),
            channels: vec![],
            memory_entries: 0,
            owns: vec![],
        },
    ];
    let fmt = formatter(4096);
    let text = fmt.stale_agents(Format::Text, 2, &agents);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "agents quiet for more than 2 hours: 2");
    assert!(
        lines[1].starts_with("amber-falcon  last call ")
            && lines[1].ends_with("; in 1 channels; owns ch_room; 2 memory entries"),
        "{text}"
    );
    assert!(lines[2].ends_with("; in 0 channels"), "{text}");
    let json: serde_json::Value =
        serde_json::from_str(&fmt.stale_agents(Format::Json, 2, &agents)).unwrap();
    assert_eq!(json["count"], 2);
    assert_eq!(json["agents"][0]["owns"][0], "ch_room");
    assert_eq!(json["agents"][0]["memory_entries"], 2);
    assert_eq!(json["agents"][1]["memory_entries"], 0);

    let outcome = StaleHidden {
        hidden: vec!["brave-otter".into()],
        kept: vec![kkt_rules::Kept {
            handle: "amber-falcon".into(),
            reason: KeptReason::Owner,
        }],
    };
    assert_eq!(
        fmt.stale_hidden(Format::Text, &outcome),
        "hidden: brave-otter\nkept: amber-falcon (owner)"
    );
    let json: serde_json::Value =
        serde_json::from_str(&fmt.stale_hidden(Format::Json, &outcome)).unwrap();
    assert_eq!(json["kept"][0]["reason"], "owner");
}

/// The exact JSON agents see for an agent's client, with every field set
/// and with no client recorded, in `list_agents`, `list_members` and the
/// admin agent view.
#[test]
fn client_json_is_pinned() {
    let mut with_client = described("amber-falcon", Some("worker"));
    with_client.client = Some(RecordedClient {
        name: "Codex".into(),
        version: "1.2.3".into(),
        recorded_at: "2026-10-23T14:05:00Z".into(),
        run_id: 4,
    });
    let without_client = described("brave-otter", None);
    let page = Page {
        items: vec![with_client.clone(), without_client.clone()],
        next_cursor: None,
    };
    let members = Members {
        channel_id: "ch_test".into(),
        owner: None,
        total: 2,
        page: page.clone(),
    };
    let fmt = formatter(Limits::DEFAULT.max_response_bytes);
    let rows = "[{\"client\":{\"name\":\"Codex\",\"recorded_at\":\"2026-10-23T14:05:00Z\",\"run_id\":4,\"version\":\"1.2.3\"},\"description\":\"worker\",\"handle\":\"amber-falcon\",\"lane\":null,\"last_seen\":\"2026-10-23T14:05:00Z\",\"last_sent\":\"2026-10-23T14:03:00Z\",\"listening\":false,\"waiting_channels\":[]},{\"client\":null,\"description\":null,\"handle\":\"brave-otter\",\"lane\":null,\"last_seen\":\"2026-10-23T14:05:00Z\",\"last_sent\":\"2026-10-23T14:03:00Z\",\"listening\":false,\"waiting_channels\":[]}]";
    assert_eq!(
        fmt.agents(Format::Json, &page),
        format!("{{\"agents\":{rows},\"more\":null}}")
    );
    assert_eq!(
        fmt.members(Format::Json, &members),
        format!(
            "{{\"channel_id\":\"ch_test\",\"members\":{rows},\"more\":null,\"owner\":null,\"total\":2}}"
        )
    );
    for (agent, client, description) in [
        (
            with_client,
            "{\"name\":\"Codex\",\"recorded_at\":\"2026-10-23T14:05:00Z\",\"run_id\":4,\"version\":\"1.2.3\"}",
            "\"worker\"",
        ),
        (without_client, "null", "null"),
    ] {
        let handle = agent.handle.clone();
        let view = kkt_rules::AgentView {
            agent,
            channels: Vec::new(),
            message_count: 0,
            memory: Vec::new(),
        };
        assert_eq!(
            fmt.admin_agent(Format::Json, &view, 0),
            format!(
                "{{\"channel_count\":0,\"channels\":[],\"client\":{client},\"created_at\":\"2026-10-23T14:00:00Z\",\"description\":{description},\"handle\":\"{handle}\",\"lane\":null,\"last_seen\":\"2026-10-23T14:05:00Z\",\"last_sent\":\"2026-10-23T14:03:00Z\",\"listening\":false,\"memory_chars\":0,\"memory_entries\":0,\"message_count\":0,\"more\":null,\"offset\":0}}"
            )
        );
    }
}

#[test]
fn client_metadata_is_added_to_lists_and_long_values_fit() {
    for name in ["Codex".to_string(), "😀\"\n".repeat(10000)] {
        let mut agent = described("amber-falcon", Some("worker"));
        agent.client = Some(RecordedClient {
            name: name.clone(),
            version: name.clone(),
            recorded_at: "2026-10-23T14:05:00Z".into(),
            run_id: 1,
        });
        let page = Page {
            items: vec![agent],
            next_cursor: None,
        };
        let members = Members {
            channel_id: "ch_test".into(),
            owner: None,
            total: 1,
            page: page.clone(),
        };
        let fmt = formatter(kkt_config::MIN_RESPONSE_BYTES);
        for (out, key) in [
            (fmt.agents(Format::Json, &page), "agents"),
            (fmt.members(Format::Json, &members), "members"),
        ] {
            assert!(out.len() <= kkt_config::MIN_RESPONSE_BYTES);
            let value: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(value[key][0]["handle"], "amber-falcon");
            assert_eq!(value[key][0]["client"]["run_id"], 1);
            if name == "Codex" {
                assert_eq!(value[key][0]["client"]["name"], "Codex");
            }
        }
        for out in [
            fmt.agents(Format::Text, &page),
            fmt.members(Format::Text, &members),
        ] {
            assert!(out.len() <= kkt_config::MIN_RESPONSE_BYTES);
            assert!(out.contains("amber-falcon"));
            assert!(out.contains("client:"));
        }
    }
}

#[test]
fn agent_and_member_lists_show_descriptions() {
    let items = vec![
        described("amber-falcon", Some("Codex, reviews only")),
        AgentInfo {
            last_sent: None,
            ..described("brave-beaver", None)
        },
    ];
    let page = Page {
        items: items.clone(),
        next_cursor: None,
    };
    assert_eq!(
        formatter(8000).agents(Format::Text, &page),
        "amber-falcon  last call 14:05, last sent 14:03; not listening — Codex, reviews only\nbrave-beaver  last call 14:05; not listening"
    );
    let json: Value = serde_json::from_str(&formatter(8000).agents(Format::Json, &page)).unwrap();
    assert_eq!(json["agents"][0]["description"], "Codex, reviews only");
    assert_eq!(json["agents"][1]["description"], Value::Null);
    assert_eq!(json["agents"][0]["last_sent"], "2026-10-23T14:03:00Z");
    assert_eq!(json["agents"][1]["last_sent"], Value::Null);

    let mut members = Members {
        channel_id: "ch_7k2m9q4x".into(),
        total: 2,
        owner: Some("amber-falcon".into()),
        page: Page {
            items,
            next_cursor: None,
        },
    };
    assert_eq!(
        formatter(8000).members(Format::Text, &members),
        "ch_7k2m9q4x: 2 members\nowner: amber-falcon\namber-falcon; not listening; last call 2026-10-23T14:05:00Z; last sent 2026-10-23T14:03:00Z — Codex, reviews only\nbrave-beaver; not listening; last call 2026-10-23T14:05:00Z; last sent never"
    );
    let json: Value =
        serde_json::from_str(&formatter(8000).members(Format::Json, &members)).unwrap();
    assert_eq!(json["owner"], "amber-falcon");
    members.owner = None;
    assert!(
        formatter(8000)
            .members(Format::Text, &members)
            .starts_with("ch_7k2m9q4x: 2 members\nowner: (none)\n")
    );
    let json: Value =
        serde_json::from_str(&formatter(8000).members(Format::Json, &members)).unwrap();
    assert_eq!(json["owner"], Value::Null);
    assert_eq!(json["members"][0]["handle"], "amber-falcon");
    assert_eq!(json["members"][0]["description"], "Codex, reviews only");
    assert_eq!(json["members"][1]["description"], Value::Null);
}

#[test]
fn the_longest_descriptions_fit_the_smallest_response_cap() {
    let cap = kkt_config::MIN_RESPONSE_BYTES;
    let max_chars = Limits::DEFAULT.max_description_chars;
    // Four-byte characters, characters JSON must escape, and control
    // characters JSON writes as six bytes each.
    for long in [
        "😀".repeat(max_chars),
        "\"\\".repeat(max_chars / 2),
        "\u{1}\n".repeat(max_chars / 2),
    ] {
        let agent = described("amber-falcon", Some(&long));
        let fmt = formatter(cap);
        for f in [Format::Text, Format::Json] {
            let reply = fmt.description(f, &agent);
            assert!(reply.len() <= cap, "{} bytes", reply.len());
            let page = Page {
                items: vec![agent.clone()],
                next_cursor: None,
            };
            let agents = fmt.agents(f, &page);
            assert!(agents.len() <= cap, "{} bytes", agents.len());
            assert!(!agents.contains("larger than"), "{agents}");
            let members = fmt.members(
                f,
                &Members {
                    channel_id: "ch_7k2m9q4x".into(),
                    total: 1,
                    owner: Some("amber-falcon".into()),
                    page,
                },
            );
            assert!(members.len() <= cap, "{} bytes", members.len());
            assert!(!members.contains("larger than"), "{members}");
            if f == Format::Json {
                // Cut only when the whole one could not fit.
                let too_big = serde_json::to_string(&long).unwrap().len() > cap;
                for out in [&agents, &members] {
                    let json: Value = serde_json::from_str(out).expect("valid JSON");
                    assert_eq!(json.to_string().contains('…'), too_big, "{out}");
                }
            }
        }
    }
}

#[test]
fn descriptions_are_whole_when_their_row_fits() {
    // 300 ASCII characters fit whole even at the smallest cap.
    let long = "x".repeat(Limits::DEFAULT.max_description_chars);
    let agent = described("amber-falcon", Some(&long));
    let page = Page {
        items: vec![agent.clone()],
        next_cursor: None,
    };
    let out = formatter(kkt_config::MIN_RESPONSE_BYTES).agents(Format::Json, &page);
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["agents"][0]["description"], long.as_str());
    // At the default cap, so do the longest four-byte descriptions.
    let long = "😀".repeat(Limits::DEFAULT.max_description_chars);
    let page = Page {
        items: vec![described("amber-falcon", Some(&long))],
        next_cursor: None,
    };
    let out = formatter(Limits::DEFAULT.max_response_bytes).agents(Format::Json, &page);
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["agents"][0]["description"], long.as_str());
}

#[test]
fn the_set_description_reply_gives_the_length_not_the_text() {
    let agent = described("amber-falcon", Some("héllo"));
    let fmt = formatter(8000);
    assert_eq!(
        fmt.description(Format::Text, &agent),
        "handle: amber-falcon\ndescription set (5 characters)"
    );
    let json: Value = serde_json::from_str(&fmt.description(Format::Json, &agent)).unwrap();
    assert_eq!(json["description_chars"], 5);
    let cleared = described("amber-falcon", None);
    assert_eq!(
        fmt.description(Format::Text, &cleared),
        "handle: amber-falcon\ndescription cleared"
    );
}

#[test]
fn cut_bytes_keeps_characters_whole() {
    assert_eq!(cut_bytes("héllo", 5), "h…");
    assert_eq!(cut_bytes("héllo", 6), "héllo");
    assert_eq!(cut_bytes("😀😀", 7), "😀…");
}

#[test]
fn short_times() {
    let now = Time::from_parts(2026, 10, 23, 15, 0, 0).unwrap();
    assert_eq!(short_time("2026-10-23T09:05:00Z", now), "09:05");
    assert_eq!(short_time("2026-10-22T09:05:00Z", now), "2026-10-22 09:05");
}

#[test]
fn cut_counts_characters() {
    assert_eq!(cut_body("héllo", Some(2)), ("hé…".to_string(), true));
    assert_eq!(cut_body("hi", Some(2)), ("hi".to_string(), false));
}

#[test]
fn bodies_are_whole_when_the_response_fits() {
    let body = "x".repeat(3000);
    let page = MessagePage {
        messages: vec![msg(5, &body)],
        more: None,
    };
    let out = formatter(8192).read(Format::Text, &page, Position::Latest, None);
    assert!(out.contains(&body));
    assert!(!out.contains("(cut;"));
}

#[test]
fn a_requested_body_cut_still_applies() {
    let page = MessagePage {
        messages: vec![msg(5, &"x".repeat(3000))],
        more: None,
    };
    let out = formatter(8192).read(Format::Text, &page, Position::Latest, Some(10));
    assert!(out.contains("xxxxxxxxxx…  (cut; get_message 5)"));
}

#[test]
fn cut_body_with_no_limit_is_whole() {
    assert_eq!(cut_body("hello", None), ("hello".to_string(), false));
}

#[test]
fn pinned_notes_are_lossless_and_paginated_after_json_escaping() {
    let fmt = formatter(1024);
    for text in [
        "hello\nworld".to_string(),
        "😀\n\t\"\\\0".repeat(1500),
        String::new(),
    ] {
        let note = kkt_rules::ChannelNote {
            channel_id: "ch_notes".into(),
            text: text.clone(),
            edited_by: Some("amber-falcon".into()),
            edited_at: Some("2026-10-23T14:00:00Z".into()),
        };
        let mut offset = 0;
        let mut recovered = String::new();
        loop {
            let out = fmt.note(Format::Json, &note, offset);
            assert!(out.len() <= 1024);
            let v: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v["edited_by"], "amber-falcon");
            assert_eq!(v["edited_at"], "2026-10-23T14:00:00Z");
            assert_eq!(v["offset"], offset);
            recovered.push_str(v["text"].as_str().unwrap());
            let Some(next) = v["next_offset"].as_u64() else {
                break;
            };
            assert!(next as usize > offset);
            offset = next as usize;
        }
        assert_eq!(recovered, text);
        let out = fmt.note(Format::Text, &note, 0);
        assert!(out.len() <= 1024);
        assert!(out.contains("last edited by amber-falcon at 2026-10-23T14:00:00Z"));
        if text.len() > 1024 {
            assert!(out.contains("next_offset:"));
        } else {
            assert!(out.ends_with(&text));
        }
    }
}

#[test]
fn channel_lists_preview_notes_and_show_edit_attribution() {
    let note = kkt_rules::ChannelNote {
        channel_id: "ch_notes".into(),
        text: "A".repeat(1000),
        edited_by: Some("amber-falcon".into()),
        edited_at: Some("2026-10-23T14:00:00Z".into()),
    };
    let ch = kkt_rules::ChannelInfo {
        id: "ch_notes".into(),
        name: "notes".into(),
        purpose: None,
        note,
        open: true,
        created_by: "amber-falcon".into(),
        created_at: "2026-10-23T14:00:00Z".into(),
        owner: Some("amber-falcon".into()),
        member_count: 1,
        members_preview: vec!["amber-falcon".into()],
        is_member: true,
        new_messages: None,
    };
    let fmt = formatter(1024);
    let page = Page {
        items: vec![ch],
        next_cursor: None,
    };
    let out = fmt.channels(Format::Text, &page, 120);
    assert!(out.contains("note: "));
    assert!(out.contains("amber-falcon at 2026-10-23T14:00:00Z"));
    assert!(!out.contains(&"A".repeat(121)));
    let out = fmt.channels(Format::Json, &page, 120);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["channels"][0]["note"], format!("{}…", "A".repeat(120)));
    assert_eq!(v["channels"][0]["note_cut"], true);
    assert_eq!(v["channels"][0]["note_edited_by"], "amber-falcon");
    assert_eq!(v["channels"][0]["note_edited_at"], "2026-10-23T14:00:00Z");
    assert_eq!(v["channels"][0]["owner"], "amber-falcon");
}

#[test]
fn the_admin_sees_which_channels_are_hidden() {
    let info = |id: &str, open: bool| kkt_rules::ChannelInfo {
        id: id.into(),
        name: id.trim_start_matches("ch_").into(),
        purpose: Some("why".into()),
        note: kkt_rules::ChannelNote {
            channel_id: id.into(),
            text: String::new(),
            edited_by: None,
            edited_at: None,
        },
        open,
        created_by: "amber-falcon".into(),
        created_at: "2026-10-23T14:00:00Z".into(),
        owner: None,
        member_count: 0,
        members_preview: vec![],
        is_member: false,
        new_messages: None,
    };
    let page = Page {
        items: vec![
            kkt_rules::AdminChannel {
                info: info("ch_gone", false),
                hidden: true,
            },
            kkt_rules::AdminChannel {
                info: info("ch_lobby", true),
                hidden: false,
            },
        ],
        next_cursor: None,
    };
    let fmt = formatter(4096);
    let out = fmt.admin_channels(Format::Text, &page, 120);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].starts_with("[ch_gone] #gone · closed · hidden · 0 members"),
        "{out}"
    );
    assert!(
        lines[1].starts_with("[ch_lobby] #lobby · open · 0 members"),
        "{out}"
    );
    let v: Value = serde_json::from_str(&fmt.admin_channels(Format::Json, &page, 120)).unwrap();
    assert_eq!(v["channels"][0]["hidden"], true);
    assert_eq!(v["channels"][1]["hidden"], false);
    assert!(v["channels"][0].get("is_member").is_none());
    let one = fmt.admin_channel(Format::Text, &page.items[0]);
    assert!(
        one.starts_with("#gone [ch_gone] closed, hidden, created by amber-falcon"),
        "{one}"
    );
    assert!(one.contains("note: (never set)"), "{one}");
}

/// At the smallest cap, a long purpose of multibyte or escaped characters is
/// shortened before serializing, so the answer stays valid JSON.
#[test]
fn the_admin_channel_json_stays_valid_at_the_smallest_cap() {
    let cap = kkt_config::MIN_RESPONSE_BYTES;
    let fmt = formatter(cap);
    for unit in ["界", "\"", "\\", "\n", "\u{1}", "😀"] {
        let purpose = unit.repeat(500);
        let channel = kkt_rules::AdminChannel {
            info: kkt_rules::ChannelInfo {
                id: "ch_big".into(),
                name: "名前\"quoted\"".into(),
                purpose: Some(purpose.clone()),
                note: kkt_rules::ChannelNote {
                    channel_id: "ch_big".into(),
                    text: String::new(),
                    edited_by: Some("amber-falcon".into()),
                    edited_at: Some("2026-10-23T14:00:00Z".into()),
                },
                open: false,
                created_by: "amber-falcon".into(),
                created_at: "2026-10-23T14:00:00Z".into(),
                owner: Some("amber-falcon".into()),
                member_count: 3,
                members_preview: vec!["amber-falcon".into(), "brisk-otter".into()],
                is_member: false,
                new_messages: None,
            },
            hidden: true,
        };
        let out = fmt.admin_channel(Format::Json, &channel);
        assert!(out.len() <= cap, "{unit:?}: {} bytes", out.len());
        let v: Value =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{unit:?}: {e}: {out}"));
        assert_eq!(v["name"], "名前\"quoted\"");
        assert_eq!(v["hidden"], true);
        assert_eq!(v["purpose_cut"], true);
        assert_eq!(v["note_edited_by"], "amber-falcon");
        let shown = v["purpose"].as_str().unwrap();
        let kept = shown.strip_suffix('…').unwrap();
        assert!(
            !kept.is_empty() && purpose.starts_with(kept),
            "{unit:?}: {shown}"
        );

        let mut short = channel;
        short.info.purpose = Some(unit.repeat(3));
        let v: Value = serde_json::from_str(&fmt.admin_channel(Format::Json, &short)).unwrap();
        assert_eq!(v["purpose"], unit.repeat(3));
        assert_eq!(v["purpose_cut"], false);
        assert_eq!(v["members_preview"].as_array().unwrap().len(), 2);
    }
}

/// At the smallest cap, an agent's details fit: a long description and
/// client are shortened, and following `more: offset=` reaches every channel.
#[test]
fn the_admin_agent_details_fit_and_page_through_channels() {
    let cap = kkt_config::MIN_RESPONSE_BYTES;
    let fmt = formatter(cap);
    let channel = |n: usize| kkt_rules::ChannelInfo {
        id: format!("ch_{n:08}"),
        name: format!("channel-{n}"),
        purpose: None,
        note: kkt_rules::ChannelNote {
            channel_id: format!("ch_{n:08}"),
            text: String::new(),
            edited_by: None,
            edited_at: None,
        },
        open: n.is_multiple_of(2),
        created_by: "amber-falcon".into(),
        created_at: "2026-10-23T14:00:00Z".into(),
        owner: None,
        member_count: 1,
        members_preview: vec!["amber-falcon".into()],
        is_member: true,
        new_messages: None,
    };
    let view = kkt_rules::AgentView {
        agent: AgentInfo {
            handle: "amber-falcon".into(),
            created_at: "2026-10-23T14:00:00Z".into(),
            last_seen: None,
            last_sent: None,
            run_id: None,
            description: Some("界\"".repeat(150)),
            client: Some(RecordedClient {
                name: "c".repeat(2000),
                version: "1.0".into(),
                recorded_at: "2026-10-23T14:00:00Z".into(),
                run_id: 1,
            }),
            listening: true,
            waiting_channels: vec!["ch_00000001".into()],
            lane: None,
        },
        channels: (0..40).map(channel).collect(),
        message_count: 7,
        memory: vec![
            kkt_rules::MemoryEntry {
                handle: "amber-falcon".into(),
                name: "handoff".into(),
                text: "Next: the parser.".into(),
                saved_at: "2026-10-23T15:00:00Z".into(),
            },
            kkt_rules::MemoryEntry {
                handle: "amber-falcon".into(),
                name: "rules".into(),
                text: "Ask first.".into(),
                saved_at: "2026-10-23T15:01:00Z".into(),
            },
        ],
    };
    let mut seen = Vec::new();
    let mut offset = 0;
    loop {
        let out = fmt.admin_agent(Format::Json, &view, offset);
        assert!(out.len() <= cap, "{} bytes at offset {offset}", out.len());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["memory_entries"], 2);
        assert_eq!(v["memory_chars"], 27);
        assert_eq!(v["message_count"], 7);
        let channels = v["channels"].as_array().unwrap();
        assert!(!channels.is_empty(), "{out}");
        seen.extend(
            channels
                .iter()
                .map(|c| c["id"].as_str().unwrap().to_string()),
        );
        match v["more"].as_str() {
            None => break,
            Some(more) => offset = more.strip_prefix("offset=").unwrap().parse().unwrap(),
        }
    }
    let all: Vec<_> = view.channels.iter().map(|c| c.id.clone()).collect();
    assert_eq!(seen, all);
    let text = fmt.admin_agent(Format::Text, &view, 0);
    assert!(text.len() <= cap);
    assert!(text.ends_with("more: offset=1"), "{text}");
    let text = formatter(64 * 1024).admin_agent(Format::Text, &view, 0);
    assert!(
        text.contains("memory: 2 entries, 27 characters (list_memories, recall)"),
        "{text}"
    );
    assert!(
        text.contains("[ch_00000001] #channel-1 · closed · waiting"),
        "{text}"
    );
}

#[test]
fn one_channel_names_its_owner_on_a_line_after_the_old_ones() {
    let mut ch = kkt_rules::ChannelInfo {
        id: "ch_owned".into(),
        name: "owned".into(),
        purpose: None,
        note: kkt_rules::ChannelNote {
            channel_id: "ch_owned".into(),
            text: String::new(),
            edited_by: None,
            edited_at: None,
        },
        open: false,
        created_by: "amber-falcon".into(),
        created_at: "2026-10-23T14:00:00Z".into(),
        owner: Some("brave-beaver".into()),
        member_count: 2,
        members_preview: vec!["amber-falcon".into(), "brave-beaver".into()],
        is_member: true,
        new_messages: None,
    };
    let fmt = formatter(8000);
    assert_eq!(
        fmt.channel(Format::Text, &ch),
        "#owned [ch_owned] closed, created by amber-falcon\n\
         members (2): amber-falcon, brave-beaver\n\
         you are a member\n\
         owner: brave-beaver"
    );
    let v: Value = serde_json::from_str(&fmt.channel(Format::Json, &ch)).unwrap();
    assert_eq!(v["owner"], "brave-beaver");
    assert_eq!(v["created_by"], "amber-falcon");
    ch.owner = None;
    assert!(fmt.channel(Format::Text, &ch).ends_with("\nowner: (none)"));
    let v: Value = serde_json::from_str(&fmt.channel(Format::Json, &ch)).unwrap();
    assert_eq!(v["owner"], Value::Null);
}

#[test]
fn escaped_note_previews_do_not_hide_a_channel_at_the_minimum_cap() {
    let note = kkt_rules::ChannelNote {
        channel_id: "ch_notes".into(),
        text: "\0".repeat(1000),
        edited_by: Some("amber-falcon".into()),
        edited_at: Some("2026-10-23T14:00:00Z".into()),
    };
    let ch = kkt_rules::ChannelInfo {
        id: "ch_notes".into(),
        name: "a".repeat(64),
        purpose: Some("p".repeat(120)),
        note,
        open: true,
        created_by: "amber-falcon".into(),
        created_at: "2026-10-23T14:00:00Z".into(),
        owner: Some("amber-falcon".into()),
        member_count: 3,
        members_preview: vec![
            "amber-falcon".into(),
            "silver-ferret".into(),
            "brave-beaver".into(),
        ],
        is_member: true,
        new_messages: None,
    };
    let page = Page {
        items: vec![ch],
        next_cursor: None,
    };
    let out = formatter(1024).channels(Format::Json, &page, 120);
    assert!(out.len() <= 1024);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["channels"].as_array().unwrap().len(), 1);
    assert_eq!(v["channels"][0]["note_cut"], true);
    assert!(v["channels"][0]["note"].as_str().unwrap().starts_with('\0'));
}

#[test]
fn many_waiting_channels_keep_paginated_agents_and_members_visible() {
    let fmt = formatter(kkt_config::MIN_RESPONSE_BYTES);
    for description in ["reviewer".into(), "😀".repeat(300), "\u{1}\n".repeat(150)] {
        let mut a = described("amber-falcon", Some(&description));
        a.listening = true;
        a.waiting_channels = (0..500).map(|i| format!("ch_{i:08}")).collect();
        let members = Members {
            channel_id: "ch_public".into(),
            total: 2,
            owner: Some("amber-falcon".into()),
            page: Page {
                items: vec![a],
                next_cursor: Some("amber-falcon".into()),
            },
        };
        for format in [Format::Text, Format::Json] {
            for out in [
                fmt.agents(format, &members.page),
                fmt.members(format, &members),
            ] {
                assert!(out.len() <= kkt_config::MIN_RESPONSE_BYTES);
                assert!(!out.contains("larger than"), "{out}");
                if matches!(format, Format::Json) {
                    let json: Value = serde_json::from_str(&out).unwrap();
                    let key = if json.get("agents").is_some() {
                        "agents"
                    } else {
                        "members"
                    };
                    assert_eq!(json[key].as_array().unwrap().len(), 1);
                    assert_eq!(json[key][0]["handle"], "amber-falcon");
                    assert!(json[key][0]["waiting_channels_more"].as_u64().unwrap() > 0);
                    assert_eq!(json[key][0]["listening"], true);
                    assert_eq!(json["more"], "cursor=amber-falcon");
                } else {
                    assert!(out.contains("amber-falcon"));
                    assert!(out.contains("more waiting channels"));
                    assert!(out.ends_with("more: cursor=amber-falcon"));
                }
            }
        }
    }
}

#[test]
fn a_long_memory_entry_reads_back_whole_in_parts_within_the_cap() {
    let cap = kkt_config::MIN_RESPONSE_BYTES;
    let text = "😀 \"quoted\"\nline ".repeat(200);
    let entry = kkt_rules::MemoryEntry {
        handle: "amber-falcon".into(),
        name: "handoff".into(),
        text: text.clone(),
        saved_at: "2026-10-23T14:00:00Z".into(),
    };
    let fmt = formatter(cap);
    for f in [Format::Text, Format::Json] {
        let mut read = String::new();
        let mut offset = 0;
        let mut parts = 0;
        loop {
            let out = fmt.memory_entry(f, &entry, offset);
            assert!(out.len() <= cap, "{} bytes", out.len());
            parts += 1;
            let next = match f {
                Format::Json => {
                    let json: Value = serde_json::from_str(&out).expect("valid JSON");
                    read.push_str(json["text"].as_str().expect("text"));
                    json["next_offset"].as_u64().map(|n| n as usize)
                }
                Format::Text => {
                    let (_, rest) = out.split_once("offset: ").expect("offset line");
                    let (_, body) = rest.split_once('\n').expect("text");
                    match body.rsplit_once("\nnext_offset: ") {
                        Some((part, tail)) => {
                            read.push_str(part);
                            tail.split(' ').next().and_then(|n| n.parse().ok())
                        }
                        None => {
                            read.push_str(body);
                            None
                        }
                    }
                }
            };
            match next {
                Some(n) => offset = n,
                None => break,
            }
        }
        assert!(parts > 1, "the entry needed several parts");
        assert_eq!(read, text, "nothing is lost");
    }
}

#[test]
fn a_short_memory_entry_names_itself_and_its_avatar() {
    let entry = kkt_rules::MemoryEntry {
        handle: "amber-falcon".into(),
        name: "handoff".into(),
        text: "Next: the parser.".into(),
        saved_at: "2026-10-23T14:00:00Z".into(),
    };
    assert_eq!(
        formatter(8000).memory_entry(Format::Text, &entry, 0),
        "memory entry `handoff` of amber-falcon\nsaved at 2026-10-23T14:00:00Z\noffset: 0\nNext: the parser."
    );
}

fn summary(name: &str, chars: usize) -> kkt_rules::MemorySummary {
    kkt_rules::MemorySummary {
        name: name.into(),
        chars,
        saved_at: "2026-10-23T14:00:00Z".into(),
    }
}

#[test]
fn memory_lists_fit_the_cap_and_follow_their_cursor() {
    use kkt_rules::MemoryList;
    let items: Vec<_> = (0..60)
        .map(|i| summary(&format!("entry-{i:02}"), i))
        .collect();
    let list = MemoryList {
        handle: "amber-falcon".into(),
        page: Page {
            items,
            next_cursor: Some("entry-59".into()),
        },
        entries: 75,
        chars: 1770,
        max_chars: 100_000,
    };
    let cap = kkt_config::MIN_RESPONSE_BYTES;
    let fmt = formatter(cap);
    let text = fmt.memory_list(Format::Text, &list);
    assert!(text.len() <= cap);
    assert!(
        text.starts_with("memory of amber-falcon: 75 entries, 1770 of 100000 characters\nentry-00  0 characters, saved "),
        "{text}"
    );
    let last_shown = text
        .lines()
        .rev()
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert!(
        text.ends_with(&format!("more: cursor={last_shown}")),
        "{text}"
    );
    let all = formatter(64 * 1024).memory_list(Format::Json, &list);
    let v: Value = serde_json::from_str(&all).unwrap();
    assert_eq!(v["memory"].as_array().unwrap().len(), 60);
    assert_eq!(v["memory"][3]["chars"], 3);
    assert_eq!(v["more"], "cursor=entry-59");
    assert_eq!(v["entries"], 75);
    assert_eq!(v["max_memory_chars"], 100_000);
}

#[test]
fn saving_an_entry_says_how_full_the_memory_is() {
    let saved = kkt_rules::MemorySaved {
        entry: summary("handoff", 12),
        replaced: true,
        entries: 3,
        chars: 40,
        max_chars: 100,
    };
    let fmt = formatter(8000);
    assert_eq!(
        fmt.memory_saved(Format::Text, &saved),
        "replaced `handoff` (12 characters)\nmemory: 3 entries, 40 of 100 characters"
    );
    let v: Value = serde_json::from_str(&fmt.memory_saved(Format::Json, &saved)).unwrap();
    assert_eq!(v["replaced"], true);
    assert_eq!(v["memory_chars"], 40);
}

#[test]
fn reclaim_names_the_memory_entries_up_to_a_bound() {
    use kkt_config::RECLAIM_MEMORY_NAMES;
    let reclaimed = |memory| kkt_rules::Reclaimed {
        handle: "amber-falcon".into(),
        token: "act_x".into(),
        server_announcement: None,
        lane: None,
        memory,
    };
    let fmt = formatter(8000);
    let none = fmt.reclaimed(Format::Text, &reclaimed(Vec::new()));
    assert!(!none.contains("memory"), "{none}");
    let two = fmt.reclaimed(
        Format::Text,
        &reclaimed(vec![summary("handoff", 17), summary("rules", 9)]),
    );
    assert!(
        two.ends_with(
            "\n\nYour memory (2 entries; `recall` reads one):\n\
             - handoff (17 characters, saved 2026-10-23T14:00:00Z)\n\
             - rules (9 characters, saved 2026-10-23T14:00:00Z)"
        ),
        "{two}"
    );
    let many: Vec<_> = (0..RECLAIM_MEMORY_NAMES + 3)
        .map(|i| summary(&format!("e-{i:03}"), 1))
        .collect();
    let text = fmt.reclaimed(Format::Text, &reclaimed(many.clone()));
    let last = many[RECLAIM_MEMORY_NAMES - 1].name.clone();
    assert!(
        text.ends_with(&format!("and 3 more: `list_memories` with cursor={last}")),
        "{text}"
    );
    let v: Value = serde_json::from_str(&fmt.reclaimed(Format::Json, &reclaimed(many))).unwrap();
    assert_eq!(v["memory"].as_array().unwrap().len(), RECLAIM_MEMORY_NAMES);
    assert_eq!(v["memory_entries"], RECLAIM_MEMORY_NAMES + 3);
    assert_eq!(v["memory_cursor"], last.as_str());
}

#[test]
fn current_server_announcement_text_and_version_are_returned_whole() {
    let instructions = kkt_rules::ServerAnnouncement {
        version: 7,
        text: "\u{1}😀\n".repeat(Limits::DEFAULT.max_server_announcement_chars / 3),
        saved_at: "2026-10-27T00:00:00Z".into(),
        run_id: 3,
        saved_by_admin: None,
    };
    let registration = kkt_rules::Registration {
        handle: "amber-falcon".into(),
        token: "act_example".into(),
        recovery_code: "arc_example".into(),
        server_announcement: Some(instructions.clone()),
        lane: None,
    };
    let fmt = formatter(kkt_config::MIN_RESPONSE_BYTES);
    for format in [Format::Text, Format::Json] {
        let registration_text = fmt.registration(format, &registration);
        let whoami = fmt.whoami(format, &registration.handle, Some(&instructions), None);
        for out in [registration_text, whoami] {
            if format == Format::Json {
                let value: Value = serde_json::from_str(&out).unwrap();
                // An avatar's handle is `handle`; there is no `name` field.
                assert_eq!(value["handle"], "amber-falcon");
                assert!(value.get("name").is_none());
                assert_eq!(value["server_version"], SERVER_VERSION);
                assert_eq!(value["server_announcement"]["text"], instructions.text);
                assert_eq!(value["server_announcement"]["version"], 7);
                assert!(value.get("operator_instructions").is_none());
            } else {
                assert!(out.starts_with("handle: amber-falcon\n"), "{out}");
                assert!(
                    out.lines()
                        .any(|line| line == format!("server_version: {SERVER_VERSION}"))
                );
                assert!(out.ends_with(&instructions.text));
                assert!(out.contains("Server announcement (version 7), saved by the operator:"));
                assert!(!out.contains("operator_instructions"));
            }
        }
    }
}

#[test]
fn server_announcement_says_who_saved_it() {
    let human = kkt_rules::ServerAnnouncement {
        version: 2,
        text: "Be brief.".into(),
        saved_at: "2026-10-27T00:00:00Z".into(),
        run_id: 3,
        saved_by_admin: None,
    };
    let admin = kkt_rules::ServerAnnouncement {
        version: 3,
        saved_by_admin: Some("admin-sentinel".into()),
        ..human.clone()
    };
    let fmt = formatter(8000);

    // Agents, at connection and from whoami and register.
    assert!(
        fmt.whoami(Format::Text, "amber-falcon", Some(&human), None)
            .contains("\nServer announcement (version 2), saved by the operator:\nBe brief.")
    );
    assert!(
        fmt.whoami(Format::Text, "amber-falcon", Some(&admin), None).contains(
            "\nServer announcement (version 3), saved by the operator (via admin-sentinel):\nBe brief."
        )
    );
    let json = |v| -> Value {
        serde_json::from_str(&fmt.whoami(Format::Json, "amber-falcon", Some(v), None)).unwrap()
    };
    assert_eq!(
        json(&human)["server_announcement"]["saved_by_admin"],
        Value::Null
    );
    assert_eq!(
        json(&admin)["server_announcement"]["saved_by_admin"],
        "admin-sentinel"
    );

    // Admin agents.
    assert_eq!(
        fmt.server_announcement(Format::Text, Some(&human)),
        "server announcement, version 2, saved 2026-10-27T00:00:00Z by the operator:\nBe brief."
    );
    assert_eq!(
        fmt.server_announcement(Format::Text, Some(&admin)),
        "server announcement, version 3, saved 2026-10-27T00:00:00Z by the operator \
         (via admin-sentinel):\nBe brief."
    );
}
