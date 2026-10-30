//! Page tests that every source must pass, run against the real one (over
//! the rules and an in-memory database) and the preview; plus tests of the
//! preview's simulated world and feed.

use super::*;
use crate::admission::{Admission, Sessions, WebAdmission};
use crate::clock::testing::FixedClock;
use crate::config::{Limits, PreviewSettings};
use crate::events::Events;
use crate::names::testing::SeqEntropy;
use crate::names::{self, SeededEntropy};
use crate::rules::{Deps, NewChannel, RegisterLimiter, Rules, SendLimiter};
use crate::shutdown::Shutdown;
use crate::store::{Db, NewServerRunRecord};
use axum::body::Body;
use axum::http::Request as HttpRequest;
use chrono::TimeZone;
use futures::StreamExt;
use preview::Happening;
use source::ChannelHidden;
use tower::ServiceExt;

fn start_time() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc
        .with_ymd_and_hms(2026, 10, 25, 12, 0, 0)
        .single()
        .expect("valid time")
}

fn open_gate() -> Arc<dyn Gate> {
    let clock = Arc::new(FixedClock::new(start_time()));
    let sessions = Sessions::new(
        clock,
        Arc::new(SeqEntropy::new(9)),
        Limits::DEFAULT.web_session_lifetime,
    );
    Arc::new(WebAdmission::new(Arc::new(Admission::Open), sessions))
}

fn site(source: Arc<dyn Source>) -> Router {
    router(
        source,
        AllowedHosts::new(&[]).expect("loopback hosts"),
        open_gate(),
    )
}

