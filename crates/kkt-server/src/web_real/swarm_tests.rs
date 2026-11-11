//! Two swarms on one web page. A viewer that `Identify` puts in another
//! swarm reads the server through the same `RulesSources` as key mode's
//! viewer: it must see nothing of `sw_main` on any page or in the live
//! feed, and its forms must change nothing there. Lanes, the server
//! announcement and server runs belong to the server, not a swarm, and are
//! the same for both until orgs arrive (phase 2).

use super::{AdmissionGate, RulesSources, WebViewer};
use kkt_admission::{Admission, Principal, Sessions, WebAdmission};
use kkt_clock::Span;
use kkt_clock::testing::FixedClock;
use kkt_entropy::testing::SeqEntropy;
use kkt_events::Events;
use kkt_hosts::AllowedHosts;
use kkt_identity::{Identified, Identify, Identifying, KeyMode};
use kkt_rules::{Actor, Agent, Deps, LaneSave, NewChannel, Operator, Rules, SwarmId};
use kkt_store::{Db, NewServerRunRecord};
use kkt_web::Sources;
use kkt_web::testing::{
    ACTIVITY_BUCKET, ACTIVITY_WINDOW, form_token_in, get, post, post_for_page, start_time,
};
use std::sync::Arc;

/// Words that appear only in `sw_main`'s data: a page or event that shows
/// one has shown `sw_main` to a viewer elsewhere.
const MAIN_WORDS: [&str; 8] = [
    "main-open",
    "main-closed",
    "main-hidden",
    "MAINPURPOSE",
    "MAINNOTE",
    "MAINMSG",
    "MAINMEM",
    "MAINDESC",
];

/// Puts every web viewer in `sw_other`, as key mode's web operator, so a
/// test can see what such a viewer is shown. Agent calls are not made here.
struct InOther;

impl Identify for InOther {
    fn avatar<'a>(
        &'a self,
        _rules: &'a Rules,
        _principal: &'a Principal,
        _token: Option<&'a str>,
    ) -> Identifying<'a, Identified<Agent>> {
        unreachable!("the web page makes no agent calls")
    }

    fn newcomer<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, SwarmId> {
        unreachable!("the web page registers no one")
    }

    fn operator<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, Identified<Operator>> {
        Box::pin(async move {
            let who = KeyMode::new(None).operator(principal).await?;
            Ok(Identified {
                swarm: other(),
                actor: who.actor,
            })
        })
    }
}

fn other() -> SwarmId {
    SwarmId::new("sw_other")
}

/// What `sw_main` holds, by name, for the checks.
struct Main {
    handles: Vec<String>,
    /// Each agent's recovery code, in `handles`' order.
    recovery_codes: Vec<String>,
    channels: Vec<String>,
}

impl Main {
    /// Everything that names `sw_main`'s data: its words, handles and
    /// channel IDs.
    fn markers(&self) -> Vec<String> {
        MAIN_WORDS
            .iter()
            .map(|w| w.to_string())
            .chain(self.handles.iter().cloned())
            .chain(self.channels.iter().cloned())
            .collect()
    }

    /// Fails if `text`, shown to the other swarm at `place`, names `sw_main`.
    fn assert_absent(&self, place: &str, text: &str) {
        self.assert_absent_but(place, text, &[]);
    }

    /// `assert_absent`, but `asked`, the IDs or handles the viewer asked
    /// for, may be repeated back: the viewer already knows them.
    fn assert_absent_but(&self, place: &str, text: &str, asked: &[&str]) {
        for marker in self.markers() {
            if asked.contains(&marker.as_str()) {
                continue;
            }
            assert!(!text.contains(&marker), "{place} shows {marker}:\n{text}");
        }
    }
}

/// What the other swarm holds.
struct OtherSwarm {
    handle: String,
    channel: String,
}

struct World {
    rules: Rules,
    clock: Arc<FixedClock>,
    sources: Arc<RulesSources>,
    main: Main,
    other: OtherSwarm,
    /// Key mode's viewer, in `sw_main`.
    main_viewer: WebViewer,
    /// The viewer `InOther` makes, in `sw_other`.
    other_viewer: WebViewer,
}

