//! The page scenarios run against the preview, behind its simulated gate;
//! plus tests of the preview's simulated world, feed and gate.

use crate::preview::Happening;
use crate::testing::{
    administration_works, announcement_editor_works, changes_work, channel_creation_works,
    earlier_pages_work, get, handoff_works, hidden_channels_show_only_on_request, history_works,
    lanes_pages_work, member_lists_show_lanes_and_latest_messages, operator_changes_are_live,
    page_script_works, pages_work, site, stale_agents_work, start_time,
    summaries_agree_with_the_pages,
};
use crate::*;
use futures::StreamExt;
use kkt_clock::Span;
use kkt_clock::testing::FixedClock;
use kkt_config::{Limits, PreviewSettings};
use kkt_names::SeededEntropy;
use kkt_names::testing::SeqEntropy;
use kkt_shutdown::Shutdown;

/// The preview's simulated gate, open: for the preview's runs.
fn preview_gate() -> Arc<dyn Gate<Viewer = ()>> {
    Arc::new(PreviewGate::open(
        Arc::new(FixedClock::new(start_time())),
        Arc::new(SeqEntropy::new(9)),
        Limits::DEFAULT.web_session_lifetime,
    ))
}

#[tokio::test]
async fn handoff_works_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    handoff_works(preview(1, clock, &Shutdown::new()), preview_gate()).await;
}

#[tokio::test]
async fn the_preview_has_a_channel_long_enough_to_page_back_through() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let source = preview(1, clock, &Shutdown::new());
    assert_eq!(source.page_messages(), Limits::DEFAULT.web_page_messages);
    assert!(
        earlier_pages_work(source, preview_gate()).await >= 3,
        "several pages back"
    );
}

#[tokio::test]
async fn changes_work_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    changes_work(preview(1, clock, &Shutdown::new()), preview_gate()).await;
}

fn preview(seed: u64, clock: Arc<FixedClock>, shutdown: &Shutdown) -> Arc<Preview> {
    Arc::new(
        Preview::new(
            clock,
            Arc::new(SeededEntropy::new(seed)),
            PreviewSettings::DEFAULT,
            shutdown,
        )
        .expect("the preview's constants are valid spans"),
    )
}

#[tokio::test]
async fn member_lists_show_lanes_and_latest_messages_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let source = preview(1, clock, &Shutdown::new());
    assert_eq!(
        source.member_excerpt_chars(),
        Limits::DEFAULT.web_member_excerpt_chars
    );
    member_lists_show_lanes_and_latest_messages(source, preview_gate()).await;
}

#[tokio::test]
async fn pages_work_from_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    pages_work(preview(1, clock, &Shutdown::new()), preview_gate()).await;
}

#[tokio::test]
async fn the_page_script_shows_toasts_and_updates_the_dashboard_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let site = site(preview(1, clock, &Shutdown::new()), preview_gate());
    let (_, page) = get(&site, "/").await;
    let (_, with_hidden) = get(&site, "/?hidden=1").await;
    page_script_works(&page, &with_hidden);
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
    hidden_channels_show_only_on_request(p, preview_gate()).await;
}

#[tokio::test]
async fn the_show_older_link_appears_only_when_older_messages_exist() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(1, clock, &Shutdown::new());
    let site = site(p.clone(), preview_gate());
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

#[tokio::test]
async fn summaries_agree_with_the_pages_from_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let p = preview(4, clock, &Shutdown::new());
    for _ in 0..30 {
        p.step();
    }
    summaries_agree_with_the_pages(p, preview_gate()).await;
}

const PREVIEW_KEY: &str = "preview-key-0123456789-abcdefghijklmnop";

/// A preview gate with `PREVIEW_KEY`, sessions lasting the default lifetime,
/// and the clock that times them.
fn keyed_preview_gate() -> (PreviewGate, Arc<FixedClock>) {
    let clock = Arc::new(FixedClock::new(start_time()));
    let gate = PreviewGate::with_key(
        &format!("  {PREVIEW_KEY}\n"),
        clock.clone(),
        Arc::new(SeqEntropy::new(9)),
        Limits::DEFAULT.web_session_lifetime,
    )
    .expect("a good key");
    (gate, clock)
}

