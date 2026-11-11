//! The inventory, over HTTP: two swarms seeded with words that name them,
//! then everything each swarm's callers can read, read as them. No
//! complete result, in text or JSON, at any page size, may carry anything
//! of the other swarm: its words, handles, channel IDs, tokens or recovery
//! codes. Each read is also checked to show the reader's own swarm, so a
//! read that shows nothing at all does not pass for one that hides well.
//!
//! What is read (roadmap section 5, in core form):
//! - agent tools: channel and agent lists with their member previews,
//!   counts and waiting-channel lists, member lists, activity, reading,
//!   search, notes, single messages, memory, and `whoami`; each list read
//!   whole and one row per page;
//! - the other swarm's channel IDs, handles and message IDs, asked for by
//!   name: `not_found`, with nothing but what was asked repeated back;
//! - every web page the operator sees, hidden channels and agents
//!   included;
//! - the live feed: every field of every event, member counts after joins
//!   and leaves, hidden channels and agents;
//! - an agent's wait, and the waiting-channel lists while it waits;
//! - `/mcp-admin`, as each swarm's admin.
//!
//! Markers catch foreign text, not foreign numbers: a count, an order or a
//! page boundary that the other swarm changes carries no marker. So the
//! same reads are also made twice, over a fixed clock and seeded entropy,
//! once with the other swarm empty and once beside it, and must be the
//! same byte for byte
//! (`each_swarm_reads_the_same_whether_or_not_the_other_is_there`).

use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_fake_product::client::{Answer, Client, Event};
use kkt_fake_product::{Bearers, Product, main, other};
use kkt_rules::SwarmId;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::time::Duration;

/// One agent, as `register` gave it.
#[derive(Clone)]
struct Avatar {
    handle: String,
    token: String,
    recovery_code: String,
}

/// What a swarm was seeded with.
struct Seeded {
    swarm: SwarmId,
    /// The word in everything written in this swarm, and nowhere else.
    tag: &'static str,
    /// Agents: 0 owns the channels; 1 is in them all and waits; 2 joined and
    /// left the open channel, then unregistered; 3 was in them all, then
    /// hidden (and so revoked) by the operator.
    agents: Vec<Avatar>,
    /// The open, closed and hidden channels.
    channels: Vec<String>,
    /// Every message sent in it.
    messages: Vec<i64>,
}

impl Seeded {
    fn bearer(&self) -> &'static str {
        Bearers::agents(&self.swarm)
    }

    fn owner(&self) -> &Avatar {
        &self.agents[0]
    }

    /// Everything that names this swarm's data.
    fn markers(&self) -> Vec<String> {
        let mut markers = vec![self.tag.to_string()];
        for agent in &self.agents {
            markers.push(agent.handle.clone());
            markers.push(agent.token.clone());
            markers.push(agent.recovery_code.clone());
        }
        markers.extend(self.channels.iter().cloned());
        markers
    }

    /// Fails if `text`, shown to the other swarm at `place`, names this
    /// swarm, except for `asked`: what the reader asked for by name and may
    /// be told back.
    fn assert_absent(&self, place: &str, text: &str, asked: &[&str]) {
        let lower = text.to_lowercase();
        for marker in self.markers() {
            if asked.contains(&marker.as_str()) {
                continue;
            }
            assert!(
                !lower.contains(&marker.to_lowercase()),
                "{place} shows {marker} of {}:\n{text}",
                self.swarm.as_str()
            );
        }
    }
}

/// A message body in the swarm tagged `tag`. Every body has `sharedword`,
/// so a search for it would find both swarms' messages if it could.
fn body(tag: &str, what: &str) -> String {
    format!("{tag}-{what} sharedword")
}

fn avatar(answer: &Answer) -> Avatar {
    assert!(!answer.is_error, "{}", answer.text);
    let json = answer.json();
    Avatar {
        handle: json["handle"].as_str().expect("a handle").into(),
        token: json["token"].as_str().expect("a token").into(),
        recovery_code: json["recovery_code"]
            .as_str()
            .expect("a recovery code")
            .into(),
    }
}

/// Calls an agent tool that must succeed, and returns its JSON.
async fn ok(client: &Client, bearer: &str, tool: &str, arguments: Value) -> Value {
    let answer = client.tool(bearer, tool, arguments).await;
    assert!(!answer.is_error, "{tool}: {}", answer.text);
    answer.json()
}

