//! The page scenarios run against the server's source, over the rules and an
//! in-memory database, behind the server's gate over real admission; plus
//! page tests that need the rules.

use super::{AdmissionGate, RulesSource};
use axum::http::StatusCode;
use kkt_admission::{Admission, Sessions, WebAdmission};
use kkt_clock::Span;
use kkt_clock::testing::FixedClock;
use kkt_config::Limits;
use kkt_events::Events;
use kkt_names as names;
use kkt_names::testing::SeqEntropy;
use kkt_rules::{Deps, NewChannel, Rules};
use kkt_store::{Db, NewServerRunRecord};
use kkt_web::testing::{
    ChannelHidden, administration_works, agent_row, announcement_editor_works, changes_work,
    channel_creation_works, channel_row, earlier_pages_work, get, handoff_works,
    hidden_channels_show_only_on_request, history_works, lanes_pages_work, lists_channel,
    member_lists_show_lanes_and_latest_messages, members_shown, next_change,
    operator_changes_are_live, page_script_works, pages_work, site, stale_agents_work, start_time,
    summaries_agree_with_the_pages, toast,
};
use kkt_web::{ChannelAction, Gate, Left, LiveEvent, Source};
use std::sync::Arc;

/// The server's gate over real admission, open: for the rules' runs.
fn rules_gate() -> Arc<dyn Gate> {
    let clock = Arc::new(FixedClock::new(start_time()));
    let sessions = Sessions::new(
        clock,
        Arc::new(SeqEntropy::new(9)),
        Limits::DEFAULT.web_session_lifetime,
    );
    Arc::new(AdmissionGate::new(WebAdmission::new(
        Arc::new(Admission::Open),
        sessions,
    )))
}

#[tokio::test]
async fn handoff_works_through_the_rules() {
    let (rules, source) = rules_world().await;
    let (name, code) = handoff_works(source, rules_gate()).await;
    // The code shown on the page is the one that now takes over the avatar.
    let taken = rules.reclaim(name, code).await.expect("reclaim");
    rules.authenticate(Some(&taken.token)).await.expect("auth");
}

#[tokio::test]
async fn earlier_pages_work_through_the_rules() {
    let limits = Limits {
        web_page_messages: 2,
        ..Limits::DEFAULT
    };
    let (_, source) = rules_world_with(limits).await;
    assert!(earlier_pages_work(source, rules_gate()).await >= 2);
}

#[tokio::test]
async fn changes_work_through_the_rules() {
    changes_work(rules_source().await, rules_gate()).await;
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
    let (rules, source, clock, _) = rules_world_and_db(limits).await;
    (rules, source, clock)
}