/// Signs in to `gate` with `key` and returns the session ID.
fn signed_in(gate: &PreviewGate, key: &str) -> String {
    match gate.sign_in(key) {
        SignIn::Started {
            session_id,
            lifetime,
        } => {
            assert_eq!(lifetime, Limits::DEFAULT.web_session_lifetime);
            session_id
        }
        SignIn::Refused(reason) => panic!("refused: {reason}"),
    }
}

#[tokio::test]
async fn the_preview_gate_is_open_without_a_key() {
    let gate = PreviewGate::open(
        Arc::new(FixedClock::new(start_time())),
        Arc::new(SeqEntropy::new(9)),
        Limits::DEFAULT.web_session_lifetime,
    );
    assert!(!gate.sign_in_required());
    assert_eq!(gate.admits(None).await, Admit::Yes(()));
    assert_eq!(gate.admits(Some("never-issued")).await, Admit::Yes(()));
    // One form token for everyone, as the server gives an open page.
    let token = gate.form_token(None).expect("a form token");
    assert_eq!(gate.form_token(Some("anything")), Some(token.clone()));
    assert!(gate.form_token_matches(None, &token));
    assert!(!gate.form_token_matches(None, "guess"));
}

#[tokio::test]
async fn the_preview_gate_refuses_a_wrong_key() {
    let (gate, _) = keyed_preview_gate();
    assert!(gate.sign_in_required());
    assert_eq!(gate.admits(None).await, Admit::No("sign in first".into()));
    assert_eq!(gate.form_token(None), None);
    for wrong in ["", "guess", &PREVIEW_KEY[1..], &format!("{PREVIEW_KEY}x")] {
        assert_eq!(
            gate.sign_in(wrong),
            SignIn::Refused("the access key is wrong".into()),
            "{wrong:?}"
        );
    }
    // The right key, typed with spaces around it, starts a session.
    let id = signed_in(&gate, &format!(" {PREVIEW_KEY} "));
    assert_eq!(gate.admits(Some(&id)).await, Admit::Yes(()));
    assert_eq!(
        gate.admits(Some("never-issued")).await,
        Admit::No("the session has ended; sign in again".into())
    );
}