/// Seeds `swarm` over HTTP, as its agents and its operator.
async fn seed(client: &Client, swarm: SwarmId, tag: &'static str) -> Seeded {
    let bearer = Bearers::agents(&swarm);
    let mut agents = Vec::new();
    for _ in 0..4 {
        agents.push(avatar(
            &client
                .tool(bearer, "register", json!({ "format": "json" }))
                .await,
        ));
    }
    let [a0, a1, a2, a3] = [0, 1, 2, 3].map(|i| agents[i].clone());
    for (agent, what) in [(&a0, "desc-0"), (&a2, "desc-2"), (&a3, "desc-3")] {
        ok(
            client,
            bearer,
            "set_description",
            json!({ "token": agent.token, "text": format!("{tag}-{what}"), "format": "json" }),
        )
        .await;
    }
    for (agent, name) in [(&a0, "handoff"), (&a1, "notes")] {
        ok(
            client,
            bearer,
            "remember",
            json!({ "token": agent.token, "name": name, "text": format!("{tag}-memory"), "format": "json" }),
        )
        .await;
    }

    let mut channels = Vec::new();
    let mut messages = Vec::new();
    for (name, open) in [("open", true), ("closed", false), ("hidden", true)] {
        let channel = ok(
            client,
            bearer,
            "create_channel",
            json!({
                "token": a0.token, "name": format!("{tag}-{name}"), "open": open,
                "purpose": format!("{tag}-purpose"), "members": [a1.handle, a3.handle],
                "format": "json",
            }),
        )
        .await["id"]
            .as_str()
            .expect("an ID")
            .to_string();
        ok(
            client,
            bearer,
            "set_note",
            json!({ "token": a1.token, "channel_id": channel, "text": format!("{tag}-note"), "format": "json" }),
        )
        .await;
        for (i, agent) in [&a0, &a1, &a3].into_iter().enumerate() {
            let sent = ok(
                client,
                bearer,
                "send",
                json!({ "token": agent.token, "channel_id": channel, "body": body(tag, &format!("message-{i}")), "format": "json" }),
            )
            .await;
            messages.push(sent["message_id"].as_i64().expect("a message ID"));
        }
        channels.push(channel);
    }
    // A member who joins, speaks and leaves, then unregisters.
    ok(
        client,
        bearer,
        "join_channel",
        json!({ "token": a2.token, "channel_id": channels[0], "format": "json" }),
    )
    .await;
    let sent = ok(
        client,
        bearer,
        "send",
        json!({ "token": a2.token, "channel_id": channels[0], "body": body(tag, "passing"), "format": "json" }),
    )
    .await;
    messages.push(sent["message_id"].as_i64().expect("a message ID"));
    ok(
        client,
        bearer,
        "leave_channel",
        json!({ "token": a2.token, "channel_id": channels[0], "format": "json" }),
    )
    .await;
    ok(
        client,
        bearer,
        "hide_channel",
        json!({ "token": a0.token, "channel_id": channels[2], "format": "json" }),
    )
    .await;
    ok(
        client,
        bearer,
        "unregister",
        json!({ "token": a2.token, "format": "json" }),
    )
    .await;

    // The operator hides an agent.
    let operator = client.sign_in(Bearers::operator(&swarm)).await;
    let page = format!("/agents/{}", a3.handle);
    let (status, _) = client
        .form(&operator, &page, &format!("{page}/hide"), &[])
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "hid {}", a3.handle);

    Seeded {
        swarm,
        tag,
        agents,
        channels,
        messages,
    }
}

/// `cursor` from a JSON answer's `more` line, if it gives one.
fn cursor(answer: &Value) -> Option<String> {
    answer["more"]
        .as_str()?
        .split_whitespace()
        .find_map(|part| part.strip_prefix("cursor="))
        .map(str::to_string)
}

/// Reads as `reader`, checking each answer against `foreign`. Returns
/// everything read, for the checks that the reader saw its own.
struct Reader<'a> {
    client: &'a Client,
    reader: &'a Seeded,
    foreign: &'a Seeded,
    seen: String,
}