/// `rules_world_and_clock`, and the operator's commands on the same
/// database, as `kkt agent` and `kkt lanes` use them.
async fn rules_world_and_db(
    limits: Limits,
) -> (Rules, Arc<dyn Source>, Arc<FixedClock>, kkt_rules::Operator) {
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
    let operator = kkt_rules::Operator::new(db.clone(), kkt_store::SwarmId::main(), clock.clone());
    let rules = Rules::new(Deps {
        db,
        swarm: kkt_store::SwarmId::main(),
        events: Events::new(),
        clock: clock.clone(),
        entropy: Arc::new(SeqEntropy::new(4)),
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
                    members: vec![agents[1].handle.clone()],
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
        .remember(
            &agents[0],
            "handoff".into(),
            "Next: the parser.\nOwner approves pushes.".into(),
        )
        .await
        .expect("memory entry");
    rules
        .remember(&agents[0], "rules".into(), "<b>Ask first.</b>".into())
        .await
        .expect("memory entry");
    (
        rules.clone(),
        Arc::new(RulesSource::new(rules)),
        clock,
        operator,
    )
}

#[tokio::test]
async fn the_lane_page_says_where_an_imported_version_came_from() {
    let (rules, source, _, operator) = rules_world_and_db(Limits::DEFAULT).await;
    rules
        .save_lane(
            kkt_rules::Actor::Human,
            kkt_rules::LaneSave {
                name: "reviewer".into(),
                description: "Reviews.".into(),
                text: "Read.".into(),
            },
        )
        .await
        .expect("lane");
    operator
        .import_lanes(kkt_rules::LaneImport {
            lanes: vec![kkt_rules::LaneSave {
                name: "reviewer".into(),
                description: "Reviews.".into(),
                text: "Read twice.".into(),
            }],
            source: kkt_rules::LaneSource {
                folder: "/home/sam/lanes".into(),
                commit: Some("abc123-dirty".into()),
            },
            retire_missing: false,
            skipped_files: Vec::new(),
            max_lane_chars: Limits::DEFAULT.max_lane_chars,
        })
        .await
        .expect("import");
    let (_, page) = get(&site(source, rules_gate()), "/lanes/reviewer").await;
    assert!(
        page.contains("Imported from /home/sam/lanes at git commit abc123-dirty"),
        "{page}"
    );
    // Only the imported version says so.
    assert_eq!(page.matches("Imported from").count(), 1, "{page}");
}

#[tokio::test]
async fn agent_pages_and_the_front_page_show_lanes_and_mark_old_versions() {
    let (rules, source) = rules_world().await;
    let save = |text: &str| kkt_rules::LaneSave {
        name: "reviewer".into(),
        description: "Reviews.".into(),
        text: text.into(),
    };
    rules
        .save_lane(kkt_rules::Actor::Human, save("Read."))
        .await
        .expect("lane");
    let reg = rules
        .register_with_client(None, None, Some("reviewer".into()))
        .await
        .expect("register");
    let site = site(source, rules_gate());
    let page_of = format!("/agents/{}", reg.handle);
    let (_, page) = get(&site, &page_of).await;
    assert!(
        page.contains(r#"<a href="/lanes/reviewer">reviewer</a>, version 1"#),
        "{page}"
    );
    assert!(!page.contains("out of date"), "{page}");

    rules
        .save_lane(kkt_rules::Actor::Human, save("Read twice."))
        .await
        .expect("lane");
    let (_, page) = get(&site, &page_of).await;
    assert!(page.contains("out of date"), "{page}");
    assert!(page.contains("Version 2 is the newest"), "{page}");
    let (_, front) = get(&site, "/").await;
    assert!(
        front.contains(r#"<a href="/lanes/reviewer">reviewer</a> version 1"#),
        "{front}"
    );
    assert!(front.contains("out of date"), "{front}");
}

#[tokio::test]
async fn member_lists_show_lanes_and_latest_messages_from_the_rules() {
    let limits = Limits {
        web_member_excerpt_chars: 12,
        ..Limits::DEFAULT
    };
    let (rules, source) = rules_world_with(limits).await;
    let open = source
        .overview()
        .await
        .expect("overview")
        .channels
        .into_iter()
        .find(|c| c.channel.open)
        .expect("an open channel")
        .channel
        .id;
    rules
        .save_lane(
            kkt_rules::Actor::Human,
            kkt_rules::LaneSave {
                name: "reviewer".into(),
                description: "Reviews.".into(),
                text: "Read.".into(),
            },
        )
        .await
        .expect("lane");
    let reg = rules.register().await.expect("register");
    let newcomer = rules.authenticate(Some(&reg.token)).await.expect("auth");
    rules
        .take_lane(&newcomer, Some("reviewer".into()))
        .await
        .expect("take lane");
    rules
        .join_channel(&newcomer, open.clone())
        .await
        .expect("join");
    let (_, page) = get(&rules_site(source.clone()), &format!("/channels/{open}")).await;
    let row = member_row(&page, &newcomer.handle);
    assert!(
        row.contains("lane <a href=\"/lanes/reviewer\">reviewer</a> version 1"),
        "{row}"
    );
    assert!(!row.contains("class=\"latest"), "no message yet: {row}");

    let sent = rules
        .send(
            &newcomer,
            open.clone(),
            "Looking at the\nparser now, then tests".into(),
        )
        .await
        .expect("send");
    let (_, page) = get(&rules_site(source.clone()), &format!("/channels/{open}")).await;
    let row = member_row(&page, &newcomer.handle);
    assert!(
        row.contains(&format!(
            "href=\"/channels/{open}?before_id={}#m{}\" title=\"Message #{}\">Looking at t…</a>",
            sent.id + 1,
            sent.id,
            sent.id
        )),
        "{row}"
    );
    assert!(member_lists_show_lanes_and_latest_messages(source, rules_gate()).await >= 3);
}

/// The site over `source`, behind the open gate.
fn rules_site(source: Arc<dyn Source>) -> axum::Router {
    site(source, rules_gate())
}

/// The member list's row for `handle` on a channel page.
fn member_row<'a>(page: &'a str, handle: &str) -> &'a str {
    let start = page
        .find(&format!("<li><a href=\"/agents/{handle}\""))
        .expect("a member row");
    let row = &page[start..];
    &row[..row.find("</li>").expect("the row's end")]
}

#[tokio::test]
async fn pages_work_from_the_rules() {
    pages_work(rules_source().await, rules_gate()).await;
}

#[tokio::test]
async fn hidden_channels_show_only_on_request_from_the_rules() {
    hidden_channels_show_only_on_request(rules_source().await, rules_gate()).await;
}

#[tokio::test]
async fn summaries_agree_with_the_pages_from_the_rules() {
    summaries_agree_with_the_pages(rules_source().await, rules_gate()).await;
}

#[tokio::test]
async fn channel_page_shows_the_pinned_note_and_editor_as_escaped_text() {
    let source = rules_source().await;
    let overview = source.overview().await.unwrap();
    let ch = &overview.channels[0].channel;
    let (status, page) = get(
        &site(source.clone(), rules_gate()),
        &format!("/channels/{}", ch.id),
    )
    .await;
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

#[tokio::test]
async fn administration_works_through_the_rules() {
    administration_works(rules_source().await, rules_gate()).await;
}

#[tokio::test]
async fn operator_changes_are_live_through_the_rules() {
    operator_changes_are_live(rules_source().await).await;
}

/// Each way an agent leaves a channel, a channel is hidden, or an agent goes:
/// the live event says who did it and what remains, its toast says so in
/// words, and the dashboard drawn afresh agrees with what the page's script
/// does with the event (`dashboardLeft`, `dashboardChannelHidden`,
/// `dashboardAgentHidden` in `base.html`).
#[tokio::test]
async fn leaving_removal_and_hiding_reach_the_toasts_and_dashboard() {
    let (rules, source) = rules_world().await;
    let site = site(source.clone(), rules_gate());
    let mut agents = Vec::new();
    for _ in 0..6 {
        let reg = rules.register().await.expect("register");
        agents.push(rules.authenticate(Some(&reg.token)).await.expect("auth"));
    }
    let [owner, leaver, removed, removed_by_op, quitter, hidden] = &agents[..] else {
        panic!("six agents");
    };
    let names_of = |list: &[&kkt_rules::Agent]| list.iter().map(|a| a.handle.clone()).collect();
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
            agent: leaver.handle.clone(),
            channel_id: town.clone(),
            channel_name: "town".into(),
            member_count: 5,
            by: leaver.handle.clone(),
            by_operator: false,
        })
    );
    let said = toast::of(&event).expect("a toast");
    assert_eq!(said.text(), format!("{} left #town", leaver.handle));
    assert_eq!(said.hue_of, leaver.handle);
    let (_, page) = get(&site, "/").await;
    assert_eq!(members_shown(&page, &town), 5);
    assert!(!lists_channel(&page, &leaver.handle, &town));
    assert!(lists_channel(&page, &removed.handle, &town));

    // Removal by the owner.
    rules
        .remove_from_channel(owner, town.clone(), removed.handle.clone())
        .await
        .expect("remove");
    let event = next_change(&mut live).await;
    let LiveEvent::Left(l) = &event else {
        panic!("expected left, got {event:?}");
    };
    assert_eq!(
        (&l.agent, &l.by, l.by_operator, l.member_count),
        (&removed.handle, &owner.handle, false, 4)
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("{} removed {} from #town", owner.handle, removed.handle)
    );
    let (_, page) = get(&site, "/").await;
    assert_eq!(members_shown(&page, &town), 4);
    assert!(!lists_channel(&page, &removed.handle, &town));

    // Removal by the operator.
    source
        .change_channel(
            town.clone(),
            ChannelAction::RemoveMember(removed_by_op.handle.clone()),
        )
        .await
        .expect("remove");
    let event = next_change(&mut live).await;
    let LiveEvent::Left(l) = &event else {
        panic!("expected left, got {event:?}");
    };
    assert_eq!(
        (&l.agent, l.by.as_str(), l.by_operator, l.member_count),
        (&removed_by_op.handle, op, true, 3)
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("operator removed {} from #town", removed_by_op.handle)
    );
    let (_, page) = get(&site, "/").await;
    assert_eq!(members_shown(&page, &town), 3);
    assert!(!lists_channel(&page, &removed_by_op.handle, &town));

    // Unregistering: the agent goes from every channel it was in.
    rules.unregister(quitter).await.expect("unregister");
    let event = next_change(&mut live).await;
    let LiveEvent::AgentHidden(h) = &event else {
        panic!("expected agent_hidden, got {event:?}");
    };
    assert_eq!(
        (&h.agent, &h.by, h.by_operator),
        (&quitter.handle, &quitter.handle, false)
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
        format!("{} unregistered", quitter.handle)
    );
    assert_eq!(
        serde_json::to_value(toast::of(&event).expect("a toast").parts[0].clone()).expect("json"),
        serde_json::json!({"kind": "gone_agent", "handle": quitter.handle})
    );
    let (_, page) = get(&site, "/").await;
    assert!(agent_row(&page, &quitter.handle).is_none());
    for (id, count) in &counts {
        assert_eq!(members_shown(&page, id), *count);
    }

    // Hiding an agent: the operator did it.
    source
        .hide_agent(hidden.handle.clone())
        .await
        .expect("hide agent");
    let event = next_change(&mut live).await;
    let LiveEvent::AgentHidden(h) = &event else {
        panic!("expected agent_hidden, got {event:?}");
    };
    assert_eq!(
        (&h.agent, h.by.as_str(), h.by_operator),
        (&hidden.handle, op, true)
    );
    assert_eq!(h.channels.len(), 2);
    assert!(h.channels.iter().all(|c| c.member_count == 1), "{h:?}");
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("operator hid {}", hidden.handle)
    );
    assert_eq!(
        serde_json::to_value(toast::of(&event).expect("a toast").parts).expect("json"),
        serde_json::json!([
            {"kind": "name", "name": "operator"},
            {"kind": "text", "text": " hid "},
            {"kind": "gone_agent", "handle": hidden.handle},
        ])
    );
    let (_, page) = get(&site, "/").await;
    assert!(agent_row(&page, &hidden.handle).is_none());
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
            by: owner.handle.clone(),
            by_operator: false,
        })
    );
    assert_eq!(
        toast::of(&event).expect("a toast").text(),
        format!("{} hid #side", owner.handle)
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
            assert!(!lists_channel(shown, &owner.handle, id));
            assert!(!lists_channel(shown, &leaver.handle, id));
        }
    }
}