#[test]
fn the_preview_gate_takes_only_a_key_the_server_would() {
    let clock = Arc::new(FixedClock::new(start_time()));
    for bad in [
        "",
        "   \n",
        "short",
        "has spaces in the middle of a long key 0123",
    ] {
        let made = PreviewGate::with_key(
            bad,
            clock.clone(),
            Arc::new(SeqEntropy::new(9)),
            Limits::DEFAULT.web_session_lifetime,
        );
        assert_eq!(
            made.err().map(|e| e.name),
            Some("access_key_file"),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn a_preview_session_ends_after_its_lifetime() {
    let (gate, clock) = keyed_preview_gate();
    let id = signed_in(&gate, PREVIEW_KEY);
    let lifetime =
        Span::from_std(Limits::DEFAULT.web_session_lifetime).expect("a lifetime a span can hold");
    clock.advance(lifetime - Span::seconds(1));
    assert_eq!(gate.admits(Some(&id)).await, Admit::Yes(()));
    assert!(gate.form_token(Some(&id)).is_some());
    clock.advance(Span::seconds(1));
    assert_eq!(
        gate.admits(Some(&id)).await,
        Admit::No("the session has ended; sign in again".into())
    );
    assert_eq!(gate.form_token(Some(&id)), None);
    // Signing in again starts a fresh session.
    let again = signed_in(&gate, PREVIEW_KEY);
    assert_ne!(again, id);
    assert_eq!(gate.admits(Some(&again)).await, Admit::Yes(()));
}

#[test]
fn a_preview_form_token_matches_only_its_own_session() {
    let (gate, _) = keyed_preview_gate();
    let first = signed_in(&gate, PREVIEW_KEY);
    let second = signed_in(&gate, PREVIEW_KEY);
    assert_ne!(first, second);
    let first_token = gate.form_token(Some(&first)).expect("first token");
    let second_token = gate.form_token(Some(&second)).expect("second token");
    assert_ne!(first_token, second_token);
    assert_ne!(first_token, first, "the token is not the session ID");
    assert!(gate.form_token_matches(Some(&first), &first_token));
    assert!(gate.form_token_matches(Some(&second), &second_token));
    assert!(!gate.form_token_matches(Some(&first), &second_token));
    assert!(!gate.form_token_matches(Some(&second), &first_token));
    assert!(!gate.form_token_matches(None, &first_token));
    assert!(!gate.form_token_matches(Some("never-issued"), &first_token));
}

#[tokio::test]
async fn signing_out_of_the_preview_ends_the_session() {
    let (gate, _) = keyed_preview_gate();
    let id = signed_in(&gate, PREVIEW_KEY);
    let other = signed_in(&gate, PREVIEW_KEY);
    let token = gate.form_token(Some(&id)).expect("a token");
    gate.sign_out(&id);
    assert_eq!(
        gate.admits(Some(&id)).await,
        Admit::No("the session has ended; sign in again".into())
    );
    assert_eq!(gate.form_token(Some(&id)), None);
    assert!(!gate.form_token_matches(Some(&id), &token));
    // Only that session ends.
    assert_eq!(gate.admits(Some(&other)).await, Admit::Yes(()));
    // Signing out of a session that is gone changes nothing.
    gate.sign_out(&id);
    gate.sign_out("never-issued");
    assert_eq!(gate.admits(Some(&other)).await, Admit::Yes(()));
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
    clock.advance(Span::minutes(10));
    let after = p.overview().await.expect("overview");
    assert!(after.channels.iter().all(|c| c.recent_activity.is_empty()));
    assert_eq!(
        after.now,
        kkt_clock::to_text(start_time() + Span::minutes(10))
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
#[expect(
    clippy::disallowed_methods,
    reason = "the live feed has no clock; a short real wait shows nothing more is coming"
)]
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
                newest_agent = a.handle;
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
#[expect(
    clippy::disallowed_methods,
    reason = "bounds real waits on the running preview, so a failure ends instead of hanging"
)]
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
    clock.advance(Span::seconds(2));
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), live.next())
        .await
        .expect("a step after two seconds");
    assert!(
        matches!(first, Some(ref e) if !matches!(e, LiveEvent::Missed)),
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
async fn administration_works_through_the_preview() {
    administration_works(
        preview(1, Arc::new(FixedClock::new(start_time())), &Shutdown::new()),
        preview_gate(),
    )
    .await;
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
        p.hide_agent(agent.agent.handle).await.unwrap();
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

#[tokio::test]
async fn announcement_editor_works_in_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    announcement_editor_works(preview(1, clock, &Shutdown::new()), preview_gate()).await;
}

#[tokio::test]
async fn lanes_pages_work_in_the_preview() {
    lanes_pages_work(
        preview(1, Arc::new(FixedClock::new(start_time())), &Shutdown::new()),
        preview_gate(),
    )
    .await;
}

#[tokio::test]
async fn channel_creation_through_preview() {
    channel_creation_works(
        preview(1, Arc::new(FixedClock::new(start_time())), &Shutdown::new()),
        preview_gate(),
    )
    .await;
}

#[tokio::test]
async fn stale_agents_work_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    stale_agents_work(preview(1, clock, &Shutdown::new()), preview_gate()).await;
}

#[tokio::test]
async fn history_works_in_the_preview() {
    let clock = Arc::new(FixedClock::new(start_time()));
    history_works(
        preview(1, clock, &Shutdown::new()),
        preview_gate(),
        "6 hours",
    )
    .await;
}