impl Reader<'_> {
    /// Calls `tool` as the reader's owner, in text and in JSON; neither may
    /// name the other swarm, beyond `asked`.
    async fn both(&mut self, tool: &str, arguments: Value, asked: &[&str]) -> Answer {
        let mut text = arguments.clone();
        text["token"] = json!(self.reader.owner().token);
        let answer = self
            .client
            .tool(self.reader.bearer(), tool, text.clone())
            .await;
        self.check(&format!("{tool} {text}"), &answer, asked);
        let mut as_json = text;
        as_json["format"] = json!("json");
        let answer = self
            .client
            .tool(self.reader.bearer(), tool, as_json.clone())
            .await;
        self.check(&format!("{tool} {as_json}"), &answer, asked);
        answer
    }

    fn check(&mut self, place: &str, answer: &Answer, asked: &[&str]) {
        self.foreign.assert_absent(place, &answer.raw, asked);
        self.seen.push_str(&answer.raw);
        self.seen.push('\n');
    }

    /// Reads a list tool whole, then one row per page to its end.
    async fn pages(&mut self, tool: &str, arguments: Value) -> usize {
        let whole = self.both(tool, arguments.clone(), &[]).await;
        assert!(!whole.is_error, "{tool}: {}", whole.text);
        let mut page = arguments;
        page["limit"] = json!(1);
        let mut pages = 0;
        loop {
            let answer = self.both(tool, page.clone(), &[]).await;
            assert!(!answer.is_error, "{tool}: {}", answer.text);
            pages += 1;
            match cursor(&answer.json()) {
                Some(next) => page["cursor"] = json!(next),
                None => return pages,
            }
            assert!(pages < 50, "{tool} pages without end");
        }
    }

    /// Asks for something of the other swarm by name: it does not exist.
    async fn missing(&mut self, tool: &str, arguments: Value, asked: &[&str]) {
        let answer = self.both(tool, arguments, asked).await;
        assert!(answer.is_error, "{tool} found {asked:?}: {}", answer.text);
        assert!(
            answer.text.contains("not_found"),
            "{tool} {asked:?}: {}",
            answer.text
        );
    }
}