/// A database with two swarms. `sw_main` has two agents, an open, a closed
/// and a hidden channel with purposes, notes and messages, memory and a
/// description, all marked with `MAIN_WORDS`; `sw_other` has one agent and
/// one channel. The server has a lane and an announcement.
async fn world() -> World {
    let clock = Arc::new(FixedClock::new(start_time()));
    let db = Db::new(
        kkt_db_sqlite::MemoryPlace::new()
            .expect("an in-memory database")
            .open(&[kkt_store::schema()])
            .expect("the core's tables"),
    );
    let run_id = db
        .write(|w| {
            let run_id = w.insert_server_run(&NewServerRunRecord {
                started_at: "2026-10-25T12:00:00Z".into(),
                version: "test".into(),
                git_commit: "test".into(),
                settings: "test".into(),
                instructions_sha256: "test".into(),
            })?;
            w.insert_swarm(&other(), "2026-10-25T12:00:00Z", run_id)?;
            Ok::<_, kkt_store::StoreError>(run_id)
        })
        .await
        .expect("a server run and a second swarm");
    let rules = Rules::new(Deps {
        db,
        swarm: SwarmId::main(),
        events: Events::new(kkt_events::Settings::DEFAULT),
        clock: clock.clone(),
        entropy: Arc::new(SeqEntropy::new(4)),
        run_id,
        settings: kkt_rules::Settings::DEFAULT,
        wording: Arc::new(kkt_format::NoticeWording),
    });
    let main_viewer = viewer(Principal::Anyone, &KeyMode::new(None)).await;
    let other_viewer = viewer(Principal::Anyone, &InOther).await;
    assert_eq!(main_viewer.swarm, SwarmId::main());
    assert_eq!(other_viewer.swarm, other());

    let mut agents = Vec::new();
    let mut recovery_codes = Vec::new();
    for _ in 0..2 {
        let reg = rules.register().await.expect("register");
        agents.push(rules.authenticate(Some(&reg.token)).await.expect("auth"));
        recovery_codes.push(reg.recovery_code);
    }
    rules
        .set_description(&agents[0], "MAINDESC".into())
        .await
        .expect("describe");
    rules
        .remember(&agents[0], "handoff".into(), "MAINMEM".into())
        .await
        .expect("memory");
    let mut channels = Vec::new();
    for (name, open) in [
        ("main-open", true),
        ("main-closed", false),
        ("main-hidden", true),
    ] {
        let channel = rules
            .create_channel(
                &agents[0],
                NewChannel {
                    name: name.into(),
                    open,
                    purpose: Some("MAINPURPOSE".into()),
                    members: vec![agents[1].handle.clone()],
                },
            )
            .await
            .expect("channel");
        rules
            .set_note(&agents[1], channel.id.clone(), "MAINNOTE".into())
            .await
            .expect("note");
        for (i, agent) in agents.iter().enumerate() {
            rules
                .send(agent, channel.id.clone(), format!("MAINMSG {i}"))
                .await
                .expect("send");
        }
        channels.push(channel.id);
    }
    rules
        .hide_channel(&agents[0], channels[2].clone())
        .await
        .expect("hide");

    let theirs = rules.in_swarm(other());
    let reg = theirs.register().await.expect("register");
    let them = theirs.authenticate(Some(&reg.token)).await.expect("auth");
    let channel = theirs
        .create_channel(
            &them,
            NewChannel {
                name: "other-room".into(),
                open: true,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("channel");
    theirs
        .send(&them, channel.id.clone(), "hello from elsewhere".into())
        .await
        .expect("send");

    // The server's own: one lane, one announcement.
    let operator = Actor::Operator(main_viewer.operator.clone());
    rules
        .save_lane(
            operator.clone(),
            LaneSave {
                name: "shared-lane".into(),
                description: "SHAREDLANE".into(),
                text: "Read.".into(),
            },
        )
        .await
        .expect("lane");
    rules
        .save_server_announcement(operator, "SHAREDNEWS".into())
        .await
        .expect("announcement");

    World {
        sources: Arc::new(RulesSources::new(rules.clone())),
        rules,
        clock,
        main: Main {
            handles: agents.iter().map(|a| a.handle.clone()).collect(),
            recovery_codes,
            channels,
        },
        other: OtherSwarm {
            handle: them.handle,
            channel: channel.id,
        },
        main_viewer,
        other_viewer,
    }
}

/// The viewer the server's gate makes of `principal` through `identify`.
async fn viewer(principal: Principal, identify: &dyn Identify) -> WebViewer {
    let who = identify.operator(&principal).await.expect("an operator");
    WebViewer {
        swarm: who.swarm,
        operator: who.actor,
    }
}

/// The server's web page, with every viewer put in `sw_other`.
fn other_site(world: &World) -> axum::Router {
    let sessions = Sessions::new(
        world.clock.clone(),
        Arc::new(SeqEntropy::new(9)),
        kkt_admission::Settings::DEFAULT.web_session_lifetime,
    );
    let gate = AdmissionGate::new(
        WebAdmission::new(Arc::new(Admission::Open), sessions),
        Arc::new(InOther),
    );
    kkt_web::router(
        world.sources.clone(),
        AllowedHosts::new(&[]).expect("loopback hosts"),
        Arc::new(gate),
        kkt_web::Settings::DEFAULT,
    )
}

/// Everything key mode's viewer can read of `sw_main`, to compare before
/// and after the other swarm's forms.
async fn main_state(world: &World) -> String {
    let source = world.sources.source(&world.main_viewer);
    let mut state = format!(
        "{:?}\n{:?}\n",
        source
            .overview(ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .expect("overview"),
        source
            .hidden_channels(ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .expect("hidden")
    );
    for id in &world.main.channels {
        state += &format!(
            "{:?}\n",
            source
                .channel(id.clone(), None, kkt_web::Settings::DEFAULT.page_messages)
                .await
        );
    }
    for handle in &world.main.handles {
        state += &format!("{:?}\n", source.agent(handle.clone()).await);
    }
    state
}

#[tokio::test]
async fn no_page_shows_another_swarm() {
    let world = world().await;
    let site = other_site(&world);
    // Long enough for every agent to be quiet on the stale-agents page.
    world.clock.advance(Span::days(10));

    // The other swarm's own pages work, and show only its own.
    let other_pages = [
        "/".to_string(),
        "/?hidden=1".to_string(),
        "/stale-agents?hours=1".to_string(),
        format!("/channels/{}", world.other.channel),
        format!("/agents/{}", world.other.handle),
    ];
    for path in &other_pages {
        let (status, page) = get(&site, path).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{path}");
        world.main.assert_absent(path, &page);
    }
    let (_, front) = get(&site, "/").await;
    assert!(front.contains(&world.other.channel));
    assert!(front.contains(&world.other.handle));
    let (_, stale) = get(&site, "/stale-agents?hours=1").await;
    assert!(stale.contains(&world.other.handle), "its own quiet agent");

    // `sw_main`'s channels and agents do not exist there.
    let mut missing = Vec::new();
    for id in &world.main.channels {
        missing.push((format!("/channels/{id}"), id));
        missing.push((format!("/channels/{id}/messages?before_id=1000000"), id));
    }
    for handle in &world.main.handles {
        missing.push((format!("/agents/{handle}"), handle));
    }
    for (path, asked) in &missing {
        let (status, page) = get(&site, path).await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "{path}");
        world.main.assert_absent_but(path, &page, &[asked.as_str()]);
    }

    // The server's own pages are the same for both swarms.
    for (path, shared) in [
        ("/lanes", "shared-lane"),
        ("/lanes/shared-lane", "SHAREDLANE"),
        ("/announcement", "SHAREDNEWS"),
        ("/history", "2026-10-25"),
    ] {
        let (status, page) = get(&site, path).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{path}");
        assert!(page.contains(shared), "{path} shows {shared}");
        world.main.assert_absent(path, &page);
    }
}

#[tokio::test]
async fn the_live_feed_carries_only_the_viewers_swarm() {
    let world = world().await;
    let mut theirs = world.sources.source(&world.other_viewer).live();
    let mut ours = world.sources.source(&world.main_viewer).live();

    // Everything that happens in `sw_main` that the page shows live.
    let reg = world.rules.register().await.expect("register");
    let newcomer = world
        .rules
        .authenticate(Some(&reg.token))
        .await
        .expect("auth");
    let channel = world
        .rules
        .create_channel(
            &newcomer,
            NewChannel {
                name: "main-open-2".into(),
                open: true,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("channel");
    world
        .rules
        .join_channel(&newcomer, world.main.channels[0].clone())
        .await
        .expect("join");
    world
        .rules
        .send(&newcomer, channel.id.clone(), "MAINMSG live".into())
        .await
        .expect("send");
    world
        .rules
        .leave_channel(&newcomer, world.main.channels[0].clone())
        .await
        .expect("leave");
    world
        .rules
        .hide_channel(&newcomer, channel.id.clone())
        .await
        .expect("hide");
    world.rules.unregister(&newcomer).await.expect("unregister");

    // Then one message in the other swarm: the first thing its feed
    // carries, so nothing of `sw_main` came before it.
    let elsewhere = world.rules.in_swarm(other());
    let reg = elsewhere.register().await.expect("register");
    let them = elsewhere
        .authenticate(Some(&reg.token))
        .await
        .expect("auth");
    elsewhere
        .join_channel(&them, world.other.channel.clone())
        .await
        .expect("join");

    use futures::StreamExt;
    let first = theirs.next().await.expect("an event");
    let shown = format!("{first:?}");
    world
        .main
        .assert_absent("the other swarm's live feed", &shown);
    assert!(shown.contains(&reg.handle), "{shown}");
    assert!(
        !shown.contains(&newcomer.handle),
        "the other swarm's feed shows sw_main's newcomer: {shown}"
    );

    // `sw_main`'s viewer heard its own swarm's first.
    let first = ours.next().await.expect("an event");
    assert!(format!("{first:?}").contains(&newcomer.handle), "{first:?}");
}

#[tokio::test]
async fn no_form_changes_another_swarm() {
    let world = world().await;
    let site = other_site(&world);
    world.clock.advance(Span::days(10));
    let before = main_state(&world).await;
    let (_, front) = get(&site, "/").await;
    let token = form_token_in(&front);
    let main_handle = &world.main.handles[1];
    let other_handle = &world.other.handle;

    let mut forms = Vec::new();
    for id in &world.main.channels {
        forms.extend([
            (
                format!("/channels/{id}/purpose"),
                "purpose=changed".to_string(),
            ),
            (
                format!("/channels/{id}/visibility"),
                "open=false".to_string(),
            ),
            (
                format!("/channels/{id}/visibility"),
                "open=true".to_string(),
            ),
            (
                format!("/channels/{id}/members/add"),
                format!("handle={other_handle}"),
            ),
            (
                format!("/channels/{id}/members/remove"),
                format!("handle={main_handle}"),
            ),
            (format!("/channels/{id}/owner"), "owner=".to_string()),
            (format!("/channels/{id}/hide"), String::new()),
        ]);
    }
    for handle in &world.main.handles {
        forms.push((format!("/agents/{handle}/recovery-code"), String::new()));
        forms.push((format!("/agents/{handle}/hide"), String::new()));
    }
    let chosen: String = world
        .main
        .handles
        .iter()
        .map(|h| format!("&agent_{h}=on"))
        .collect();
    // Adding one of `sw_main`'s agents to the other swarm's channel.
    forms.push((
        format!("/channels/{}/members/add", world.other.channel),
        format!("handle={main_handle}"),
    ));

    // Each names a channel or agent the other swarm does not have.
    for (path, fields) in &forms {
        let body = match fields.is_empty() {
            true => format!("form_token={token}"),
            false => format!("form_token={token}&{fields}"),
        };
        let (status, page) = post_for_page(&site, path, &body).await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "{path}: {page}");
    }
    // Hiding stale agents names agents by handle in its fields; the other
    // swarm has none of those, so it hides nothing and shows its own page,
    // saying only that the handles it was sent were kept.
    let (status, page) = post_for_page(
        &site,
        "/stale-agents/hide",
        &format!("form_token={token}&hours=1{chosen}"),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let asked: Vec<&str> = world.main.handles.iter().map(String::as_str).collect();
    world
        .main
        .assert_absent_but("hiding stale agents", &page, &asked);
    for handle in &asked {
        assert!(
            page.contains(&format!(
                "Kept {handle}: it is already hidden or does not exist."
            )),
            "{page}"
        );
    }

    assert_eq!(main_state(&world).await, before, "sw_main changed");

    // A channel made there is made in the other swarm, not `sw_main`.
    let (status, _) = post(
        &site,
        "/channels/create",
        &format!("form_token={token}&name=made-elsewhere&open=true"),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::SEE_OTHER);
    assert_eq!(main_state(&world).await, before, "sw_main changed");
    let (_, front) = get(&site, "/").await;
    assert!(front.contains("made-elsewhere"));
    let other_member = world.sources.source(&world.other_viewer);
    let channel = other_member
        .channel(
            world.other.channel.clone(),
            None,
            kkt_web::Settings::DEFAULT.page_messages,
        )
        .await
        .expect("the other swarm's channel");
    assert!(
        channel.members.iter().all(|m| m.handle != *main_handle),
        "sw_main's agent joined the other swarm's channel"
    );

    // Last, as reclaiming is a change: no recovery code was issued, so
    // each agent's first code still works.
    for (handle, code) in world.main.handles.iter().zip(&world.main.recovery_codes) {
        world
            .rules
            .reclaim(handle.clone(), code.clone())
            .await
            .unwrap_or_else(|e| panic!("{handle}'s first recovery code: {e:?}"));
    }
}