async fn get(site: &Router, path: &str) -> (StatusCode, String) {
    let request = HttpRequest::builder()
        .uri(path)
        .header("Host", "localhost")
        .body(Body::empty())
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A form post to `path`, sent from this site.
async fn post(site: &Router, path: &str, body: &str) -> (StatusCode, Option<String>) {
    let request = HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Origin", "http://localhost")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|l| l.to_str().ok())
        .map(str::to_string);
    (response.status(), location)
}

/// The value of the first `form_token` field in `page`.
fn form_token_in(page: &str) -> String {
    let (_, rest) = page
        .split_once(r#"name="form_token" value=""#)
        .expect("a form token");
    rest.split_once('"').expect("closing quote").0.to_string()
}

/// What every source must do for a change: refuse it without the page's
/// form token; with it, make it, and show the operator's message saying so.
async fn changes_work(source: Arc<dyn Source>) {
    let overview = source.overview().await.expect("overview");
    let id = overview
        .channels
        .first()
        .expect("a channel")
        .channel
        .id
        .clone();
    let site = site(source.clone());
    let path = format!("/channels/{id}/purpose");
    let (_, page) = get(&site, &format!("/channels/{id}")).await;
    assert!(page.contains(&format!(r#"action="/channels/{id}/purpose""#)));
    let token = form_token_in(&page);

    let (status, _) = post(&site, &path, "purpose=forged").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = post(&site, &path, "purpose=forged&form_token=guess").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let view = source.channel(id.clone(), None).await.expect("view");
    assert_ne!(view.channel.purpose.as_deref(), Some("forged"));

    let (status, location) = post(
        &site,
        &path,
        &format!("purpose=Release+%26+prep&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, Some(format!("/channels/{id}")));
    let view = source.channel(id.clone(), None).await.expect("view");
    assert_eq!(view.channel.purpose.as_deref(), Some("Release & prep"));
    let said = view.messages.last().expect("the operator's message");
    assert!(said.from_operator);
    assert_eq!(said.agent, crate::names::OPERATOR);
    assert_eq!(
        said.body,
        "The operator changed the purpose to: Release & prep"
    );
    // The page shows the operator by name, with no link to an agent page.
    let (_, page) = get(&site, &format!("/channels/{id}")).await;
    assert!(page.contains(r#"<span class="who">operator</span>"#));
    assert!(!page.contains("/agents/operator"));
    // So does the front page, where the operator spoke last in the channel.
    let overview = source.overview().await.expect("overview");
    let summary = overview
        .channels
        .iter()
        .find(|c| c.channel.id == id)
        .expect("the channel");
    assert!(summary.last_from_operator);
    let (_, page) = get(&site, "/").await;
    assert!(!page.contains("/agents/operator"));

    let (status, _) = post(
        &site,
        "/channels/ch_missing0/purpose",
        &format!("purpose=x&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// What every source must do for an agent's handoff: show its note, and give
/// a new recovery code only to a form from the page, on a page the browser
/// does not keep.
async fn handoff_works(source: Arc<dyn Source>) -> (String, String) {
    let overview = source.overview().await.expect("overview");
    let mut noted = None;
    for a in &overview.agents {
        let view = source.agent(a.agent.name.clone()).await.expect("view");
        if !view.handoff_note.is_empty() {
            noted = Some(view);
            break;
        }
    }
    let view = noted.expect("an agent with a handoff note");
    let name = view.agent.name.clone();
    let site = site(source.clone());
    let (_, page) = get(&site, &format!("/agents/{name}")).await;
    let first_line = view.handoff_note.lines().next().expect("a line");
    assert!(page.contains(first_line), "the page shows the note");
    let path = format!("/agents/{name}/recovery-code");
    assert!(page.contains(&format!(r#"action="{path}""#)));
    let token = form_token_in(&page);

    let (status, _) = post(&site, &path, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let request = HttpRequest::builder()
        .method("POST")
        .uri(&path)
        .header("Host", "localhost")
        .header("Origin", "http://localhost")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("form_token={token}")))
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.as_bytes()),
        Some(&b"no-store"[..])
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let page = String::from_utf8_lossy(&bytes).into_owned();
    let (_, rest) = page
        .split_once(r#"<p class="mono" style="overflow-wrap: anywhere">"#)
        .expect("the code");
    let code = rest.split_once('<').expect("end of code").0.to_string();
    assert!(code.starts_with("arc_"), "{code}");

    let (status, _) = post(
        &site,
        "/agents/nobody-here/recovery-code",
        &format!("form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    (name, code)
}

#[tokio::test]
async fn handoff_works_through_the_rules() {
    let (rules, source) = rules_world().await;
    let (name, code) = handoff_works(source).await;
    // The code shown on the page is the one that now takes over the identity.
    let taken = rules.reclaim(name, code).await.expect("reclaim");
    rules.authenticate(Some(&taken.token)).await.expect("auth");
}

#[tokio::test]
async fn handoff_works_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    handoff_works(preview(1, clock, &Shutdown::new())).await;
}

/// What every source must do for "Show previous": from the first page of the
/// busiest channel, the earlier pages read back every message once, oldest
/// first, a page at a time, until there are no more.
async fn earlier_pages_work(source: Arc<dyn Source>) -> usize {
    let overview = source.overview().await.expect("overview");
    let busiest = overview
        .channels
        .iter()
        .max_by_key(|c| c.message_count)
        .expect("a channel");
    let id = busiest.channel.id.clone();
    let site = site(source.clone());
    let first = source.channel(id.clone(), None).await.expect("first page");
    let (_, page) = get(&site, &format!("/channels/{id}")).await;
    let before = first.older_before_id.expect("earlier messages to load");
    assert!(page.contains(&format!(r#"data-before="{before}""#)));
    assert!(page.contains(&format!(
        "Show previous {} messages",
        source.page_messages()
    )));

    let mut ids: Vec<i64> = first.messages.iter().map(|m| m.id).collect();
    let mut next = Some(before);
    let mut pages = 1;
    while let Some(before) = next {
        let (status, body) = get(
            &site,
            &format!("/channels/{id}/messages?before_id={before}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        let page: Vec<i64> = json["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .map(|m| m["id"].as_i64().expect("id"))
            .collect();
        assert!(!page.is_empty() && page.len() <= source.page_messages());
        assert!(page.iter().all(|&m| m < before), "only earlier messages");
        ids.splice(0..0, page);
        next = json["older_before_id"].as_i64();
        pages += 1;
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids, sorted, "oldest first, each once");
    assert_eq!(ids.len() as i64, busiest.message_count, "every message");

    let (status, _) = get(&site, "/channels/ch_missing0/messages?before_id=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&site, &format!("/channels/{id}/messages")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "before_id is required");
    pages
}

#[tokio::test]
async fn earlier_pages_work_through_the_rules() {
    let limits = Limits {
        web_page_messages: 2,
        ..Limits::DEFAULT
    };
    let (_, source) = rules_world_with(limits).await;
    assert!(earlier_pages_work(source).await >= 2);
}

#[tokio::test]
async fn the_preview_has_a_channel_long_enough_to_page_back_through() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let source = preview(1, clock, &Shutdown::new());
    assert_eq!(source.page_messages(), Limits::DEFAULT.web_page_messages);
    assert!(earlier_pages_work(source).await >= 3, "several pages back");
}

#[tokio::test]
async fn changes_work_through_the_rules() {
    changes_work(rules_source().await).await;
}

#[tokio::test]
async fn changes_work_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    changes_work(preview(1, clock, &Shutdown::new())).await;
}

/// What every source must do: serve each page, page back through a
/// channel, and answer 404 for what does not exist.
async fn pages_work(source: Arc<dyn Source>) {
    let overview = source.overview().await.expect("overview");
    let channel = overview
        .channels
        .first()
        .expect("a channel")
        .channel
        .clone();
    let agent = overview.agents.first().expect("an agent").agent.clone();
    let busy = overview
        .channels
        .iter()
        .find(|c| c.message_count > 0)
        .expect("a channel with messages")
        .clone();
    let site = site(source.clone());

    let (status, page) = get(&site, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains(&format!("#{}", channel.name)),
        "front page lacks the channel"
    );
    assert!(page.contains(&agent.name), "front page lacks the agent");
    assert!(page.contains(&format!("data-now=\"{}\"", overview.now)));
    assert!(
        page.contains(&format!("data-channel-id=\"{}\"", busy.channel.id)),
        "front page lacks a channel row"
    );
    assert!(
        !page.contains("<li class=\"msg\""),
        "the front page lists no messages"
    );

    let (status, page) = get(&site, &format!("/channels/{}", channel.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&format!("#{}", channel.name)));
    assert!(page.contains("data-follow"), "the newest page is live");
    assert!(page.contains(&format!("data-max=\"{}\"", source.page_messages())));

    let view = source
        .channel(busy.channel.id.clone(), None)
        .await
        .expect("channel view");
    let newest = view.messages.last().expect("a message").id;
    let (status, page) = get(
        &site,
        &format!("/channels/{}?before_id={newest}", busy.channel.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("Latest messages"),
        "a page of older messages links back"
    );

    let (status, page) = get(&site, &format!("/agents/{}", agent.name)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&agent.name));

    // An agent's description shows on the front page, its own page, and as
    // the title of its chip on the pages of channels it is in.
    let described = overview
        .agents
        .iter()
        .find_map(|a| Some((a, a.agent.description.clone()?)))
        .expect("an agent with a description");
    let (a, text) = described;
    let (_, page) = get(&site, "/").await;
    assert!(page.contains(&text), "front page lacks a description");
    let (_, page) = get(&site, &format!("/agents/{}", a.agent.name)).await;
    assert!(page.contains(&text), "agent page lacks its description");
    if let Some(c) = a.channels.first() {
        let (_, page) = get(&site, &format!("/channels/{}", c.id)).await;
        assert!(
            page.contains(&format!("title=\"{text}\"")),
            "channel page lacks a member's description"
        );
    }

    let (status, _) = get(&site, "/channels/ch_nothere0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&site, "/agents/nobody-here").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Rules over an in-memory database with two agents, an open and a closed
/// channel, and a few messages.
async fn rules_source() -> Arc<dyn Source> {
    rules_world().await.1
}

/// The rules behind `rules_source`, too, for checking what a change did.
async fn rules_world() -> (Rules, Arc<dyn Source>) {
    rules_world_with(Limits::DEFAULT).await
}

async fn rules_world_with(limits: Limits) -> (Rules, Arc<dyn Source>) {
    let (rules, source, _) = rules_world_and_clock(limits).await;
    (rules, source)
}

/// `rules_world_with`, and the clock the rules read.
async fn rules_world_and_clock(limits: Limits) -> (Rules, Arc<dyn Source>, Arc<FixedClock>) {
    let clock = Arc::new(FixedClock::new(start_time()));
    let db = Db::open_in_memory().expect("db");
    // An earlier run, from before an upgrade, with values as long as real
    // ones, then this run.
    db.write(|w| {
        w.insert_server_run(&NewServerRunRecord {
            started_at: "2026-10-20T09:30:00Z".into(),
            version: "0.5.0".into(),
            git_commit: "4f1c2a9e7b3d5f60a8c1e2d3b4a5968778695a4b-dirty".into(),
            settings: "max_response_bytes=65536\nallowed_origin=https://a-long-machine-name.tail1234abcd.ts.net".into(),
            instructions_sha256: "ab".repeat(32),
        })
    })
    .await
    .expect("record an earlier run");
    let run_id = db
        .write(|w| {
            w.insert_server_run(&NewServerRunRecord {
                started_at: "2026-10-25T12:00:00Z".into(),
                version: "test".into(),
                git_commit: "test".into(),
                settings: "test".into(),
                instructions_sha256: "test".into(),
            })
        })
        .await
        .expect("record run");
    let rules = Rules::new(Deps {
        waiting: crate::rules::Waiting::default(),
        db,
        events: Events::new(),
        clock: clock.clone(),
        entropy: Arc::new(SeqEntropy::new(4)),
        sends: SendLimiter::new(limits.sends_per_minute),
        registrations: RegisterLimiter::new(limits.registrations_per_minute),
        run_id,
        limits,
    });
    let mut agents = Vec::new();
    for _ in 0..2 {
        let reg = rules.register().await.expect("register");
        agents.push(rules.authenticate(Some(&reg.token)).await.expect("auth"));
    }
    rules
        .set_description(&agents[0], "Codex, reviews only".into())
        .await
        .expect("describe");
    for (name, open) in [("open-room", true), ("closed-room", false)] {
        let channel = rules
            .create_channel(
                &agents[0],
                NewChannel {
                    name: name.into(),
                    open,
                    purpose: None,
                    members: vec![agents[1].name.clone()],
                },
            )
            .await
            .expect("channel");
        rules
            .set_note(
                &agents[1],
                channel.id.clone(),
                "First line\n<script>alert(1)</script>".into(),
            )
            .await
            .expect("note");
        for i in 0..3 {
            rules
                .send(&agents[i % 2], channel.id.clone(), format!("message {i}"))
                .await
                .expect("send");
        }
    }
    rules
        .set_handoff_note(
            &agents[0],
            "Next: the parser.\nOwner approves pushes.".into(),
        )
        .await
        .expect("handoff note");
    (rules.clone(), Arc::new(RulesSource::new(rules)), clock)
}

fn preview(seed: u64, clock: Arc<FixedClock>, shutdown: &Shutdown) -> Arc<Preview> {
    Arc::new(Preview::new(
        clock,
        Arc::new(SeededEntropy::new(seed)),
        PreviewSettings::DEFAULT,
        shutdown,
    ))
}

#[tokio::test]
async fn pages_work_from_the_rules() {
    pages_work(rules_source().await).await;
}

#[tokio::test]
async fn pages_work_from_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    pages_work(preview(1, clock, &Shutdown::new())).await;
}

/// What every source must do with hidden channels: leave them off the front
/// page unless asked, list them marked when asked, and show a hidden
/// channel's page to read but not to change.
async fn hidden_channels_show_only_on_request(source: Arc<dyn Source>) {
    let site = site(source.clone());
    let overview = source.overview().await.expect("overview");
    let id = overview.channels[0].channel.id.clone();
    let still_listed = overview.channels[1].channel.id.clone();
    source
        .change_channel(id.clone(), ChannelAction::Hide)
        .await
        .expect("hide");
    let hidden = source.hidden_channels().await.expect("hidden");
    assert!(hidden.iter().any(|c| c.channel.id == id));
    assert!(hidden.iter().all(|c| c.hidden));
    assert!(
        source
            .overview()
            .await
            .expect("overview")
            .channels
            .iter()
            .all(|c| !c.hidden && !hidden.iter().any(|h| h.channel.id == c.channel.id))
    );

    let row = |id: &str| format!("<tr data-channel-id=\"{id}\"");
    let (status, page) = get(&site, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<a href="/?hidden=1" data-hidden-toggle>Show hidden channels</a>"#));
    assert!(page.contains(&row(&still_listed)));
    for c in &hidden {
        assert!(!page.contains(&row(&c.channel.id)), "{}", c.channel.name);
    }
    assert!(!page.contains("data-hidden-channel"));

    let (status, page) = get(&site, "/?hidden=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<a href="/" data-hidden-toggle>Leave out hidden channels</a>"#));
    assert!(page.contains(&row(&still_listed)));
    for c in &hidden {
        assert!(
            page.contains(&format!("{} data-hidden-channel", row(&c.channel.id))),
            "{}",
            c.channel.name
        );
    }
    assert_eq!(page.matches("data-hidden-channel").count(), hidden.len());
    assert!(page.contains(&format!("and {} hidden", hidden.len())));
    // Any other value is the default view.
    let (_, page) = get(&site, "/?hidden=yes").await;
    assert!(!page.contains("data-hidden-channel"));

    let (status, page) = get(&site, &format!("/channels/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-hidden-notice"));
    assert!(page.contains(r#"<span class="badge gone">hidden</span>"#));
    assert!(page.contains("The operator hid the channel. Its messages are kept."));
    assert!(!page.contains(&format!("/channels/{id}/purpose")));
    assert!(!page.contains(&format!("/channels/{id}/hide")));
    assert!(!page.contains(" data-live data-follow"));
    let (_, page) = get(&site, &format!("/channels/{still_listed}")).await;
    assert!(!page.contains("data-hidden-notice"));
    assert!(page.contains(" data-live data-follow"));

    // A change sent anyway is refused, as for a channel that does not exist.
    let (_, page) = get(&site, "/").await;
    let token = form_token_in(&page);
    let (status, _) = post(
        &site,
        &format!("/channels/{id}/purpose"),
        &format!("purpose=back&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hidden_channels_show_only_on_request_from_the_rules() {
    hidden_channels_show_only_on_request(rules_source().await).await;
}

#[tokio::test]
async fn hidden_channels_show_only_on_request_from_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(1, clock, &Shutdown::new());
    // The preview starts with one hidden channel, with messages.
    let hidden = p.hidden_channels().await.expect("hidden");
    assert_eq!(hidden.len(), 1);
    assert!(hidden[0].message_count > 1);
    assert!(hidden[0].last_from_operator);
    hidden_channels_show_only_on_request(p).await;
}

#[tokio::test]
async fn the_show_older_link_appears_only_when_older_messages_exist() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(1, clock, &Shutdown::new());
    let site = site(p.clone());
    let overview = p.overview().await.expect("overview");
    let mut seen_paged = false;
    let mut seen_single = false;
    for c in overview.channels.iter().map(|s| &s.channel) {
        let view = p.channel(c.id.clone(), None).await.expect("view");
        let (status, page) = get(&site, &format!("/channels/{}", c.id)).await;
        assert_eq!(status, StatusCode::OK);
        match view.older_before_id {
            Some(before) => {
                seen_paged = true;
                assert!(
                    page.contains(&format!(
                        "?before_id={before}\" data-before=\"{before}\">Show previous"
                    )),
                    "{}",
                    c.name
                );
                assert!(!page.contains("id=\"older\" hidden"), "{}", c.name);
            }
            None => {
                seen_single = true;
                assert!(page.contains("id=\"older\" hidden"), "{}", c.name);
            }
        }
    }
    assert!(seen_paged && seen_single);
}

/// The front page's summary must agree with what the channel and agent
/// pages show, for any source.
async fn summaries_agree_with_the_pages(source: Arc<dyn Source>) {
    let overview = source.overview().await.expect("overview");
    for c in &overview.channels {
        let view = source
            .channel(c.channel.id.clone(), None)
            .await
            .expect("view");
        let newest = view.messages.last();
        assert_eq!(c.last_message_at, newest.map(|m| m.created_at.clone()));
        assert_eq!(c.last_speaker, newest.map(|m| m.agent.clone()));
        assert!(c.message_count >= view.messages.len() as i64);
        assert_eq!(c.message_count == 0, view.messages.is_empty());
        assert_eq!(c.channel.member_count, view.members.len() as i64);
        let starts: Vec<&str> = c.recent_activity.iter().map(|a| a.start.as_str()).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(sorted, starts, "recent counts are oldest first");
        for a in &c.recent_activity {
            assert_eq!(
                crate::clock::floor_text(&a.start, crate::config::WEB_ACTIVITY_BUCKET).as_deref(),
                Some(a.start.as_str()),
                "stretches start on a stretch boundary"
            );
        }
        // With every message on one page, the recent count is exact.
        if view.older_before_id.is_none() {
            let since = crate::clock::to_text(
                crate::clock::from_text(&overview.now).expect("now")
                    - chrono::Duration::from_std(crate::config::WEB_ACTIVITY_WINDOW)
                        .expect("window"),
            );
            let in_window = view
                .messages
                .iter()
                .filter(|m| m.created_at >= since)
                .count();
            assert_eq!(c.recent_count(), in_window as i64, "{}", c.channel.name);
        }
    }
    for a in &overview.agents {
        let view = source.agent(a.agent.name.clone()).await.expect("view");
        let mut theirs: Vec<&str> = view.channels.iter().map(|c| c.id.as_str()).collect();
        let mut listed: Vec<&str> = a.channels.iter().map(|c| c.id.as_str()).collect();
        theirs.sort();
        listed.sort();
        assert_eq!(listed, theirs, "{}", a.agent.name);
    }
    // The front page keeps channels and agents in creation order.
    let (_, page) = get(&site(source), "/").await;
    let rows: Vec<&str> = page
        .split("<tr data-channel-id=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let mut channels = overview.channels.clone();
    channels.sort_by(|a, b| {
        (&a.channel.created_at, &a.channel.id).cmp(&(&b.channel.created_at, &b.channel.id))
    });
    let expected: Vec<&str> = channels.iter().map(|c| c.channel.id.as_str()).collect();
    assert_eq!(rows, expected);
    let agent_rows: Vec<&str> = page
        .split("<tr data-agent=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let mut agents = overview.agents.clone();
    agents.sort_by(|a, b| {
        (&a.agent.created_at, &a.agent.name).cmp(&(&b.agent.created_at, &b.agent.name))
    });
    let expected: Vec<&str> = agents.iter().map(|a| a.agent.name.as_str()).collect();
    assert_eq!(agent_rows, expected);
}

#[tokio::test]
async fn summaries_agree_with_the_pages_from_the_rules() {
    summaries_agree_with_the_pages(rules_source().await).await;
}

#[tokio::test]
async fn summaries_agree_with_the_pages_from_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(4, clock, &Shutdown::new());
    for _ in 0..30 {
        p.step();
    }
    summaries_agree_with_the_pages(p).await;
}

#[tokio::test]
async fn the_preview_counts_recent_messages_in_the_window() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(1, clock.clone(), &Shutdown::new());
    // The starting history ends now, so some of it is recent.
    let before: i64 = p
        .overview()
        .await
        .expect("overview")
        .channels
        .iter()
        .map(|c| c.recent_count())
        .sum();
    assert!(before > 0);
    // Ten minutes later, with nothing new, nothing is recent.
    clock.advance(chrono::Duration::minutes(10));
    let after = p.overview().await.expect("overview");
    assert!(after.channels.iter().all(|c| c.recent_activity.is_empty()));
    assert_eq!(
        after.now,
        crate::clock::to_text(start_time() + chrono::Duration::minutes(10))
    );
    // A new message is recent again.
    p.step();
    let again: i64 = p
        .overview()
        .await
        .expect("overview")
        .channels
        .iter()
        .map(|c| c.recent_count())
        .sum();
    assert!(again >= 1);
}

#[tokio::test]
async fn the_preview_world_repeats_from_its_seed() {
    let shutdown = Shutdown::new();
    let world = |seed| {
        let clock = Arc::new(FixedClock::new(start_time()));
        let p = preview(seed, clock, &shutdown);
        for _ in 0..50 {
            p.step();
        }
        p
    };
    let a = world(5).overview().await.expect("a");
    let b = world(5).overview().await.expect("b");
    let c = world(6).overview().await.expect("c");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[tokio::test]
async fn the_preview_world_has_what_the_page_must_show() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(1, clock, &Shutdown::new());
    let overview = p.overview().await.expect("overview");
    assert!(overview.channels.iter().any(|c| c.channel.open));
    assert!(overview.channels.iter().any(|c| !c.channel.open));
    let mut empty = 0;
    let mut paged = 0;
    let mut long = false;
    for c in overview.channels.iter().map(|s| &s.channel) {
        let view = p.channel(c.id.clone(), None).await.expect("view");
        if view.messages.is_empty() {
            empty += 1;
        }
        if let Some(before) = view.older_before_id {
            paged += 1;
            assert_eq!(view.messages.len(), p.page_messages());
            let older = p.channel(c.id.clone(), Some(before)).await.expect("older");
            assert!(older.messages.iter().all(|m| m.id < before));
        }
        long |= view.messages.iter().any(|m| m.body.len() > 500);
    }
    assert_eq!(empty, 1, "one channel starts empty");
    assert!(paged >= 1, "a channel has more than one page");
    assert!(long, "some messages are very long");
}

#[tokio::test]
async fn the_preview_feed_has_every_kind_of_happening_and_publishes_it() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(3, clock, &Shutdown::new());
    let mut live = p.live();
    let before = p.overview().await.expect("before");
    let (mut bursts, mut agents, mut channels, mut messages) = (0, 0, 0, 0);
    let (mut leaves, mut unregistered, mut hidden) = (0, 0, 0);
    for _ in 0..300 {
        match p.step() {
            Happening::Message => messages += 1,
            Happening::Burst(n) => {
                bursts += 1;
                messages += n;
            }
            // A new agent says hello; a new channel gets a first message.
            Happening::NewAgent(_) => {
                agents += 1;
                messages += 1;
            }
            Happening::NewChannel(_) => {
                channels += 1;
                messages += 1;
            }
            Happening::Left(_) => leaves += 1,
            // Each says so in its channels.
            Happening::Unregistered(_) => unregistered += 1,
            Happening::ChannelHidden(_) => {
                hidden += 1;
                messages += 1;
            }
        }
    }
    assert!(bursts > 0 && agents > 0 && channels > 0);
    assert!(leaves > 0 && unregistered > 0 && hidden > 0);
    let after = p.overview().await.expect("after");
    assert_eq!(
        after.agents.len(),
        before.agents.len() + agents - unregistered
    );
    assert_eq!(
        after.channels.len(),
        before.channels.len() + channels - hidden
    );

    // Everything was published, in order. (The buffer holds it all.)
    let (mut heard_messages, mut heard_agents, mut heard_channels, mut heard_joins) = (0, 0, 0, 0);
    let (mut heard_leaves, mut heard_unregistered, mut heard_hidden) = (0, 0, 0);
    let mut last = 0;
    let mut joined_new_agent = true;
    let mut newest_agent = String::new();
    while let Ok(event) =
        tokio::time::timeout(std::time::Duration::from_millis(50), live.next()).await
    {
        match event {
            Some(LiveEvent::Message(m)) => {
                assert!(m.id > last);
                last = m.id;
                heard_messages += 1;
            }
            Some(LiveEvent::Agent(a)) => {
                newest_agent = a.name;
                heard_agents += 1;
            }
            Some(LiveEvent::Channel(_)) => heard_channels += 1,
            Some(LiveEvent::Joined(j)) => {
                joined_new_agent &= j.agent == newest_agent;
                heard_joins += 1;
            }
            // In the feed, agents leave, unregister and hide channels themselves.
            Some(LiveEvent::Left(l)) => {
                assert!(l.by == l.agent && !l.by_operator, "{l:?}");
                heard_leaves += 1;
            }
            Some(LiveEvent::AgentHidden(h)) => {
                assert!(h.by == h.agent && !h.by_operator, "{h:?}");
                heard_unregistered += 1;
            }
            Some(LiveEvent::ChannelHidden(h)) => {
                assert!(!h.by_operator, "{h:?}");
                heard_hidden += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(heard_messages >= messages);
    assert_eq!(heard_agents, agents);
    assert_eq!(heard_channels, channels);
    assert_eq!(heard_leaves, leaves);
    assert_eq!(heard_unregistered, unregistered);
    assert_eq!(heard_hidden, hidden);
    assert!(
        heard_joins >= agents,
        "each new agent joins at least one channel"
    );
    assert!(joined_new_agent, "joins follow the agent that joined");
}

#[tokio::test]
async fn the_preview_runs_by_the_clock_and_stops_with_the_server() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let shutdown = Shutdown::new();
    let p = preview(2, clock.clone(), &shutdown);
    let mut live = p.live();
    let runner = {
        let p = p.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move { p.run(shutdown).await })
    };
    // At 30 steps a minute, one step every two seconds.
    tokio::task::yield_now().await;
    clock.advance(chrono::Duration::seconds(2));
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), live.next())
        .await
        .expect("a step after two seconds");
    assert!(
        matches!(first, Some(ref e) if !matches!(e, LiveEvent::Missed(_))),
        "{first:?}"
    );
    shutdown.stop();
    tokio::time::timeout(std::time::Duration::from_secs(5), runner)
        .await
        .expect("the preview stops")
        .expect("no panic");
    assert!(
        live.next().await.is_none(),
        "live feeds end with the server"
    );
}

#[tokio::test]
async fn channel_page_shows_the_pinned_note_and_editor_as_escaped_text() {
    let source = rules_source().await;
    let overview = source.overview().await.unwrap();
    let ch = &overview.channels[0].channel;
    let (status, page) = get(&site(source.clone()), &format!("/channels/{}", ch.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Pinned note"));
    assert!(
        page.contains("First line\n&#60;script&#62;alert(1)&#60;/script&#62;"),
        "{page}"
    );
    assert!(!page.contains("<script>alert(1)</script>"));
    assert!(page.contains("Last edited by"));
    assert!(page.contains(ch.note_edited_by.as_deref().unwrap()));
    assert!(page.contains("2026-10-25"));
}

async fn administration_works(source: Arc<dyn Source>) {
    let overview = source.overview().await.unwrap();
    let id = overview.channels[0].channel.id.clone();
    let current = source.channel(id.clone(), None).await.unwrap();
    let name = current.members[0].name.clone();
    let count = current.members.len();
    let app = site(source.clone());
    let (_, page) = get(&app, &format!("/channels/{id}")).await;
    let token = form_token_in(&page);
    let routes = [
        (
            format!("/channels/{id}/visibility"),
            "open=false".to_string(),
        ),
        (
            format!("/channels/{id}/members/add"),
            format!("name={name}"),
        ),
        (
            format!("/channels/{id}/members/remove"),
            format!("name={name}"),
        ),
        (format!("/channels/{id}/hide"), String::new()),
        (format!("/agents/{name}/hide"), String::new()),
        (format!("/channels/{id}/owner"), "owner=".to_string()),
    ];
    for (path, fields) in &routes {
        assert_eq!(post(&app, path, fields).await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            post(&app, path, &format!("{fields}&form_token=invalid"))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(get(&app, path).await.0, StatusCode::METHOD_NOT_ALLOWED);
        let request = HttpRequest::builder()
            .method("POST")
            .uri(path)
            .header("Host", "localhost")
            .header("Origin", "http://other.example")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("{fields}&form_token={token}")))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    // The forged requests made no changes.
    assert_eq!(
        source
            .channel(id.clone(), None)
            .await
            .unwrap()
            .members
            .len(),
        count
    );
    assert!(source.agent(name.clone()).await.is_ok());
    let open = !current.channel.open;
    assert_eq!(
        post(
            &app,
            &routes[0].0,
            &format!("open={open}&form_token={token}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        source.channel(id.clone(), None).await.unwrap().channel.open,
        open
    );
    assert_eq!(
        post(
            &app,
            &routes[2].0,
            &format!("name={name}&form_token={token}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        source
            .channel(id.clone(), None)
            .await
            .unwrap()
            .members
            .len(),
        count - 1
    );
    assert_eq!(
        post(
            &app,
            &routes[1].0,
            &format!("name={name}&form_token={token}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    let view = source.channel(id.clone(), None).await.unwrap();
    assert_eq!(view.members.len(), count);
    assert!(view.messages.last().unwrap().from_operator);
    assert!(view.messages.last().unwrap().body.contains("added"));
    assert_eq!(view.owner, current.owner);
    // The operator makes the agent the owner; an owner cannot be hidden.
    let set_owner = |owner: &str| format!("owner={owner}&form_token={token}");
    assert_eq!(
        post(&app, &routes[5].0, &set_owner("")).await.0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(source.channel(id.clone(), None).await.unwrap().owner, None);
    assert_eq!(
        post(&app, &routes[5].0, &set_owner(&name)).await.0,
        StatusCode::SEE_OTHER
    );
    let view = source.channel(id.clone(), None).await.unwrap();
    assert_eq!(view.owner.as_deref(), Some(name.as_str()));
    let said = view.messages.last().unwrap();
    assert!(said.from_operator);
    assert_eq!(
        said.body,
        format!("The operator made {name} the owner of the channel.")
    );
    let (_, page) = get(&app, &format!("/channels/{id}")).await;
    assert!(page.contains(&format!("data-owner=\"{name}\"")), "{page}");
    assert_eq!(
        post(&app, &routes[4].0, &format!("form_token={token}"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert!(source.agent(name.clone()).await.is_ok());
    // Once it owns no channel, it can be hidden.
    for c in source.overview().await.unwrap().channels {
        let owned = source
            .channel(c.channel.id.clone(), None)
            .await
            .unwrap()
            .owner;
        if owned.as_deref() == Some(name.as_str()) {
            let path = format!("/channels/{}/owner", c.channel.id);
            assert_eq!(
                post(&app, &path, &set_owner("")).await.0,
                StatusCode::SEE_OTHER
            );
        }
    }
    let view = source.channel(id.clone(), None).await.unwrap();
    assert_eq!(view.owner, None);
    assert_eq!(
        view.messages.last().unwrap().body,
        format!(
            "The operator removed {name} as the owner of the channel. \
             The channel no longer has an owner."
        )
    );
    assert_eq!(
        post(&app, &routes[4].0, &format!("form_token={token}"))
            .await
            .0,
        StatusCode::SEE_OTHER
    );
    assert!(matches!(
        source.agent(name.clone()).await,
        Err(SourceError::NotFound(_))
    ));
    let view = source.channel(id.clone(), None).await.unwrap();
    assert!(!view.members.iter().any(|m| m.name == name));
    assert!(view.messages.last().unwrap().from_operator);
    assert!(view.messages.last().unwrap().body.contains("hid"));
    assert_eq!(
        post(&app, &routes[3].0, &format!("form_token={token}")).await,
        (StatusCode::SEE_OTHER, Some("/".into()))
    );
    // The human can still read it, marked hidden.
    assert!(source.channel(id.clone(), None).await.unwrap().hidden);
    let overview = source.overview().await.unwrap();
    assert!(!overview.channels.iter().any(|c| c.channel.id == id));
    assert!(!overview.agents.iter().any(|a| a.agent.name == name));
}

#[tokio::test]
async fn administration_works_through_the_rules() {
    administration_works(rules_source().await).await;
}

#[tokio::test]
async fn administration_works_through_the_preview() {
    administration_works(preview(
        1,
        Arc::new(FixedClock::new(start_time())),
        &Shutdown::new(),
    ))
    .await;
}

/// The operator's removals and hides reach the live feed, saying the
/// operator did them, with the member counts afterwards.
async fn operator_changes_are_live(source: Arc<dyn Source>) {
    let overview = source.overview().await.unwrap();
    // Owners cannot be hidden, so the channels are left with none first.
    for c in &overview.channels {
        source
            .change_channel(c.channel.id.clone(), ChannelAction::SetOwner(None))
            .await
            .unwrap();
    }
    let channel = overview
        .channels
        .iter()
        .map(|c| &c.channel)
        .find(|c| c.member_count >= 2)
        .expect("a channel with two members")
        .clone();
    let view = source.channel(channel.id.clone(), None).await.unwrap();
    let (removed, hidden) = (view.members[0].name.clone(), view.members[1].name.clone());
    let hidden_in: Vec<String> = source
        .agent(hidden.clone())
        .await
        .unwrap()
        .channels
        .iter()
        .map(|c| c.id.clone())
        .collect();

    let mut live = source.live();
    source
        .change_channel(
            channel.id.clone(),
            ChannelAction::RemoveMember(removed.clone()),
        )
        .await
        .unwrap();
    source.hide_agent(hidden.clone()).await.unwrap();
    source
        .change_channel(channel.id.clone(), ChannelAction::Hide)
        .await
        .unwrap();

    let mut heard = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), live.next()).await
    {
        match event {
            LiveEvent::Message(_) => {}
            other => heard.push(other),
        }
    }
    let op = names::OPERATOR.to_string();
    assert_eq!(heard.len(), 3, "{heard:?}");
    assert_eq!(
        heard[0],
        LiveEvent::Left(Left {
            agent: removed,
            channel_id: channel.id.clone(),
            channel_name: channel.name.clone(),
            member_count: channel.member_count - 1,
            by: op.clone(),
            by_operator: true,
        })
    );
    let LiveEvent::AgentHidden(gone) = &heard[1] else {
        panic!("expected agent_hidden, got {:?}", heard[1]);
    };
    assert_eq!(
        (&gone.agent, &gone.by, gone.by_operator),
        (&hidden, &op, true)
    );
    let mut left: Vec<String> = gone.channels.iter().map(|c| c.channel_id.clone()).collect();
    left.sort();
    let mut expected = hidden_in;
    expected.sort();
    assert_eq!(left, expected);
    // Each count is one less than before, and the first channel had also
    // lost the removed agent.
    for c in &gone.channels {
        let before = overview
            .channels
            .iter()
            .find(|s| s.channel.id == c.channel_id)
            .expect("listed before")
            .channel
            .member_count;
        let also_removed = i64::from(c.channel_id == channel.id);
        assert_eq!(c.member_count, before - 1 - also_removed, "{c:?}");
    }
    assert_eq!(
        heard[2],
        LiveEvent::ChannelHidden(ChannelHidden {
            channel_id: channel.id,
            channel_name: channel.name,
            by: op,
            by_operator: true,
        })
    );
}

#[tokio::test]
async fn operator_changes_are_live_through_the_rules() {
    operator_changes_are_live(rules_source().await).await;
}

#[tokio::test]
async fn operator_changes_are_live_in_the_preview() {
    operator_changes_are_live(preview(
        1,
        Arc::new(FixedClock::new(start_time())),
        &Shutdown::new(),
    ))
    .await;
}

/// The next live event that is not a message.
async fn next_change(live: &mut futures::stream::BoxStream<'static, LiveEvent>) -> LiveEvent {
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), live.next())
            .await
            .expect("an event in time")
            .expect("the feed is open");
        if !matches!(event, LiveEvent::Message(_)) {
            return event;
        }
    }
}

/// The row of `page` that starts with `start`, up to its end.
fn row_of<'a>(page: &'a str, start: &str) -> Option<&'a str> {
    let (_, rest) = page.split_once(start)?;
    Some(rest.split_once("</tr>").map_or(rest, |(row, _)| row))
}

fn agent_row<'a>(page: &'a str, name: &str) -> Option<&'a str> {
    row_of(page, &format!("<tr data-agent=\"{name}\""))
}

fn channel_row<'a>(page: &'a str, id: &str) -> Option<&'a str> {
    row_of(page, &format!("<tr data-channel-id=\"{id}\""))
}

/// The member count a channel's dashboard row shows.
fn members_shown(page: &str, id: &str) -> i64 {
    let row = channel_row(page, id).expect("the channel's row");
    let (_, rest) = row
        .split_once(r#"<td class="num members">"#)
        .expect("a members cell");
    rest.split_once('<')
        .expect("the cell's end")
        .0
        .parse()
        .expect("a number")
}

/// Whether an agent's dashboard row lists the channel.
fn lists_channel(page: &str, agent: &str, id: &str) -> bool {
    agent_row(page, agent)
        .expect("the agent's row")
        .contains(&format!("data-channel-id=\"{id}\""))
}

/// Each way an agent leaves a channel, a channel is hidden, or an agent goes:
/// the live event says who did it and what remains, its toast says so in
/// words, and the dashboard drawn afresh agrees with what the page's script
/// does with the event (`dashboardLeft`, `dashboardChannelHidden`,
/// `dashboardAgentHidden` in `base.html`).
#[tokio::test]
async fn leaving_removal_and_hiding_reach_the_toasts_and_dashboard() {
    let (rules, source) = rules_world().await;
    let site = site(source.clone());
    let mut agents = Vec::new();
    for _ in 0..6 {
        let reg = rules.register().await.expect("register");
        agents.push(rules.authenticate(Some(&reg.token)).await.expect("auth"));
    }
    let [owner, leaver, removed, removed_by_op, quitter, hidden] = &agents[..] else {
        panic!("six agents");
    };
    let names_of = |list: &[&crate::rules::Agent]| list.iter().map(|a| a.name.clone()).collect();
    let town = rules
        .create_channel(
            owner,
            NewChannel {
                name: "town".into(),
                open: true,
                purpose: None,
                members: names_of(&[leaver, removed, removed_by_op, quitter, hidden]),
            },
        )
        .await
        .expect("town")
        .id;
    let side = rules
        .create_channel(
            owner,
            NewChannel {
                name: "side".into(),
                open: false,
                purpose: None,
                members: names_of(&[quitter, hidden]),
            },
        )
        .await
        .expect("side")
        .id;
    let mut live = source.live();
    let op = names::OPERATOR;

    // Leaving: the agent did it.
    rules
        .leave_channel(leaver, town.clone())
        .await
        .expect("leave");
    let event = next_change(&mut live).await;
    assert_eq!(
        event,
        LiveEvent::Left(Left {
            agent: leaver.name.clone(),
            channel_id: town.clone(),
            channel_name: "town".into(),
            member_count: 5,
            by: leaver.name.clone(),
            by_operator: false,
        })
    );
    let said = toast::of(&event).expect("a toast");
    assert_eq!(said.text(), format!("{} left #town", leaver.name));
    assert_eq!(said.hue_of, leaver.name);
    let (_, page) = get(&site, "/").await;
    assert_eq!(members_shown(&page, &town), 5);
    assert!(!lists_channel(&page, &leaver.name, &town));
    assert!(lists_channel(&page, &removed.name, &town));

    // Removal by the owner.
    rules
        .remove_from_channel(owner, town.clone(), removed.name.clone())
        .await
        .expect("remove");
    let event = next_change(&mut live).await;
    let LiveEvent::Left(l) = &event else {
        panic!("expected left, got {event:?}");
    };
    assert_eq!(
        (&l.agent, &l.by, l.by_operator, l.member_count),
        (&removed.name, &owner.name, false, 4)
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("{} removed {} from #town", owner.name, removed.name)
    );
    let (_, page) = get(&site, "/").await;
    assert_eq!(members_shown(&page, &town), 4);
    assert!(!lists_channel(&page, &removed.name, &town));

    // Removal by the operator.
    source
        .change_channel(
            town.clone(),
            ChannelAction::RemoveMember(removed_by_op.name.clone()),
        )
        .await
        .expect("remove");
    let event = next_change(&mut live).await;
    let LiveEvent::Left(l) = &event else {
        panic!("expected left, got {event:?}");
    };
    assert_eq!(
        (&l.agent, l.by.as_str(), l.by_operator, l.member_count),
        (&removed_by_op.name, op, true, 3)
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("operator removed {} from #town", removed_by_op.name)
    );
    let (_, page) = get(&site, "/").await;
    assert_eq!(members_shown(&page, &town), 3);
    assert!(!lists_channel(&page, &removed_by_op.name, &town));

    // Unregistering: the agent goes from every channel it was in.
    rules.unregister(quitter).await.expect("unregister");
    let event = next_change(&mut live).await;
    let LiveEvent::AgentHidden(h) = &event else {
        panic!("expected agent_hidden, got {event:?}");
    };
    assert_eq!(
        (&h.agent, &h.by, h.by_operator),
        (&quitter.name, &quitter.name, false)
    );
    let mut counts: Vec<(String, i64)> = h
        .channels
        .iter()
        .map(|c| (c.channel_id.clone(), c.member_count))
        .collect();
    counts.sort();
    let mut expected = vec![(town.clone(), 2), (side.clone(), 2)];
    expected.sort();
    assert_eq!(counts, expected);
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("{} unregistered", quitter.name)
    );
    let (_, page) = get(&site, "/").await;
    assert!(agent_row(&page, &quitter.name).is_none());
    for (id, count) in &counts {
        assert_eq!(members_shown(&page, id), *count);
    }

    // Hiding an agent: the operator did it.
    source
        .hide_agent(hidden.name.clone())
        .await
        .expect("hide agent");
    let event = next_change(&mut live).await;
    let LiveEvent::AgentHidden(h) = &event else {
        panic!("expected agent_hidden, got {event:?}");
    };
    assert_eq!(
        (&h.agent, h.by.as_str(), h.by_operator),
        (&hidden.name, op, true)
    );
    assert_eq!(h.channels.len(), 2);
    assert!(h.channels.iter().all(|c| c.member_count == 1), "{h:?}");
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("operator hid {}", hidden.name)
    );
    let (_, page) = get(&site, "/").await;
    assert!(agent_row(&page, &hidden.name).is_none());
    assert_eq!(members_shown(&page, &town), 1);
    assert_eq!(members_shown(&page, &side), 1);

    // Hiding a channel, by its owner and by the operator. It leaves the
    // default view, and is marked hidden, with no members, in the other;
    // no agent lists it.
    rules
        .hide_channel(owner, side.clone())
        .await
        .expect("hide side");
    let event = next_change(&mut live).await;
    assert_eq!(
        event,
        LiveEvent::ChannelHidden(ChannelHidden {
            channel_id: side.clone(),
            channel_name: "side".into(),
            by: owner.name.clone(),
            by_operator: false,
        })
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("{} hid #side", owner.name)
    );
    source
        .change_channel(town.clone(), ChannelAction::Hide)
        .await
        .expect("hide town");
    let event = next_change(&mut live).await;
    assert_eq!(
        event,
        LiveEvent::ChannelHidden(ChannelHidden {
            channel_id: town.clone(),
            channel_name: "town".into(),
            by: op.into(),
            by_operator: true,
        })
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        "operator hid #town"
    );
    let (_, page) = get(&site, "/").await;
    let (_, with_hidden) = get(&site, "/?hidden=1").await;
    for id in [&side, &town] {
        assert!(channel_row(&page, id).is_none(), "{id}");
        let row = channel_row(&with_hidden, id).expect("listed when asked");
        assert!(row.starts_with(" data-hidden-channel"), "{row}");
        assert!(row.contains(r#"<span class="badge gone""#), "{row}");
        assert_eq!(members_shown(&with_hidden, id), 0);
        for shown in [&page, &with_hidden] {
            assert!(!lists_channel(shown, &owner.name, id));
            assert!(!lists_channel(shown, &leaver.name, id));
        }
    }
}

/// The first function named `name` in the page's script, up to its end.
fn script_function<'a>(page: &'a str, name: &str) -> &'a str {
    let (_, rest) = page
        .split_once(&format!("  function {name}("))
        .unwrap_or_else(|| panic!("no function {name}"));
    rest.split_once("\n  }\n")
        .unwrap_or_else(|| panic!("no end to {name}"))
        .0
}

/// The page's script shows each event's toast and updates the dashboard
/// with it, draws every kind of toast part, and finds the dashboard by the
/// marks the page has. Removing a toast or dashboard call fails here.
#[tokio::test]
async fn the_page_script_shows_toasts_and_updates_the_dashboard() {
    let (_, page) = get(&site(rules_source().await), "/").await;
    for (event, update) in [
        ("agent", "dashboardAgent"),
        ("channel", "dashboardChannel"),
        ("joined", "dashboardJoined"),
        ("left", "dashboardLeft"),
        ("channel_hidden", "dashboardChannelHidden"),
        ("agent_hidden", "dashboardAgentHidden"),
    ] {
        let (_, rest) = page
            .split_once(&format!("source.addEventListener(\"{event}\", (e) => {{"))
            .unwrap_or_else(|| panic!("no handler for {event}"));
        let handler = rest.split_once("});").expect("handler's end").0;
        assert!(
            handler.contains("const d = JSON.parse(e.data);"),
            "{event}: {handler}"
        );
        assert!(handler.contains("toast(d.toast);"), "{event}: {handler}");
        assert!(
            handler.contains(&format!("{update}(d);")),
            "{event}: {handler}"
        );
    }

    // Every kind of part the server sends has a way to be drawn.
    let drawn = script_function(&page, "toastPart");
    let parts = [
        toast::Part::Text { text: "x".into() },
        toast::Part::Agent { name: "x".into() },
        toast::Part::Name { name: "x".into() },
        toast::Part::Channel {
            id: "x".into(),
            name: "x".into(),
        },
        toast::Part::Badge {
            class: "x".into(),
            text: "x".into(),
        },
    ];
    for part in parts {
        let json = serde_json::to_value(&part).expect("json");
        let kind = json["kind"].as_str().expect("a kind");
        if kind != "text" {
            assert!(drawn.contains(&format!("case \"{kind}\":")), "{kind}");
        }
    }
    assert!(drawn.contains("default: return p.text;"));
    let toast_fn = script_function(&page, "toast");
    assert!(toast_fn.contains("spec.parts.map(toastPart)"));
    assert!(toast_fn.contains("hue(spec.hue_of)"));
    assert!(toast_fn.contains("spec.icon"));
    assert!(toast_fn.contains("toasts.append(t)"));

    // What each dashboard update does with its event.
    let left = script_function(&page, "dashboardLeft");
    assert!(left.contains("setMembers(l.channel_id, l.member_count)"));
    assert!(left.contains(
        "changeChannels(l.agent, (pairs) => pairs.filter(([id]) => id !== l.channel_id))"
    ));
    let channel_hidden = script_function(&page, "dashboardChannelHidden");
    assert!(channel_hidden.contains(r#"dashboard.hasAttribute("data-show-hidden")"#));
    assert!(channel_hidden.contains("setMembers(h.channel_id, 0)"));
    assert!(channel_hidden.contains("row.dataset.hiddenChannel"));
    assert!(channel_hidden.contains("row.remove()"));
    assert!(channel_hidden.contains("pairs.filter(([id]) => id !== h.channel_id)"));
    let agent_hidden = script_function(&page, "dashboardAgentHidden");
    assert!(agent_hidden.contains("row.remove()"));
    assert!(agent_hidden.contains("agentRows.delete(h.agent)"));
    assert!(
        agent_hidden
            .contains("h.channels.forEach((c) => setMembers(c.channel_id, c.member_count))")
    );

    // The marks the script looks for are on the page.
    assert!(page.contains(r#"querySelector("td.members")"#));
    assert!(page.contains(r#"<td class="num members">"#));
    assert!(page.contains(r#"querySelector("td.channels")"#));
    assert!(page.contains(r#"<td class="small channels">"#));
    assert!(page.contains(r#"querySelectorAll("a[data-channel-id]")"#));
    assert!(page.contains(r#"" data-channel-id=""#));
    assert!(page.contains(r#"querySelectorAll("tr[data-agent]")"#));
    assert!(page.contains("<tr data-agent=\""));
    assert!(page.contains(r#"querySelectorAll("tr[data-channel-id]")"#));
    assert!(page.contains("<tr data-channel-id=\""));
    assert!(page.contains(r#"<div class="toasts" id="toasts""#));
    let (_, with_hidden) = get(&site(rules_source().await), "/?hidden=1").await;
    assert!(with_hidden.contains("data-dashboard data-show-hidden"));
}

#[tokio::test]
async fn preview_feed_survives_empty_channels_and_hiding_every_entry() {
    let p = preview(1, Arc::new(FixedClock::new(start_time())), &Shutdown::new());
    let overview = p.overview().await.unwrap();
    // Owners cannot be hidden, so the channels are left with none first.
    for channel in &overview.channels {
        p.change_channel(channel.channel.id.clone(), ChannelAction::SetOwner(None))
            .await
            .unwrap();
    }
    for agent in overview.agents {
        p.hide_agent(agent.agent.name).await.unwrap();
    }
    // No remaining channel members must not cause an entropy-index panic.
    p.step();
    for channel in p.overview().await.unwrap().channels {
        p.change_channel(channel.channel.id, ChannelAction::Hide)
            .await
            .unwrap();
    }
    assert!(p.overview().await.unwrap().channels.is_empty());
    p.step();
    assert!(!p.overview().await.unwrap().channels.is_empty());
}

async fn operator_editor_works(source: Arc<dyn Source>) {
    let site = site(source.clone());
    let (status, page) = get(&site, "/instructions").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("No versions saved."));
    let token = form_token_in(&page);
    let foreign = HttpRequest::builder()
        .method("POST")
        .uri("/instructions")
        .header("Host", "localhost")
        .header("Origin", "https://evil.example")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("text=forged&form_token={token}")))
        .unwrap();
    assert_eq!(
        site.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        post(&site, "/instructions", "text=forged").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&site, "/instructions", "text=forged&form_token=wrong")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert!(
        source
            .operator_instructions()
            .await
            .unwrap()
            .history
            .is_empty()
    );
    assert_eq!(
        post(
            &site,
            "/instructions",
            &format!("text=%3Cscript%3E%26%0Ahello&form_token={token}")
        )
        .await,
        (StatusCode::SEE_OTHER, Some("/instructions".into()))
    );
    let first = source.operator_instructions().await.unwrap();
    assert_eq!(first.history[0].version, 1);
    assert_eq!(first.history[0].text, "<script>&\nhello");
    let (_, page) = get(&site, "/instructions").await;
    assert!(page.contains("&#60;script&#62;&#38;"));
    assert!(!page.contains("<script>&"));
    assert!(page.contains("Version 1"));
    assert_eq!(
        post(&site, "/instructions", &format!("text=&form_token={token}"))
            .await
            .0,
        StatusCode::SEE_OTHER
    );
    let history = source.operator_instructions().await.unwrap().history;
    assert_eq!(history[0].text, "");
    assert_eq!(history[0].version, 2);
    assert_eq!(history[1], first.history[0]);
    let too_long = "x".repeat(first.max_chars + 1);
    assert_eq!(
        post(
            &site,
            "/instructions",
            &format!("text={too_long}&form_token={token}")
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        source.operator_instructions().await.unwrap().history,
        history
    );
    // A page read cannot make another version.
    get(&site, "/instructions?text=forged").await;
    assert_eq!(
        source.operator_instructions().await.unwrap().history.len(),
        2
    );
}
#[tokio::test]
async fn operator_editor_works_through_rules() {
    operator_editor_works(rules_source().await).await;
}
#[tokio::test]
async fn operator_editor_works_in_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    operator_editor_works(preview(1, clock, &Shutdown::new())).await;
}

async fn channel_creation_works(source: Arc<dyn Source>) {
    let before = source.overview().await.unwrap();
    let member = before.agents[0].agent.name.clone();
    let app = site(source.clone());
    let (_, html) = get(&app, "/").await;
    assert!(html.contains("Create a channel"));
    assert!(html.contains(&format!("name=\"member_{member}\"")));
    let token = form_token_in(&html);
    let fields = format!("name=feedback&purpose=Help+desk&open=false&member_{member}=yes");
    assert_eq!(
        get(&app, "/channels/create").await.0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    for invalid in [fields.clone(), format!("{fields}&form_token=bad")] {
        assert_eq!(
            post(&app, "/channels/create", &invalid).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let forged = HttpRequest::builder()
        .method("POST")
        .uri("/channels/create")
        .header("Host", "localhost")
        .header("Origin", "http://elsewhere.example")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("{fields}&form_token={token}")))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for invalid in [
        "name=closed&open=false",
        "name=&open=true",
        "name=bad%0Aname&open=true",
        "name=unknown&open=false&member_missing-agent=yes",
        "name=test&open=other",
    ] {
        assert_ne!(
            post(
                &app,
                "/channels/create",
                &format!("{invalid}&form_token={token}")
            )
            .await
            .0,
            StatusCode::SEE_OTHER
        );
    }
    assert_eq!(
        source.overview().await.unwrap().channels.len(),
        before.channels.len()
    );
    let (status, location) = post(
        &app,
        "/channels/create",
        &format!("{fields}&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.unwrap();
    let id = location.strip_prefix("/channels/").unwrap();
    let view = source.channel(id.into(), None).await.unwrap();
    assert_eq!(view.channel.name, "feedback");
    assert_eq!(view.channel.purpose.as_deref(), Some("Help desk"));
    assert!(!view.channel.open);
    assert_eq!(view.channel.created_by, "operator");
    assert_eq!(view.members.len(), 1);
    assert_eq!(view.members[0].name, member);
    assert_eq!(view.messages.len(), 1);
    assert!(view.messages[0].from_operator);
    assert_eq!(view.messages[0].body, "The operator created the channel.");
    assert!(view.messages[0].run_id.is_some());
    let (_, html) = get(&app, &location).await;
    assert!(!html.contains("/agents/operator"));
    let (status, location) = post(
        &app,
        "/channels/create",
        &format!("name=public&open=true&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let view = source
        .channel(location.unwrap()[10..].into(), None)
        .await
        .unwrap();
    assert!(view.channel.open);
    assert!(view.members.is_empty());
}

#[tokio::test]
async fn channel_creation_through_rules() {
    channel_creation_works(rules_source().await).await;
}

#[tokio::test]
async fn channel_creation_through_preview() {
    channel_creation_works(preview(
        1,
        Arc::new(FixedClock::new(start_time())),
        &Shutdown::new(),
    ))
    .await;
}

/// A form post to `path`, sent from this site, and the page it answers with.
async fn post_for_page(site: &Router, path: &str, body: &str) -> (StatusCode, String) {
    let request = HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Origin", "http://localhost")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// What every source must do for stale agents, given a world where at
/// least one channel owner and two other agents have been quiet for a day:
/// list them with every non-owner selected and owners not selectable, and
/// hide only the agents chosen, never an owner, each as hiding one does.
async fn stale_agents_work(source: Arc<dyn Source>) {
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let site = site(source.clone());
    let (_, front) = get(&site, "/").await;
    assert!(front.contains(r#"href="/stale-agents""#));

    let stale = source.stale_agents(day).await.expect("stale");
    let (status, page) = get(&site, "/stale-agents").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<option value="24" selected>1 day</option>"#));
    let rows: Vec<&str> = page
        .split("<tr data-stale=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let listed: Vec<&str> = stale.iter().map(|s| s.agent.name.as_str()).collect();
    assert_eq!(rows, listed);
    let (owners, others): (Vec<&StaleAgent>, Vec<&StaleAgent>) =
        stale.iter().partition(|s| !s.owns.is_empty());
    assert!(!owners.is_empty(), "the world has a quiet owner");
    assert!(others.len() >= 2, "the world has two quiet non-owners");
    for s in &others {
        assert!(page.contains(&format!(
            r#"name="agent_{}" value="yes" checked"#,
            s.agent.name
        )));
    }
    for s in &owners {
        assert!(!page.contains(&format!(r#"name="agent_{}""#, s.agent.name)));
        assert!(page.contains(&format!(
            r#"aria-label="{} owns a channel and cannot be hidden""#,
            s.agent.name
        )));
    }
    assert!(page.contains("Owns a channel, so cannot be hidden."));
    assert!(!page.contains("hide it from its own page"));

    let (status, _) = get(&site, "/stale-agents?hours=5").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The operator deselects one and sends the form, with an owner forged in.
    let chosen = &others[0].agent.name;
    let deselected = &others[1].agent.name;
    let owner = &owners[0].agent.name;
    let body = format!("hours=24&agent_{chosen}=yes&agent_{owner}=yes");
    let (status, _) = post_for_page(&site, "/stale-agents/hide", &body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    source.agent(chosen.clone()).await.expect("not hidden yet");

    let token = form_token_in(&page);
    let (status, _) = post_for_page(
        &site,
        "/stale-agents/hide",
        &format!("hours=5&agent_{chosen}=yes&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let was_in = source.agent(chosen.clone()).await.expect("view").channels;
    let (status, after) = post_for_page(
        &site,
        "/stale-agents/hide",
        &format!("{body}&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(after.contains(&format!("Hid 1: {chosen}.")), "{after}");
    assert!(after.contains(&format!("Kept {owner}: it owns a channel.")));
    assert!(!after.contains(&format!(r#"<tr data-stale="{chosen}""#)));
    assert!(after.contains(&format!(r#"<tr data-stale="{deselected}""#)));
    assert!(after.contains(&format!(r#"<tr data-stale="{owner}""#)));
    assert!(matches!(
        source.agent(chosen.clone()).await,
        Err(SourceError::NotFound(_))
    ));
    source
        .agent(deselected.clone())
        .await
        .expect("deselected kept");
    source.agent(owner.clone()).await.expect("owner kept");
    assert!(!was_in.is_empty());
    for c in was_in {
        let view = source.channel(c.id.clone(), None).await.expect("channel");
        let last = view.messages.last().expect("the operator's message");
        assert!(last.from_operator);
        assert_eq!(
            last.body,
            format!("The operator hid {chosen} and revoked its access. Its messages are kept.")
        );
    }
}

#[tokio::test]
async fn stale_agents_work_through_the_rules() {
    let (rules, source, clock) = rules_world_and_clock(Limits::DEFAULT).await;
    // A third agent, in the open channel, so two non-owners are quiet.
    let third = rules.register().await.expect("register");
    let third = rules.authenticate(Some(&third.token)).await.expect("auth");
    let overview = source.overview().await.expect("overview");
    let open = overview
        .channels
        .iter()
        .find(|c| c.channel.open)
        .expect("an open channel");
    rules
        .join_channel(&third, open.channel.id.clone())
        .await
        .expect("join");
    clock.advance(chrono::Duration::hours(25));
    stale_agents_work(source).await;
}

#[tokio::test]
async fn stale_agents_work_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    stale_agents_work(preview(1, clock, &Shutdown::new())).await;
}

/// What every source must do for the upgrade history: the front page shows
/// only this run's version and uptime, with a link to the history; the
/// history lists every run, newest first, with long values wrapped instead
/// of in a table that scrolls sideways.
async fn history_works(source: Arc<dyn Source>, uptime: &str) {
    let overview = source.overview().await.expect("overview");
    let current = overview.current_run.expect("the current run");
    let runs = source.server_runs().await.expect("runs");
    assert!(runs.len() >= 2, "an earlier run to show");
    assert_eq!(runs[0], current, "this run is the newest");
    let starts: Vec<&str> = runs.iter().map(|r| r.started_at.as_str()).collect();
    let mut newest_first = starts.clone();
    newest_first.sort_by(|a, b| b.cmp(a));
    assert_eq!(starts, newest_first);
    let site = site(source);

    let (status, page) = get(&site, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<a href="/history">Upgrade history</a>"#));
    assert!(page.contains(&format!("kkt {} · up ", current.version)));
    assert!(
        page.contains(&format!(">{uptime}</span>")),
        "uptime {uptime}"
    );
    for r in &runs[1..] {
        assert!(
            !page.contains(&r.git_commit),
            "the front page lists no earlier runs"
        );
    }

    let (status, page) = get(&site, "/history").await;
    assert_eq!(status, StatusCode::OK);
    let mut at = 0;
    for r in &runs {
        let found = page[at..]
            .find(&format!(r#"data-run="{}""#, r.id))
            .unwrap_or_else(|| panic!("run {} missing or out of order", r.id));
        at += found;
        for long in [&r.git_commit, &r.instructions_sha256] {
            assert!(page.contains(&format!(r#"class="mono wrap">{long}</dd>"#)));
        }
        // The settings hold nothing HTML escapes.
        assert!(page.contains(&format!(r#"<pre class="wrap">{}</pre>"#, r.settings)));
    }
    let (_, main) = page.split_once("<main>").expect("main");
    let (main, _) = main.split_once("</main>").expect("end of main");
    assert!(!main.contains("<table"), "no table to scroll sideways");
    assert!(!main.contains("nowrap"));
    assert!(!main.contains("scroll"));
}

#[tokio::test]
async fn history_works_through_the_rules() {
    let (_, source, clock) = rules_world_and_clock(Limits::DEFAULT).await;
    history_works(source.clone(), "less than a minute").await;
    clock.advance(chrono::Duration::minutes(125));
    history_works(source.clone(), "2 hours 5 minutes").await;
    clock.advance(chrono::Duration::days(3));
    history_works(source, "3 days 2 hours").await;
}

#[tokio::test]
async fn history_works_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    history_works(preview(1, clock, &Shutdown::new()), "6 hours").await;
}

#[test]
fn uptime_reads_as_the_two_largest_units() {
    let at = |s: &str| format!("2026-10-25T{s}Z");
    let up = |from: &str, to: &str| uptime(from, to).expect("readable times");
    assert_eq!(up(&at("12:00:00"), &at("12:00:59")), "less than a minute");
    assert_eq!(up(&at("12:00:00"), &at("12:01:00")), "1 minute");
    assert_eq!(up(&at("12:00:00"), &at("13:00:00")), "1 hour");
    assert_eq!(up(&at("12:00:00"), &at("14:30:00")), "2 hours 30 minutes");
    assert_eq!(
        up("2026-10-24T11:00:00Z", "2026-10-25T12:00:00Z"),
        "1 day 1 hour"
    );
    assert_eq!(up("2026-10-23T12:00:00Z", "2026-10-25T12:05:00Z"), "2 days");
    // A start after now, from a clock set back, counts as just started.
    assert_eq!(up(&at("13:00:00"), &at("12:00:00")), "less than a minute");
    assert_eq!(uptime("not a time", &at("12:00:00")), None);
}

#[test]
fn stale_periods_read_as_hours_or_days() {
    assert_eq!(period_label(1), "1 hour");
    assert_eq!(period_label(6), "6 hours");
    assert_eq!(period_label(24), "1 day");
    assert_eq!(period_label(168), "7 days");
    assert_eq!(period_label(36), "36 hours");
}