/// Every agent-tool read, as `reader`'s owner.
async fn agent_reads(client: &Client, reader: &Seeded, foreign: &Seeded) -> String {
    let mut r = Reader {
        client,
        reader,
        foreign,
        seen: String::new(),
    };
    r.both("whoami", json!({}), &[]).await;
    assert!(r.pages("list_channels", json!({})).await >= 2);
    r.pages("list_channels", json!({ "member_only": true }))
        .await;
    r.pages("list_agents", json!({})).await;
    r.both("list_agents", json!({ "query": "desc" }), &[]).await;
    r.both("list_channels", json!({ "query": "sharedword" }), &[])
        .await;
    r.pages("list_memories", json!({})).await;
    r.both("recall", json!({ "name": "handoff" }), &[]).await;
    r.both("activity", json!({}), &[]).await;
    r.both("activity", json!({ "after_id": 0 }), &[]).await;
    // A hidden channel is for operators: agents cannot find it.
    r.missing("read", json!({ "channel_id": reader.channels[2] }), &[])
        .await;
    for channel in &reader.channels[..2] {
        r.pages("list_members", json!({ "channel_id": channel }))
            .await;
        r.both("read", json!({ "channel_id": channel, "limit": 100 }), &[])
            .await;
        r.both("read", json!({ "channel_id": channel, "after_id": 0 }), &[])
            .await;
        r.both("get_note", json!({ "channel_id": channel }), &[])
            .await;
    }
    let search = r
        .both(
            "search_messages",
            json!({ "query": "sharedword", "limit": 100 }),
            &[],
        )
        .await;
    assert!(!search.is_error, "{}", search.text);
    r.both(
        "search_messages",
        json!({ "query": "sharedword", "limit": 1 }),
        &[],
    )
    .await;
    r.both(
        "search_messages",
        json!({ "query": "sharedword", "phrase": true }),
        &[],
    )
    .await;
    for id in &reader.messages {
        r.both("get_message", json!({ "message_id": id }), &[])
            .await;
    }
    // A wait with messages waiting returns at once, one message and a
    // count of the rest.
    for channels in [json!(null), json!(&reader.channels[..2])] {
        let answer = r
            .both(
                "wait_for_messages",
                json!({ "channel_ids": channels, "after_id": 0, "limit": 1, "timeout_s": 1 }),
                &[],
            )
            .await;
        assert!(
            answer.json()["more_waiting"].as_i64() > Some(0),
            "{}",
            answer.text
        );
    }

    // Asked for by name, the other swarm's things do not exist.
    for id in &foreign.messages {
        r.missing("get_message", json!({ "message_id": id }), &[])
            .await;
    }
    for channel in &foreign.channels {
        let asked = [channel.as_str()];
        for tool in ["read", "get_note", "list_members", "join_channel"] {
            r.missing(tool, json!({ "channel_id": channel }), &asked)
                .await;
        }
        r.missing(
            "send",
            json!({ "channel_id": channel, "body": "hello" }),
            &asked,
        )
        .await;
    }
    for agent in &foreign.agents {
        let asked = [agent.handle.as_str()];
        // Neither a filter nor a member: nothing to find.
        let filtered = r
            .both(
                "list_channels",
                json!({ "with_agent": agent.handle }),
                &asked,
            )
            .await;
        r.foreign
            .assert_absent("with_agent", &filtered.text, &asked);
        r.both(
            "read",
            json!({ "channel_id": reader.channels[0], "from_agent": agent.handle }),
            &asked,
        )
        .await;
        r.both(
            "add_to_channel",
            json!({ "channel_id": reader.channels[0], "agents": [agent.handle] }),
            &asked,
        )
        .await;
    }
    // The other swarm's tokens and recovery codes are nobody's here.
    for agent in &foreign.agents {
        let asked = [agent.token.as_str()];
        let answer = client
            .tool(reader.bearer(), "whoami", json!({ "token": agent.token }))
            .await;
        assert!(answer.is_error, "{}", answer.text);
        r.check("whoami with its token", &answer, &asked);
        let asked = [agent.handle.as_str(), agent.recovery_code.as_str()];
        let answer = client
            .tool(
                reader.bearer(),
                "reclaim",
                json!({ "handle": agent.handle, "recovery_code": agent.recovery_code }),
            )
            .await;
        assert!(answer.is_error, "{}", answer.text);
        r.check("reclaim with its code", &answer, &asked);
    }
    r.seen
}