/// The page's script shows each event's toast and updates the dashboard
/// with it, draws every kind of toast part, and finds the dashboard by the
/// marks the page has (`page_script_works`), on the server's front page.
#[tokio::test]
async fn the_page_script_shows_toasts_and_updates_the_dashboard() {
    let (_, page) = get(&site(rules_source().await, rules_gate()), "/").await;
    let (_, with_hidden) = get(&site(rules_source().await, rules_gate()), "/?hidden=1").await;
    page_script_works(&page, &with_hidden);
}

#[tokio::test]
async fn announcement_editor_works_through_rules() {
    announcement_editor_works(rules_source().await, rules_gate()).await;
}
#[tokio::test]
async fn the_history_names_the_admin_agent_that_saved_a_version() {
    let (rules, source) = rules_world().await;
    let site = site(source.clone(), rules_gate());
    let admin = kkt_names::AdminName::new("sentinel", kkt_config::MAX_ADMIN_NAME_CHARS).unwrap();
    rules
        .save_server_announcement(kkt_rules::Actor::Admin(admin), "Be brief.".into())
        .await
        .unwrap();
    rules
        .save_server_announcement(kkt_rules::Actor::Human, "Be kind.".into())
        .await
        .unwrap();
    let history = source.server_announcement().await.unwrap().history;
    assert_eq!(history[0].saved_by_admin, None);
    assert_eq!(history[1].saved_by_admin.as_deref(), Some("admin-sentinel"));
    let (_, page) = get(&site, "/announcement").await;
    let admin_at = page.find("by the admin agent admin-sentinel").expect(&page);
    let human_at = page.find("by you").expect(&page);
    // Newest first: the human's version 2, then the admin's version 1.
    assert!(human_at < admin_at, "{page}");
}