/// Every web page `reader`'s operator sees, signed in with `cookie`.
async fn web_reads(client: &Client, cookie: &str, reader: &Seeded, foreign: &Seeded) -> String {
    let mut seen = String::new();
    let newest = reader
        .messages
        .iter()
        .chain(&foreign.messages)
        .max()
        .expect("messages");
    let mut pages = vec![
        "/".to_string(),
        "/?hidden=1".to_string(),
        "/stale-agents?hours=1".to_string(),
        "/history".to_string(),
        "/lanes".to_string(),
        "/announcement".to_string(),
    ];
    for channel in &reader.channels {
        pages.push(format!("/channels/{channel}"));
        pages.push(format!(
            "/channels/{channel}/messages?before_id={}",
            newest + 1
        ));
    }
    // Agent 2 unregistered and agent 3 was hidden: neither has a page.
    for i in [0, 1] {
        pages.push(format!("/agents/{}", reader.agents[i].handle));
    }
    for path in &pages {
        let (status, page) = client.page(cookie, path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {page}");
        foreign.assert_absent(path, &page, &[]);
        seen += &page;
    }
    let mut missing: Vec<(String, &str)> = Vec::new();
    for channel in &foreign.channels {
        missing.push((format!("/channels/{channel}"), channel));
        missing.push((
            format!("/channels/{channel}/messages?before_id={}", newest + 1),
            channel,
        ));
    }
    for agent in &foreign.agents {
        missing.push((format!("/agents/{}", agent.handle), &agent.handle));
    }
    for (path, asked) in &missing {
        let (status, page) = client.page(cookie, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        foreign.assert_absent(path, &page, &[asked]);
        seen += &page;
    }
    seen
}

/// Every admin-tool read, as `reader`'s admin.
async fn admin_reads(client: &Client, reader: &Seeded, foreign: &Seeded) -> String {
    let bearer = Bearers::admin(&reader.swarm);
    let mut seen = String::new();
    let mut check = async |tool: &str, arguments: Value, asked: &[&str], found: bool| {
        for format in ["text", "json"] {
            let mut arguments = arguments.clone();
            arguments["format"] = json!(format);
            let answer = client.admin(bearer, tool, arguments.clone()).await;
            let place = format!("/mcp-admin {tool} {arguments}");
            foreign.assert_absent(&place, &answer.raw, asked);
            assert_eq!(!answer.is_error, found, "{place}: {}", answer.text);
            seen += &answer.raw;
        }
    };
    for tool in ["list_channels", "list_agents"] {
        check(tool, json!({}), &[], true).await;
        check(tool, json!({ "limit": 1 }), &[], true).await;
    }
    check("activity", json!({ "after_id": 0 }), &[], true).await;
    check("new_messages", json!({ "after_id": 0 }), &[], true).await;
    check(
        "new_messages",
        json!({ "after_id": 0, "limit": 1 }),
        &[],
        true,
    )
    .await;
    check("list_stale_agents", json!({ "idle_hours": 1 }), &[], true).await;
    for channel in &reader.channels {
        for tool in ["get_channel", "read", "get_note", "list_members"] {
            check(tool, json!({ "channel_id": channel }), &[], true).await;
        }
    }
    for agent in &reader.agents[..2] {
        check("get_agent", json!({ "handle": agent.handle }), &[], true).await;
        check(
            "list_memories",
            json!({ "handle": agent.handle }),
            &[],
            true,
        )
        .await;
    }
    for id in &reader.messages {
        check("get_message", json!({ "message_id": id }), &[], true).await;
    }
    for id in &foreign.messages {
        check("get_message", json!({ "message_id": id }), &[], false).await;
    }
    for channel in &foreign.channels {
        for tool in ["get_channel", "read", "get_note", "list_members"] {
            check(tool, json!({ "channel_id": channel }), &[channel], false).await;
        }
    }
    for agent in &foreign.agents {
        let asked = [agent.handle.as_str()];
        check(
            "get_agent",
            json!({ "handle": agent.handle }),
            &asked,
            false,
        )
        .await;
        check(
            "list_memories",
            json!({ "handle": agent.handle }),
            &asked,
            false,
        )
        .await;
        check(
            "recall",
            json!({ "handle": agent.handle, "name": "handoff" }),
            &asked,
            false,
        )
        .await;
    }
    seen
}

/// Fails unless `seen` shows each of `own`'s channels and agents, and its
/// words.
fn assert_shows_own(place: &str, seen: &str, own: &Seeded, channels: usize, agents: &[usize]) {
    assert!(
        seen.contains(own.tag),
        "{place} shows none of its own words"
    );
    for channel in &own.channels[..channels] {
        assert!(seen.contains(channel), "{place} never shows {channel}");
    }
    for i in agents {
        let handle = &own.agents[*i].handle;
        assert!(seen.contains(handle), "{place} never shows {handle}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_of_one_swarm_reaches_the_other() {
    let product = Product::start(
        memory(),
        system_clock(),
        os_entropy(),
        kkt_config::Limits::DEFAULT,
    )
    .await;
    let client = Client::new(&product);

    // Each swarm's operator watches the live feed from the start.
    let mut feeds = Vec::new();
    for swarm in [main(), other()] {
        let cookie = client.sign_in(Bearers::operator(&swarm)).await;
        feeds.push(client.events(&cookie).await);
    }
    let seeded = [
        seed(&client, main(), "mainx").await,
        seed(&client, other(), "otherx").await,
    ];

    // One agent in each swarm waits on its closed channel, and is seen to.
    let mut waits = Vec::new();
    for s in &seeded {
        let latest = ok(
            &client,
            s.bearer(),
            "read",
            json!({ "token": s.agents[1].token, "channel_id": s.channels[1], "format": "json" }),
        )
        .await;
        let after = latest["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .filter_map(|m| m["id"].as_i64())
            .max()
            .expect("a newest message");
        let wait = tokio::spawn({
            let client = Client::new(&product);
            let (bearer, token, channel) =
                (s.bearer(), s.agents[1].token.clone(), s.channels[1].clone());
            async move {
                client
                    .tool(
                        bearer,
                        "wait_for_messages",
                        json!({ "token": token, "channel_ids": [channel], "after_id": after, "timeout_s": 60, "format": "json" }),
                    )
                    .await
            }
        });
        waits.push(wait);
    }
    for s in &seeded {
        let mut listening = false;
        for _ in 0..100 {
            let agents = ok(
                &client,
                s.bearer(),
                "list_agents",
                json!({ "token": s.owner().token, "format": "json" }),
            )
            .await;
            listening = agents["agents"]
                .as_array()
                .expect("agents")
                .iter()
                .any(|a| {
                    a["handle"] == json!(s.agents[1].handle)
                        && a["waiting_channels"].to_string().contains(&s.channels[1])
                });
            if listening {
                break;
            }
            let clock = SystemClock;
            clock
                .sleep_until(clock.now() + Duration::from_millis(20))
                .await;
        }
        assert!(listening, "{} shows its own wait", s.swarm.as_str());
    }

    for (reader, foreign) in [(&seeded[0], &seeded[1]), (&seeded[1], &seeded[0])] {
        let place = format!("{}'s agent tools", reader.swarm.as_str());
        let seen = agent_reads(&client, reader, foreign).await;
        assert_shows_own(&place, &seen, reader, 3, &[0, 1, 3]);
        assert!(
            seen.contains(&reader.channels[1]) && seen.contains("waiting_channels"),
            "{place} shows its waits"
        );

        let place = format!("{}'s web pages", reader.swarm.as_str());
        let cookie = client.sign_in(Bearers::operator(&reader.swarm)).await;
        let seen = web_reads(&client, &cookie, reader, foreign).await;
        assert_shows_own(&place, &seen, reader, 3, &[0, 1, 2, 3]);

        let place = format!("{}'s /mcp-admin", reader.swarm.as_str());
        let seen = admin_reads(&client, reader, foreign).await;
        assert_shows_own(&place, &seen, reader, 3, &[0, 1, 2, 3]);
    }

    // The waits end with a message in their own channel, and carry only it.
    for (s, wait) in seeded.iter().zip(waits) {
        let sent = ok(
            &client,
            s.bearer(),
            "send",
            json!({ "token": s.owner().token, "channel_id": s.channels[1], "body": body(s.tag, "wake"), "format": "json" }),
        )
        .await;
        let answer = wait.await.expect("the wait");
        assert!(!answer.is_error, "{}", answer.text);
        assert!(
            answer.text.contains(&body(s.tag, "wake")),
            "{}",
            answer.text
        );
        let foreign = if s.swarm == main() {
            &seeded[1]
        } else {
            &seeded[0]
        };
        foreign.assert_absent("a wait", &answer.raw, &[]);
        assert!(sent["message_id"].is_i64());
    }

    // The live feeds: everything each swarm did, and nothing of the other.
    for (s, feed) in seeded.iter().zip(feeds.iter_mut()) {
        ok(
            &client,
            s.bearer(),
            "send",
            json!({ "token": s.owner().token, "channel_id": s.channels[0], "body": body(s.tag, "last"), "format": "json" }),
        )
        .await;
        let foreign = if s.swarm == main() {
            &seeded[1]
        } else {
            &seeded[0]
        };
        let place = format!("{}'s live feed", s.swarm.as_str());
        let events = feed
            .until(&body(s.tag, "last"), |e| {
                foreign.assert_absent(&place, &format!("{}: {}", e.name, e.data), &[])
            })
            .await;
        assert_feed(s, foreign, &events);
    }
}

/// Fails if `events`, `own`'s live feed, names `foreign`, or lacks any kind
/// of event `own` made.
fn assert_feed(own: &Seeded, foreign: &Seeded, events: &[Event]) {
    let place = format!("{}'s live feed", own.swarm.as_str());
    let names: BTreeSet<&str> = events.iter().map(|e| e.name.as_str()).collect();
    for name in [
        "message",
        "agent",
        "channel",
        "joined",
        "left",
        "channel_hidden",
        "agent_hidden",
    ] {
        assert!(
            names.contains(name),
            "{place} has no {name} event: {names:?}"
        );
    }
    assert!(!names.contains("lagged"), "{place} lagged");
    let all: String = events
        .iter()
        .map(|e| format!("{}: {}\n", e.name, e.data))
        .collect();
    foreign.assert_absent(&place, &all, &[]);
    assert_shows_own(&place, &all, own, 3, &[0, 1, 2, 3]);
}

/// A new database in memory, with the core's tables.
fn memory() -> kkt_store::Db {
    kkt_store::Db::new(
        kkt_db_sqlite::MemoryPlace::new()
            .expect("an in-memory database")
            .open(&[kkt_store::schema()])
            .expect("the core's tables"),
    )
}

fn system_clock() -> std::sync::Arc<dyn kkt_clock::Clock> {
    std::sync::Arc::new(kkt_clock_system::SystemClock)
}

fn os_entropy() -> std::sync::Arc<dyn kkt_entropy::Entropy> {
    std::sync::Arc::new(kkt_entropy_os::OsEntropy)
}

/// A clock that stays at one moment, the same in every run.
fn fixed_clock() -> std::sync::Arc<dyn kkt_clock::Clock> {
    std::sync::Arc::new(kkt_clock::testing::FixedClock::new(
        kkt_clock::Time::from_parts(2026, 11, 10, 12, 0, 0).expect("a time"),
    ))
}

/// Everything `reader`'s agents, operator and admin read,
/// as one transcript, with `foreign`'s names asked for. `cookie` is the
/// reader's operator's session.
async fn transcript(client: &Client, cookie: &str, reader: &Seeded, foreign: &Seeded) -> String {
    let mut all = agent_reads(client, reader, foreign).await;
    all += &web_reads(client, cookie, reader, foreign).await;
    all += &admin_reads(client, reader, foreign).await;
    all
}

/// A product with a fixed clock and seeded entropy, in which `reader` is
/// seeded first, then, if `foreign` is given, the other swarm. Returns the
/// reader's transcript and what the other swarm was seeded with.
async fn run(
    reader: (SwarmId, &'static str),
    foreign: Option<(SwarmId, &'static str)>,
    probes: Option<&Seeded>,
) -> (String, Option<Seeded>) {
    let product = Product::start(
        memory(),
        fixed_clock(),
        std::sync::Arc::new(kkt_entropy::testing::SeededEntropy::new(42)),
        kkt_config::Limits::DEFAULT,
    )
    .await;
    let client = Client::new(&product);
    let own = seed(&client, reader.0, reader.1).await;
    let cookie = client.sign_in(Bearers::operator(&own.swarm)).await;
    let theirs = match foreign {
        Some((swarm, tag)) => Some(seed(&client, swarm, tag).await),
        None => None,
    };
    let asked = theirs.as_ref().or(probes).expect("names to ask for");
    let seen = transcript(&client, &cookie, &own, asked).await;
    (seen, theirs)
}

/// Fails, showing where, unless `alone` and `beside` are the same.
fn assert_same(reader: &str, alone: &str, beside: &str) {
    if alone == beside {
        return;
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (alone.lines().collect(), beside.lines().collect());
    let at = a
        .iter()
        .zip(&b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    panic!(
        "{reader} reads differently beside the other swarm, at line {at}:\n\
         alone:  {}\nbeside: {}",
        a.get(at).unwrap_or(&"(end)"),
        b.get(at).unwrap_or(&"(end)"),
    );
}

/// The markers above see foreign text; this sees everything else. Each
/// swarm's reads are made twice, by the same steps over a fixed clock and
/// seeded entropy: once with the other swarm empty, once after the other
/// swarm was seeded. Every answer and page must be the same, byte for
/// byte, so no count, order, page boundary or ID that the other swarm
/// could change reaches the reader. The reader's swarm is seeded first in
/// both runs, so its message IDs are the same in both, and it sends nothing
/// after the other swarm is seeded: the plan's known crossings (a taken
/// handle, gaps in message IDs) are not what is compared here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_swarm_reads_the_same_whether_or_not_the_other_is_there() {
    for (reader, foreign) in [
        ((main(), "mainx"), (other(), "otherx")),
        ((other(), "otherx"), (main(), "mainx")),
    ] {
        let name = reader.0.as_str().to_string();
        let (beside, theirs) = run(reader.clone(), Some(foreign), None).await;
        let (alone, _) = run(reader, None, theirs.as_ref()).await;
        assert_same(&name, &alone, &beside);
    }
}