#[tokio::test]
async fn lanes_pages_work_through_the_rules() {
    lanes_pages_work(rules_source().await, rules_gate()).await;
}

#[tokio::test]
async fn a_lane_page_names_the_admin_agent_that_saved_or_retired_it() {
    let (rules, source) = rules_world().await;
    let site = site(source, rules_gate());
    let admin = || {
        kkt_rules::Actor::Admin(
            kkt_names::AdminName::new("sentinel", kkt_config::MAX_ADMIN_NAME_CHARS).unwrap(),
        )
    };
    rules
        .save_lane(
            admin(),
            kkt_rules::LaneSave {
                name: "reviewer".into(),
                description: "d".into(),
                text: "t".into(),
            },
        )
        .await
        .unwrap();
    rules.retire_lane(admin(), "reviewer".into()).await.unwrap();
    let (_, page) = get(&site, "/lanes/reviewer").await;
    assert_eq!(
        page.matches("by the admin agent admin-sentinel").count(),
        2,
        "{page}"
    );
    assert!(!page.contains("by you"), "{page}");
}

#[tokio::test]
async fn channel_creation_through_rules() {
    channel_creation_works(rules_source().await, rules_gate()).await;
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
    clock.advance(Span::hours(25));
    stale_agents_work(source, rules_gate()).await;
}

#[tokio::test]
async fn history_works_through_the_rules() {
    let (_, source, clock) = rules_world_and_clock(Limits::DEFAULT).await;
    history_works(source.clone(), rules_gate(), "less than a minute").await;
    clock.advance(Span::minutes(125));
    history_works(source.clone(), rules_gate(), "2 hours 5 minutes").await;
    clock.advance(Span::days(3));
    history_works(source, rules_gate(), "3 days 2 hours").await;
}
