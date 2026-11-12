//! Rule tests against an in-memory database. Each row of the rules table has a test.

use super::*;
use futures::FutureExt;
use kkt_clock::testing::FixedClock;
use kkt_entropy::testing::{SameHandleEntropy, SeqEntropy};
use kkt_events::Notice;
use std::time::Duration;

use crate::scenarios::*;

#[tokio::test]
async fn registration_uses_the_selected_pack() {
    for (pack, expected) in [("animals", "amber-badger"), ("garden", "airy-acorn")] {
        let w = world_with(
            Settings {
                name_pack: kkt_names::NamePack::find(pack).unwrap(),
                ..Settings::DEFAULT
            },
            Arc::new(SameHandleEntropy(SeqEntropy::new(42))),
        )
        .await;
        let reg = w.rules.register().await.unwrap();
        assert_eq!(reg.handle, expected);
    }
}

#[tokio::test]
async fn changing_packs_keeps_existing_handles_and_credentials() {
    let mut w = world_with(
        Settings::DEFAULT,
        Arc::new(SameHandleEntropy(SeqEntropy::new(42))),
    )
    .await;
    let original = w.rules.register().await.unwrap();
    Arc::get_mut(&mut w.rules.inner)
        .unwrap()
        .deps
        .settings
        .name_pack = kkt_names::NamePack::find("garden").unwrap();
    let new = w.rules.register().await.unwrap();
    assert_eq!(new.handle, "airy-acorn");
    let existing = w.rules.authenticate(Some(&original.token)).await.unwrap();
    assert_eq!(existing.handle, original.handle);
    assert_eq!(existing.handle, "amber-badger");
}

#[tokio::test]
async fn client_metadata_is_stamped_on_register_and_successful_reclaim_only() {
    let w = world().await;
    let reg = w
        .rules
        .register_with_client(
            Some(ClientInfo {
                name: "Codex".into(),
                version: "1".into(),
            }),
            None,
            None,
        )
        .await
        .unwrap();
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    w.rules
        .set_description(&me, "worker: #152".into())
        .await
        .unwrap();
    let rows = w
        .rules
        .list_agents(&me, Some("worker: #152".into()), None, 10, None)
        .await
        .unwrap();
    assert_eq!(rows.items.len(), 1);
    let first = rows.items[0].client.clone().unwrap();
    assert_eq!(
        (first.name.as_str(), first.version.as_str(), first.run_id),
        ("Codex", "1", w.run_id)
    );
    assert_eq!(first.recorded_at, "2026-10-23T14:00:00Z");
    assert!(
        w.rules
            .reclaim_with_client(reg.handle.clone(), "wrong".into(), None, None)
            .await
            .is_err()
    );
    assert_eq!(observed(&w, &me, &reg.handle).await.client, Some(first));
    w.clock.advance(Span::seconds(60));
    let reclaimed = w
        .rules
        .reclaim_with_client(
            reg.handle.clone(),
            reg.recovery_code.clone(),
            Some(ClientInfo {
                name: "Claude Code".into(),
                version: "2".into(),
            }),
            None,
        )
        .await
        .unwrap();
    let me = w.rules.authenticate(Some(&reclaimed.token)).await.unwrap();
    let client = observed(&w, &me, &reg.handle).await.client.unwrap();
    assert_eq!(
        (client.name.as_str(), client.version.as_str(), client.run_id),
        ("Claude Code", "2", w.run_id)
    );
    assert_eq!(client.recorded_at, "2026-10-23T14:01:00Z");
    w.rules
        .reclaim(reg.handle.clone(), reg.recovery_code)
        .await
        .unwrap();
    assert_eq!(observed(&w, &me, &reg.handle).await.client, None);
}

// ---- Avatars ----

#[tokio::test]
async fn register_gives_a_name_and_a_working_token() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    assert!(reg.token.starts_with("act_"));
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    assert_eq!(me.handle, reg.handle);
}

#[tokio::test]
async fn bad_or_missing_tokens_are_unauthenticated_with_instructions() {
    let w = world().await;
    let err = w.rules.authenticate(None).await.unwrap_err();
    assert_eq!(err.code, Code::Unauthenticated);
    assert!(err.message.contains("register"));
    assert_eq!(
        code(w.rules.authenticate(Some("act_nope")).await),
        Code::Unauthenticated
    );
}

#[tokio::test]
async fn a_proposed_name_is_given_exactly_and_works() {
    let w = world().await;
    let reg = register_as(&w, "api-reviewer-2").await.unwrap();
    assert_eq!(reg.handle, "api-reviewer-2");
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    assert_eq!(me.handle, "api-reviewer-2");
    // Without one, the name is drawn as before.
    let drawn = w
        .rules
        .register_with_client(None, None, None)
        .await
        .unwrap();
    assert_eq!(drawn.handle.split('-').count(), 2, "{}", drawn.handle);
}

#[tokio::test]
async fn a_proposed_name_may_hold_capitals_and_underscores_and_is_found_exactly() {
    let w = world().await;
    let reg = register_as(&w, "COOL_PROJECT_Google_oauth_REVIEW")
        .await
        .unwrap();
    assert_eq!(reg.handle, "COOL_PROJECT_Google_oauth_REVIEW");
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    assert_eq!(me.handle, "COOL_PROJECT_Google_oauth_REVIEW");
    // Lookups by handle match exactly: another case finds no one.
    let a = w.agent().await;
    let ch = w.channel(&a, "c", false, &[]).await;
    let lower = "cool_project_google_oauth_review".to_string();
    let err = w
        .rules
        .add_to_channel(&a, ch.clone(), vec![lower])
        .await
        .expect_err("another case");
    assert_eq!(err.code, Code::NotFound, "{}", err.message);
    w.rules
        .add_to_channel(&a, ch, vec![me.handle.clone()])
        .await
        .unwrap();
}

#[tokio::test]
async fn name_limits_keep_their_numbers_and_channel_ids_their_length() {
    // The texts agents see, with the server's own limits in them.
    let w = world().await;
    let long = "a".repeat(41);
    let message = |r: Result<()>| r.expect_err("too long").message;
    assert_eq!(
        message(register_as(&w, &long).await.map(|_| ())),
        "the handle must be at most 40 characters"
    );
    assert_eq!(
        message(
            w.rules
                .register_with_client(None, None, Some(long.clone()))
                .await
                .map(|_| ())
        ),
        "the lane name must be at most 40 characters. Nothing was registered."
    );
    let a = w.agent().await;
    assert_eq!(
        message(
            w.rules
                .remember(&a, long.clone(), "x".into())
                .await
                .map(|_| ())
        ),
        "the entry name must be at most 40 characters"
    );
    assert_eq!(
        message(
            w.rules
                .save_lane(Actor::Operator(web_page()), lane(&long, "d", "t"))
                .await
                .map(|_| ())
        ),
        "the lane name must be at most 40 characters"
    );
    let id = w.channel(&a, "c", true, &[]).await;
    assert!(id.starts_with("ch_"), "{id}");
    assert_eq!(id.len(), 11, "{id}");
}

#[tokio::test]
async fn a_taken_name_gives_back_its_use_of_the_rate_limit() {
    let settings = Settings {
        registrations_per_minute: 2,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    register_as(&w, "first").await.unwrap();
    let err = register_as(&w, "first").await.expect_err("taken");
    assert_eq!(err.code, Code::HandleTaken);
    register_as(&w, "second").await.expect("the second of two");
}

#[tokio::test]
async fn descriptions_are_set_replaced_and_cleared() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    let set = w
        .rules
        .set_description(&a, "  Codex, reviews only  ".into())
        .await
        .unwrap();
    assert_eq!(set.description.as_deref(), Some("Codex, reviews only"));
    w.rules
        .set_description(&a, "Claude Code".into())
        .await
        .unwrap();
    let described = |items: &[AgentInfo]| {
        items
            .iter()
            .find(|i| i.handle == a.handle)
            .and_then(|i| i.description.clone())
    };
    // Others see it in list_agents, list_members and the web page.
    let agents = w.rules.list_agents(&b, None, None, 10, None).await.unwrap();
    assert_eq!(described(&agents.items).as_deref(), Some("Claude Code"));
    let members = w
        .rules
        .list_members(&b, ch.clone(), 10, None)
        .await
        .unwrap();
    assert_eq!(
        described(&members.page.items).as_deref(),
        Some("Claude Code")
    );
    let view = w
        .rules
        .channel_view(&web_page(), ch, None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(described(&view.members).as_deref(), Some("Claude Code"));
    let page = w
        .rules
        .agent_view(&web_page(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(page.agent.description.as_deref(), Some("Claude Code"));
    // Empty text clears it; it never touches anyone else's.
    let cleared = w.rules.set_description(&a, "   ".into()).await.unwrap();
    assert_eq!(cleared.description, None);
    let agents = w.rules.list_agents(&b, None, None, 10, None).await.unwrap();
    assert!(agents.items.iter().all(|i| i.description.is_none()));
}

#[tokio::test]
async fn descriptions_over_the_limit_are_refused() {
    let settings = Settings {
        max_description_chars: 5,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let a = w.agent().await;
    // The cap counts characters, not bytes.
    let fits = w.rules.set_description(&a, "héllo".into()).await.unwrap();
    assert_eq!(fits.description.as_deref(), Some("héllo"));
    assert_eq!(
        code(w.rules.set_description(&a, "héllo!".into()).await),
        Code::InvalidArgument
    );
    // A refused change keeps the old description.
    let me = w
        .rules
        .agent_view(&web_page(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(me.agent.description.as_deref(), Some("héllo"));
}

// ---- Rules table: appears in list_channels ----

#[tokio::test]
async fn open_channels_appear_to_everyone() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[]).await;
    assert!(w.visible_ids(&b).await.contains(&ch));
}

#[tokio::test]
async fn closed_channels_appear_to_members_only() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[&b]).await;
    assert!(w.visible_ids(&a).await.contains(&ch));
    assert!(w.visible_ids(&b).await.contains(&ch));
    assert!(!w.visible_ids(&c).await.contains(&ch));
}

// ---- Rules table: join ----

#[tokio::test]
async fn anyone_can_join_an_open_channel() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[]).await;
    let info = w.rules.join_channel(&b, ch.clone()).await.unwrap();
    assert!(info.is_member);
    assert_eq!(info.member_count, 2);
    // Joining again changes nothing.
    let again = w.rules.join_channel(&b, ch).await.unwrap();
    assert_eq!(again.member_count, 2);
}

#[tokio::test]
async fn closed_channels_can_only_be_entered_by_being_added() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[]).await;
    assert_eq!(
        code(w.rules.join_channel(&b, ch.clone()).await),
        Code::NotFound
    );
    w.rules
        .add_to_channel(&a, ch.clone(), vec![b.handle.clone()])
        .await
        .unwrap();
    assert!(w.visible_ids(&b).await.contains(&ch));
}

// ---- Rules table: add others ----

#[tokio::test]
async fn any_member_can_add_others_to_an_open_channel() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[]).await;
    // A non-member of an open channel is told to join first.
    assert_eq!(
        code(
            w.rules
                .add_to_channel(&b, ch.clone(), vec![c.handle.clone()])
                .await
        ),
        Code::NotMember
    );
    w.rules.join_channel(&b, ch.clone()).await.unwrap();
    let members = w
        .rules
        .add_to_channel(&b, ch, vec![c.handle.clone()])
        .await
        .unwrap();
    assert_eq!(members.total, 3);
    assert!(members.page.items.iter().any(|m| m.handle == c.handle));
}

#[tokio::test]
async fn any_member_can_add_others_to_a_closed_channel() {
    let w = world().await;
    let (a, b, c, d) = (
        w.agent().await,
        w.agent().await,
        w.agent().await,
        w.agent().await,
    );
    let ch = w.channel(&a, "secret", false, &[&b]).await;
    // `b` was added, not the creator, and has the same power.
    w.rules
        .add_to_channel(&b, ch.clone(), vec![c.handle.clone()])
        .await
        .unwrap();
    assert!(w.visible_ids(&c).await.contains(&ch));
    assert_eq!(
        code(w.rules.add_to_channel(&d, ch, vec![d.handle.clone()]).await),
        Code::NotFound
    );
}

#[tokio::test]
async fn adding_an_unknown_agent_is_not_found() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    assert_eq!(
        code(
            w.rules
                .add_to_channel(&a, ch, vec!["nobody-here".into()])
                .await
        ),
        Code::NotFound
    );
}

// ---- Rules table: read, send, set purpose ----

#[tokio::test]
async fn members_of_an_open_channel_can_read_send_and_set_purpose() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[]).await;

    assert_eq!(
        code(w.rules.send(&b, ch.clone(), "hi".into()).await),
        Code::NotMember
    );
    assert_eq!(code(w.read_latest(&b, &ch).await), Code::NotMember);
    assert_eq!(
        code(w.rules.set_purpose(&b, ch.clone(), "x".into()).await),
        Code::NotMember
    );

    w.rules.join_channel(&b, ch.clone()).await.unwrap();
    w.rules.send(&b, ch.clone(), "hi".into()).await.unwrap();
    assert_eq!(w.read_latest(&b, &ch).await.unwrap().messages.len(), 1);
    let info = w.rules.set_purpose(&b, ch, "talk".into()).await.unwrap();
    assert_eq!(info.purpose.as_deref(), Some("talk"));
}

#[tokio::test]
async fn members_of_a_closed_channel_can_read_send_and_set_purpose() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[&b]).await;

    w.rules.send(&b, ch.clone(), "hi".into()).await.unwrap();
    assert_eq!(w.read_latest(&b, &ch).await.unwrap().messages.len(), 1);
    w.rules
        .set_purpose(&b, ch.clone(), "plans".into())
        .await
        .unwrap();

    assert_eq!(
        code(w.rules.send(&c, ch.clone(), "hi".into()).await),
        Code::NotFound
    );
    assert_eq!(code(w.read_latest(&c, &ch).await), Code::NotFound);
    assert_eq!(
        code(w.rules.set_purpose(&c, ch, "x".into()).await),
        Code::NotFound
    );
}

// ---- The human ----

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "notices have no clock; a short real wait shows nothing more is coming"
)]
async fn the_human_changes_any_channel_and_the_operator_says_so() {
    let w = world().await;
    let mut notices = w.rules.subscribe();
    let (a, b) = (w.agent().await, w.agent().await);
    // A closed channel the human is not in, as no human is in any.
    let ch = w.channel(&a, "secret", false, &[&b]).await;

    let info = w
        .rules
        .set_purpose(
            Actor::Operator(web_page()),
            ch.clone(),
            "  release prep ".into(),
        )
        .await
        .unwrap();
    assert_eq!(info.purpose.as_deref(), Some("release prep"));
    assert!(!info.is_member);

    // The members see the change explained, from the operator.
    let page = w.read_latest(&b, &ch).await.unwrap();
    let said = page.messages.last().expect("an operator message");
    assert_eq!(said.agent, names::OPERATOR);
    assert_eq!(
        said.body,
        "The operator changed the purpose to: release prep"
    );
    assert_eq!(said.run_id, Some(w.run_id));

    w.rules
        .set_purpose(Actor::Operator(web_page()), ch.clone(), " ".into())
        .await
        .unwrap();
    let page = w.read_latest(&a, &ch).await.unwrap();
    assert_eq!(
        page.messages.last().map(|m| m.body.as_str()),
        Some("The operator cleared the purpose.")
    );

    // Waiting agents hear about it like any message.
    let mut heard = Vec::new();
    while let Ok(Some(notice)) =
        tokio::time::timeout(Duration::from_millis(50), notices.next()).await
    {
        if let Notice::Message(m) = notice {
            heard.push((m.channel_id, m.agent));
        }
    }
    assert_eq!(
        heard,
        [
            (ch.clone(), names::OPERATOR.to_string()),
            (ch.clone(), names::OPERATOR.to_string())
        ]
    );
}

#[tokio::test]
async fn the_operator_is_not_an_agent_and_no_agent_can_become_it() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    w.rules
        .set_purpose(Actor::Operator(web_page()), ch.clone(), "talk".into())
        .await
        .unwrap();
    // The operator has spoken, yet is no agent: it cannot be reclaimed,
    // added, or found, and is not listed.
    assert_eq!(
        code(
            w.rules
                .reclaim(names::OPERATOR.into(), "arc_anything".into())
                .await
        ),
        Code::Unauthenticated
    );
    assert_eq!(
        code(
            w.rules
                .add_to_channel(&a, ch.clone(), vec![names::OPERATOR.into()])
                .await
        ),
        Code::NotFound
    );
    let agents = w
        .rules
        .list_agents(&a, None, None, 100, None)
        .await
        .unwrap();
    assert!(agents.items.iter().all(|i| i.handle != names::OPERATOR));
    // Registering, even when every name draw is the same, never gives it.
    let w = world_with(
        Settings::DEFAULT,
        Arc::new(SameHandleEntropy(SeqEntropy::new(5))),
    )
    .await;
    for _ in 0..5 {
        let reg = w.rules.register().await.unwrap();
        assert_ne!(reg.handle, names::OPERATOR);
        assert!(reg.handle.contains('-'), "{}", reg.handle);
    }
}

#[tokio::test]
async fn the_human_gets_not_found_for_a_missing_channel_and_bad_input_changes_nothing() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    assert_eq!(
        code(
            w.rules
                .set_purpose(
                    Actor::Operator(web_page()),
                    "ch_missing0".into(),
                    "x".into()
                )
                .await
        ),
        Code::NotFound
    );
    let too_long = "x".repeat(Settings::DEFAULT.max_purpose_chars + 1);
    assert_eq!(
        code(
            w.rules
                .set_purpose(Actor::Operator(web_page()), ch.clone(), too_long)
                .await
        ),
        Code::InvalidArgument
    );
    // Nothing was said: a refused change posts no operator message.
    assert!(w.read_latest(&a, &ch).await.unwrap().messages.is_empty());
}

#[tokio::test]
async fn an_agent_changing_the_purpose_posts_no_operator_message() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    w.rules
        .set_purpose(&a, ch.clone(), "talk".into())
        .await
        .unwrap();
    assert!(w.read_latest(&a, &ch).await.unwrap().messages.is_empty());
}

// ---- Rules table: leave ----

#[tokio::test]
async fn any_member_can_leave_an_open_channel() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    w.rules.leave_channel(&b, ch.clone()).await.unwrap();
    assert_eq!(
        code(w.rules.leave_channel(&b, ch.clone()).await),
        Code::NotMember
    );
    // Still visible, since it is open.
    assert!(w.visible_ids(&b).await.contains(&ch));
}

#[tokio::test]
async fn any_member_can_leave_a_closed_channel_and_it_then_vanishes_for_them() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[&b]).await;
    w.rules.leave_channel(&b, ch.clone()).await.unwrap();
    assert!(!w.visible_ids(&b).await.contains(&ch));
    assert_eq!(code(w.rules.leave_channel(&b, ch).await), Code::NotFound);
}

// ---- Rules table: remove others ----

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "notices have no clock; a short real wait shows nothing more is coming"
)]
async fn the_owner_removes_a_member_of_a_closed_channel_says_so_and_it_vanishes_for_them() {
    let w = world().await;
    let mut notices = w.rules.subscribe();
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[&b, &c]).await;
    let members = w
        .rules
        .list_members(&b, ch.clone(), 20, None)
        .await
        .unwrap();
    assert_eq!(members.owner.as_deref(), Some(a.handle.as_str()));

    let members = w
        .rules
        .remove_from_channel(&a, ch.clone(), format!(" {} ", b.handle))
        .await
        .unwrap();
    assert_eq!(members.total, 2);
    assert!(members.page.items.iter().all(|m| m.handle != b.handle));

    // The remaining members see who removed whom, from the owner.
    let said = w.read_latest(&c, &ch).await.unwrap().messages;
    let said = said.last().expect("a message saying so");
    assert_eq!(said.agent, a.handle);
    assert_eq!(
        said.body,
        format!("{} removed {} from the channel.", a.handle, b.handle)
    );
    assert_eq!(said.run_id, Some(w.run_id));

    // To the removed agent, the closed channel no longer exists.
    assert!(!w.visible_ids(&b).await.contains(&ch));
    assert_eq!(code(w.read_latest(&b, &ch).await), Code::NotFound);
    assert_eq!(
        code(w.rules.list_members(&b, ch.clone(), 20, None).await),
        Code::NotFound
    );

    let mut heard = Vec::new();
    while let Ok(Some(notice)) =
        tokio::time::timeout(Duration::from_millis(50), notices.next()).await
    {
        match notice {
            Notice::Message(m) => heard.push(format!("message from {}", m.agent)),
            Notice::Left(l) => heard.push(format!("{} left {}", l.agent, l.channel_id)),
            _ => {}
        }
    }
    assert_eq!(
        heard,
        [
            format!("message from {}", a.handle),
            format!("{} left {ch}", b.handle)
        ]
    );
}

#[tokio::test]
async fn the_owner_removes_a_member_of_an_open_channel_who_may_join_again() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    w.rules
        .remove_from_channel(&a, ch.clone(), b.handle.clone())
        .await
        .unwrap();
    assert_eq!(
        code(w.rules.send(&b, ch.clone(), "hi".into()).await),
        Code::NotMember
    );
    assert!(w.visible_ids(&b).await.contains(&ch));
    w.rules.join_channel(&b, ch).await.unwrap();
}

#[tokio::test]
async fn only_the_owner_may_remove_others() {
    let w = world().await;
    let (a, b, c, d) = (
        w.agent().await,
        w.agent().await,
        w.agent().await,
        w.agent().await,
    );
    let closed = w.channel(&a, "secret", false, &[&b, &c]).await;
    let open = w.channel(&a, "general", true, &[&b, &c]).await;
    for ch in [&closed, &open] {
        // A member that does not own the channel.
        assert_eq!(
            code(
                w.rules
                    .remove_from_channel(&b, ch.clone(), c.handle.clone())
                    .await
            ),
            Code::NotOwner
        );
    }
    // Outsiders: a closed channel does not exist; an open one needs joining.
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&d, closed.clone(), c.handle.clone())
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&d, open.clone(), c.handle.clone())
                .await
        ),
        Code::NotMember
    );
    // The owner cannot remove itself, or an agent that does not exist.
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&a, closed.clone(), a.handle.clone())
                .await
        ),
        Code::InvalidArgument
    );
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&a, closed.clone(), "nobody-here".into())
                .await
        ),
        Code::NotFound
    );
    // Removing an agent that is not a member changes and says nothing.
    let members = w
        .rules
        .remove_from_channel(&a, closed.clone(), d.handle.clone())
        .await
        .unwrap();
    assert_eq!(members.total, 3);
    assert!(
        w.read_latest(&a, &closed)
            .await
            .unwrap()
            .messages
            .is_empty()
    );
    // An owner that left a closed channel is an outsider to it.
    w.rules.leave_channel(&a, closed.clone()).await.unwrap();
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&a, closed, c.handle.clone())
                .await
        ),
        Code::NotFound
    );
}

// ---- Rules table: pass the ownership on ----

#[tokio::test]
async fn the_owner_passes_the_ownership_to_a_member_and_says_so() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[&b]).await;

    // Only a member can become the owner.
    assert_eq!(
        code(
            w.rules
                .transfer_owner(&a, ch.clone(), c.handle.clone())
                .await
        ),
        Code::InvalidArgument
    );
    assert_eq!(
        code(
            w.rules
                .transfer_owner(&a, ch.clone(), "nobody-here".into())
                .await
        ),
        Code::NotFound
    );
    // Naming yourself changes and says nothing.
    let info = w
        .rules
        .transfer_owner(&a, ch.clone(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(info.owner.as_deref(), Some(a.handle.as_str()));
    assert!(w.read_latest(&a, &ch).await.unwrap().messages.is_empty());

    let info = w
        .rules
        .transfer_owner(&a, ch.clone(), b.handle.clone())
        .await
        .unwrap();
    assert_eq!(info.owner.as_deref(), Some(b.handle.as_str()));
    assert_eq!(info.created_by, a.handle);
    let said = w.read_latest(&b, &ch).await.unwrap().messages;
    let said = said.last().expect("a message saying so");
    assert_eq!(said.agent, a.handle);
    assert_eq!(
        said.body,
        format!("{} made {} the owner of the channel.", a.handle, b.handle)
    );

    // The old owner is now an ordinary member; the new one has the powers.
    assert_eq!(
        code(
            w.rules
                .transfer_owner(&a, ch.clone(), a.handle.clone())
                .await
        ),
        Code::NotOwner
    );
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&a, ch.clone(), b.handle.clone())
                .await
        ),
        Code::NotOwner
    );
    w.rules
        .remove_from_channel(&b, ch.clone(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(code(w.read_latest(&a, &ch).await), Code::NotFound);
}

#[tokio::test]
async fn only_the_owner_may_pass_the_ownership_on() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let closed = w.channel(&a, "secret", false, &[&b]).await;
    let open = w.channel(&a, "general", true, &[&b]).await;
    for ch in [&closed, &open] {
        assert_eq!(
            code(
                w.rules
                    .transfer_owner(&b, ch.clone(), b.handle.clone())
                    .await
            ),
            Code::NotOwner
        );
    }
    assert_eq!(
        code(w.rules.transfer_owner(&c, closed, c.handle.clone()).await),
        Code::NotFound
    );
    assert_eq!(
        code(w.rules.transfer_owner(&c, open, c.handle.clone()).await),
        Code::NotMember
    );
}

// ---- Rules table: hide the channel ----

#[tokio::test]
async fn the_owner_hides_its_channel_says_so_and_it_vanishes_for_every_agent() {
    for open in [false, true] {
        let w = world().await;
        let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
        let ch = w.channel(&a, "done", open, &[&b]).await;
        let sent = w
            .rules
            .send(&b, ch.clone(), "finished work".into())
            .await
            .unwrap();
        let mut notices = w.rules.subscribe();

        w.rules.hide_channel(&a, ch.clone()).await.unwrap();

        // The owner's announcement is stored with the channel's messages.
        let stored_id = ch.clone();
        let stored = w
            .rules
            .unbound_db()
            .read(move |s| {
                s.channel_messages(
                    &SwarmId::main(),
                    &stored_id,
                    None,
                    MessageRange::Latest,
                    100,
                )
            })
            .await
            .unwrap();
        assert_eq!(stored.len(), 2);
        let said = stored.iter().find(|m| m.id != sent.id).unwrap();
        assert_eq!(said.agent, a.handle);
        assert_eq!(
            said.body,
            format!("{} hid the channel. Its messages are kept.", a.handle)
        );
        assert_eq!(said.run_id, Some(w.run_id));
        match notices.next().now_or_never() {
            Some(Some(Notice::Message(m))) => assert_eq!(m.agent, a.handle),
            _ => panic!("expected the owner's message notice"),
        }

        // To the owner, the former member and an outsider alike, it is gone.
        for x in [&a, &b, &c] {
            assert!(!w.visible_ids(x).await.contains(&ch));
            assert_eq!(code(w.read_latest(x, &ch).await), Code::NotFound);
            assert_eq!(code(w.rules.get_message(x, sent.id).await), Code::NotFound);
            assert_eq!(code(w.rules.get_note(x, ch.clone()).await), Code::NotFound);
            assert_eq!(
                code(w.rules.join_channel(x, ch.clone()).await),
                Code::NotFound
            );
            assert_eq!(
                code(w.rules.send(x, ch.clone(), "later".into()).await),
                Code::NotFound
            );
            assert_eq!(
                code(w.rules.list_members(x, ch.clone(), 20, None).await),
                Code::NotFound
            );
            assert_eq!(
                code(w.rules.hide_channel(x, ch.clone()).await),
                Code::NotFound
            );
            assert!(w.rules.activity(x, 0).await.unwrap().is_empty());
        }
        assert!(
            w.rules
                .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
                .await
                .unwrap()
                .channels
                .is_empty()
        );
    }
}

#[tokio::test]
async fn only_the_owner_may_hide_a_channel() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let closed = w.channel(&a, "secret", false, &[&b]).await;
    let open = w.channel(&a, "general", true, &[&b]).await;
    for ch in [&closed, &open] {
        assert_eq!(
            code(w.rules.hide_channel(&b, ch.clone()).await),
            Code::NotOwner
        );
    }
    assert_eq!(
        code(w.rules.hide_channel(&c, closed.clone()).await),
        Code::NotFound
    );
    assert_eq!(
        code(w.rules.hide_channel(&c, open.clone()).await),
        Code::NotMember
    );
    // An owner that left is an outsider to the channel.
    w.rules.leave_channel(&a, closed.clone()).await.unwrap();
    assert_eq!(
        code(w.rules.hide_channel(&a, closed.clone()).await),
        Code::NotFound
    );
    // Refusals change nothing.
    for ch in [&closed, &open] {
        assert!(w.visible_ids(&b).await.contains(ch));
        let said = w.read_latest(&b, ch).await.unwrap().messages;
        assert!(said.iter().all(|m| !m.body.contains("hid the channel")));
    }
}

#[tokio::test]
async fn an_owner_that_left_and_then_unregisters_leaves_an_explanation() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let left = w.channel(&a, "left", false, &[&b]).await;
    let removed = w.channel(&a, "removed", false, &[&b]).await;
    w.rules.leave_channel(&a, left.clone()).await.unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            removed.clone(),
            ChannelChange::RemoveMember(a.handle.clone()),
        )
        .await
        .unwrap();

    w.rules.unregister(&a).await.unwrap();

    for (ch, owner, what) in [(&left, &a, "unregistered"), (&removed, &a, "unregistered")] {
        let members = w
            .rules
            .list_members(&b, ch.clone(), 20, None)
            .await
            .unwrap();
        assert_eq!(members.owner, None);
        let said = w.read_latest(&b, ch).await.unwrap().messages;
        let said = said.last().expect("an explanation");
        assert_eq!(said.agent, names::OPERATOR);
        assert_eq!(
            said.body,
            format!(
                "{} owned the channel and {what}. The channel no longer has an owner.",
                owner.handle
            )
        );
    }
}

#[tokio::test]
async fn the_human_cannot_hide_a_channel_owner_by_either_route() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let room = w.channel(&a, "room", false, &[&b]).await;
    // An owner that has left its channel still owns it.
    let left = w.channel(&c, "left", true, &[&b]).await;
    w.rules.leave_channel(&c, left.clone()).await.unwrap();
    w.clock.advance(Span::hours(3));

    for owner in [&a, &c] {
        assert_eq!(
            code(
                w.rules
                    .hide_agent(Actor::Operator(web_page()), owner.handle.clone())
                    .await
            ),
            Code::InvalidArgument
        );
    }
    let outcome = w
        .rules
        .hide_stale_agents(
            Actor::Operator(web_page()),
            2 * HOUR,
            vec![a.handle.clone(), c.handle.clone()],
        )
        .await
        .unwrap();
    assert!(outcome.hidden.is_empty());
    assert!(outcome.kept.iter().all(|k| k.reason == KeptReason::Owner));
    assert_eq!(outcome.kept.len(), 2);

    // Nothing changed: both still own their channels and can still call.
    for (ch, owner) in [(&room, &a), (&left, &c)] {
        let view = w
            .rules
            .channel_view(&web_page(), ch.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap();
        assert_eq!(view.channel.owner.as_deref(), Some(owner.handle.as_str()));
        assert!(view.messages.iter().all(|m| m.agent != names::OPERATOR));
        w.rules
            .agent_view(&web_page(), owner.handle.clone())
            .await
            .unwrap();
    }
    w.read_latest(&a, &room).await.unwrap();
}

#[tokio::test]
async fn the_human_passes_or_clears_the_ownership_and_can_then_hide() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let passed = w.channel(&a, "passed", false, &[&b]).await;
    let cleared = w.channel(&a, "cleared", true, &[&b]).await;
    let stale_owned = w.channel(&b, "b's", false, &[&a]).await;

    // Only the human may, only to a member, and only an agent that exists.
    assert_eq!(
        code(
            w.rules
                .change_channel(
                    Actor::from(&b),
                    passed.clone(),
                    ChannelChange::SetOwner(Some(b.handle.clone()))
                )
                .await
        ),
        Code::OperatorOnly
    );
    assert_eq!(
        code(
            w.rules
                .change_channel(
                    Actor::Operator(web_page()),
                    passed.clone(),
                    ChannelChange::SetOwner(Some(c.handle.clone()))
                )
                .await
        ),
        Code::InvalidArgument
    );
    assert_eq!(
        code(
            w.rules
                .change_channel(
                    Actor::Operator(web_page()),
                    passed.clone(),
                    ChannelChange::SetOwner(Some("nobody-here".into()))
                )
                .await
        ),
        Code::NotFound
    );
    // Naming the current owner changes nothing and says nothing.
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            passed.clone(),
            ChannelChange::SetOwner(Some(a.handle.clone())),
        )
        .await
        .unwrap();
    let view = w
        .rules
        .channel_view(&web_page(), passed.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert!(view.messages.iter().all(|m| m.agent != names::OPERATOR));

    let mut live = w.rules.subscribe();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            passed.clone(),
            ChannelChange::SetOwner(Some(b.handle.clone())),
        )
        .await
        .unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            cleared.clone(),
            ChannelChange::SetOwner(None),
        )
        .await
        .unwrap();
    // Clearing a channel with no owner changes nothing and says nothing.
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            cleared.clone(),
            ChannelChange::SetOwner(None),
        )
        .await
        .unwrap();
    for (ch, owner, said) in [
        (
            &passed,
            Some(b.handle.as_str()),
            format!("The operator made {} the owner of the channel.", b.handle),
        ),
        (
            &cleared,
            None,
            format!(
                "The operator removed {} as the owner of the channel. \
                 The channel no longer has an owner.",
                a.handle
            ),
        ),
    ] {
        let view = w
            .rules
            .channel_view(&web_page(), ch.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap();
        assert_eq!(view.channel.owner.as_deref(), owner);
        let from_operator: Vec<_> = view
            .messages
            .iter()
            .filter(|m| m.agent == names::OPERATOR)
            .collect();
        assert_eq!(from_operator.len(), 1);
        assert_eq!(from_operator[0].body, said);
    }
    let mut published = Vec::new();
    while let Some(Some(Notice::Message(m))) = live.next().now_or_never() {
        published.push(m.channel_id);
    }
    assert_eq!(published, [passed.clone(), cleared.clone()]);
    // The new owner has the owner's powers; the old one does not.
    assert_eq!(
        code(
            w.rules
                .transfer_owner(&a, passed.clone(), a.handle.clone())
                .await
        ),
        Code::NotOwner
    );
    w.rules
        .transfer_owner(&b, passed.clone(), b.handle.clone())
        .await
        .unwrap();

    // a owns nothing now, so the human may hide it.
    w.rules
        .hide_agent(Actor::Operator(web_page()), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(
        code(w.rules.agent_view(&web_page(), a.handle.clone()).await),
        Code::NotFound
    );

    // b owns two channels; once both are cleared, it is hidden as stale.
    for ch in [&passed, &stale_owned] {
        w.rules
            .change_channel(
                Actor::Operator(web_page()),
                ch.clone(),
                ChannelChange::SetOwner(None),
            )
            .await
            .unwrap();
    }
    w.clock.advance(Span::hours(3));
    let outcome = w
        .rules
        .hide_stale_agents(
            Actor::Operator(web_page()),
            2 * HOUR,
            vec![b.handle.clone()],
        )
        .await
        .unwrap();
    assert_eq!(outcome.hidden, std::slice::from_ref(&b.handle));
}

#[tokio::test]
async fn channels_and_note_edits_record_the_server_run() {
    let w = world().await;
    let a = w.agent().await;
    let db = w.rules.unbound_db().clone();
    let stored = |id: String| {
        let db = db.clone();
        async move {
            db.read(move |s| s.any_channel(&SwarmId::main(), &id))
                .await
                .unwrap()
                .unwrap()
        }
    };
    let by_agent = w.channel(&a, "plans", true, &[]).await;
    let by_operator = w
        .rules
        .create_channel_as_operator(
            Actor::Operator(web_page()),
            NewChannel {
                name: "help".into(),
                open: true,
                purpose: None,
                members: vec![],
            },
        )
        .await
        .unwrap()
        .id;
    let made = stored(by_agent.clone()).await;
    assert_eq!((made.run_id, made.note_run_id), (Some(w.run_id), None));
    let made = stored(by_operator).await;
    assert_eq!((made.run_id, made.note_run_id), (Some(w.run_id), None));
    w.rules
        .set_note(&a, by_agent.clone(), "a note".into())
        .await
        .unwrap();
    assert_eq!(stored(by_agent).await.note_run_id, Some(w.run_id));
}

#[tokio::test]
async fn a_channel_the_operator_creates_has_no_owner() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let created = w
        .rules
        .create_channel_as_operator(
            Actor::Operator(web_page()),
            NewChannel {
                name: "help".into(),
                open: false,
                purpose: None,
                members: vec![a.handle.clone(), b.handle.clone()],
            },
        )
        .await
        .unwrap();
    assert_eq!(created.owner, None);
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&a, created.id.clone(), b.handle.clone())
                .await
        ),
        Code::NotOwner
    );
    assert_eq!(
        code(
            w.rules
                .transfer_owner(&a, created.id.clone(), a.handle.clone())
                .await
        ),
        Code::NotOwner
    );
    // Only the operator can hide it.
    assert_eq!(
        code(w.rules.hide_channel(&a, created.id.clone()).await),
        Code::NotOwner
    );
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            created.id.clone(),
            ChannelChange::Hide,
        )
        .await
        .unwrap();
    assert_eq!(code(w.read_latest(&a, &created.id).await), Code::NotFound);
}

#[tokio::test]
async fn a_member_that_is_not_the_owner_keeps_every_power_it_had() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[&b]).await;
    w.rules
        .add_to_channel(&b, ch.clone(), vec![c.handle.clone()])
        .await
        .unwrap();
    w.rules
        .set_purpose(&b, ch.clone(), "plans".into())
        .await
        .unwrap();
    w.rules
        .set_note(&b, ch.clone(), "a note".into())
        .await
        .unwrap();
    w.rules.send(&b, ch.clone(), "hi".into()).await.unwrap();
    w.rules.leave_channel(&b, ch).await.unwrap();
}

#[tokio::test]
async fn a_closed_channel_with_no_members_is_unreachable_but_the_human_sees_it() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[]).await;
    w.rules
        .send(&a, ch.clone(), "last words".into())
        .await
        .unwrap();
    w.rules.leave_channel(&a, ch.clone()).await.unwrap();
    for who in [&a, &b] {
        assert_eq!(
            code(w.rules.join_channel(who, ch.clone()).await),
            Code::NotFound
        );
    }
    let view = w
        .rules
        .channel_view(&web_page(), ch, None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.messages.len(), 1);
    assert!(view.members.is_empty());
}

// ---- Closed channels do not exist to outsiders ----

#[tokio::test]
async fn hidden_and_missing_channels_look_the_same() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[]).await;
    let msg = w.rules.send(&a, ch.clone(), "psst".into()).await.unwrap();
    let missing = "ch_zzzzzzzz".to_string();

    for id in [ch.clone(), missing.clone()] {
        let hidden = w
            .rules
            .list_members(&b, id.clone(), 10, None)
            .await
            .unwrap_err();
        assert_eq!(hidden.code, Code::NotFound);
        assert_eq!(hidden.message, format!("no channel {id}"));
        assert_eq!(
            code(w.rules.join_channel(&b, id.clone()).await),
            Code::NotFound
        );
        assert_eq!(
            code(w.rules.leave_channel(&b, id.clone()).await),
            Code::NotFound
        );
        assert_eq!(
            code(w.rules.send(&b, id.clone(), "x".into()).await),
            Code::NotFound
        );
        assert_eq!(code(w.read_latest(&b, &id).await), Code::NotFound);
        assert_eq!(
            code(
                w.rules
                    .add_to_channel(&b, id.clone(), vec![b.handle.clone()])
                    .await
            ),
            Code::NotFound
        );
        assert_eq!(
            code(w.rules.set_purpose(&b, id.clone(), "x".into()).await),
            Code::NotFound
        );
        let search = SearchQuery {
            channel_id: Some(id.clone()),
            ..words("psst")
        };
        assert_eq!(code(w.rules.search(&b, search).await), Code::NotFound);
        let wait = WaitQuery {
            after_id: 0,
            channel_ids: Some(vec![id.clone()]),
            timeout: Some(Duration::from_millis(1)),
            limit: 10,
        };
        assert_eq!(
            code(w.rules.wait_for_messages(&b, wait).await),
            Code::NotFound
        );
    }
    let hidden_msg = w.rules.get_message(&b, msg.id).await.unwrap_err();
    let missing_msg = w.rules.get_message(&b, 999_999).await.unwrap_err();
    assert_eq!(hidden_msg.code, Code::NotFound);
    assert_eq!(missing_msg.code, Code::NotFound);
}

#[tokio::test]
async fn channel_names_can_repeat() {
    let w = world().await;
    let a = w.agent().await;
    let one = w.channel(&a, "general", true, &[]).await;
    let two = w.channel(&a, "general", false, &[]).await;
    assert_ne!(one, two);
}

#[tokio::test]
async fn members_can_read_history_from_before_they_joined() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "secret", false, &[]).await;
    w.rules.send(&a, ch.clone(), "before".into()).await.unwrap();
    w.rules
        .add_to_channel(&a, ch.clone(), vec![b.handle.clone()])
        .await
        .unwrap();
    let page = w.read_latest(&b, &ch).await.unwrap();
    assert_eq!(page.messages[0].body, "before");
}

// ---- Messages ----

#[tokio::test]
async fn send_checks_body() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    assert_eq!(
        code(w.rules.send(&a, ch.clone(), "x".repeat(8 * 1024 + 1)).await),
        Code::BodyTooLong
    );
    let reg = w.rules.register().await.unwrap();
    assert_eq!(
        code(
            w.rules
                .send(&a, ch.clone(), format!("token: {}", reg.token))
                .await
        ),
        Code::ContainsSecret
    );
    assert_eq!(
        code(w.rules.send(&a, ch, "  ".into()).await),
        Code::InvalidArgument
    );
}

#[tokio::test]
async fn send_is_rate_limited() {
    let settings = Settings {
        sends_per_minute: 2,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(1))).await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    w.rules.send(&a, ch.clone(), "1".into()).await.unwrap();
    w.rules.send(&a, ch.clone(), "2".into()).await.unwrap();
    assert_eq!(
        code(w.rules.send(&a, ch.clone(), "3".into()).await),
        Code::RateLimited
    );
    w.clock.advance(Span::seconds(30));
    w.rules.send(&a, ch, "4".into()).await.unwrap();
}

#[tokio::test]
async fn get_message_needs_membership() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[]).await;
    let msg = w.rules.send(&a, ch, "hello".into()).await.unwrap();
    assert_eq!(w.rules.get_message(&a, msg.id).await.unwrap(), msg);
    assert_eq!(code(w.rules.get_message(&b, msg.id).await), Code::NotMember);
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "lets the spawned wait start, then bounds the real wait for it to wake"
)]
async fn wait_wakes_when_a_message_arrives() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    let waiter = {
        let rules = w.rules.clone();
        let b = b.clone();
        tokio::spawn(async move {
            rules
                .wait_for_messages(
                    &b,
                    WaitQuery {
                        after_id: 0,
                        channel_ids: None,
                        timeout: Some(Duration::from_secs(10)),
                        limit: 10,
                    },
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    w.rules.send(&a, ch, "ping".into()).await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("waiter should wake")
        .expect("join")
        .expect("wait");
    assert_eq!(got.messages[0].body, "ping");
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "bounds the real wait for the spawned wait to end, so a failure ends instead of hanging"
)]
async fn wait_ignores_channels_i_am_not_in_and_times_out() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[]).await;
    let waiter = {
        let rules = w.rules.clone();
        let b = b.clone();
        tokio::spawn(async move {
            rules
                .wait_for_messages(
                    &b,
                    WaitQuery {
                        after_id: 0,
                        channel_ids: None,
                        timeout: Some(Duration::from_secs(30)),
                        limit: 10,
                    },
                )
                .await
        })
    };
    until_sleeping(&w.clock).await;
    w.rules.send(&a, ch, "not for b".into()).await.unwrap();
    w.clock.advance(Span::seconds(30));
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the wait should time out")
        .expect("join")
        .expect("wait");
    assert!(got.messages.is_empty());
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "notices have no clock; a short real wait shows nothing more is coming"
)]
async fn new_agents_channels_joins_and_leaves_are_published() {
    let w = world().await;
    let mut notices = w.rules.subscribe();
    let a = w.agent().await;
    let b = w.agent().await;
    let c = w.agent().await;
    let open = w.channel(&a, "open-room", true, &[]).await;
    let closed = w.channel(&a, "closed-room", false, &[]).await;
    w.rules.join_channel(&b, open.clone()).await.unwrap();
    // Joining again, or being in a channel from its creation, is not news.
    w.rules.join_channel(&b, open.clone()).await.unwrap();
    w.rules.join_channel(&a, open.clone()).await.unwrap();
    // Leaving and coming back: each event carries the count afterwards.
    w.rules.leave_channel(&b, open.clone()).await.unwrap();
    w.rules.join_channel(&b, open.clone()).await.unwrap();
    // Being added by a member is joining; adding a member again is not.
    w.rules
        .add_to_channel(&a, open.clone(), vec![c.handle.clone(), b.handle.clone()])
        .await
        .unwrap();

    let mut heard = Vec::new();
    while let Ok(Some(notice)) =
        tokio::time::timeout(Duration::from_millis(50), notices.next()).await
    {
        heard.push(match notice {
            Notice::Agent(x) => format!("agent {}", x.handle),
            Notice::Channel(x) => format!(
                "channel {} {} {} {} {}",
                x.id, x.name, x.open, x.created_by, x.member_count
            ),
            Notice::Joined(x) => format!(
                "joined {} {} {} {}",
                x.agent, x.channel_id, x.channel_name, x.member_count
            ),
            Notice::Left(x) => format!(
                "left {} {} {} {}",
                x.agent, x.channel_id, x.channel_name, x.member_count
            ),
            other => format!("{other:?}"),
        });
    }
    assert_eq!(
        heard,
        [
            format!("agent {}", a.handle),
            format!("agent {}", b.handle),
            format!("agent {}", c.handle),
            format!("channel {open} open-room true {} 1", a.handle),
            format!("channel {closed} closed-room false {} 1", a.handle),
            format!("joined {} {open} open-room 2", b.handle),
            format!("left {} {open} open-room 1", b.handle),
            format!("joined {} {open} open-room 2", b.handle),
            format!("joined {} {open} open-room 3", c.handle),
        ]
    );
}

#[tokio::test]
async fn leaving_and_removal_say_who_did_it() {
    let w = world().await;
    let (a, b, c, d) = (
        w.agent().await,
        w.agent().await,
        w.agent().await,
        w.agent().await,
    );
    let ch = w.channel(&a, "room", true, &[&b, &c, &d]).await;
    let mut notices = w.rules.subscribe();
    w.rules.leave_channel(&b, ch.clone()).await.unwrap();
    w.rules
        .remove_from_channel(&a, ch.clone(), c.handle.clone())
        .await
        .unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            ch.clone(),
            ChannelChange::RemoveMember(d.handle.clone()),
        )
        .await
        .unwrap();
    // Removing a non-member changes nothing and raises nothing.
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            ch.clone(),
            ChannelChange::RemoveMember(d.handle.clone()),
        )
        .await
        .unwrap();
    let op = names::OPERATOR;
    assert_eq!(
        heard_changes(&mut notices).await,
        [
            format!("left {} {ch} room 3 by {} false", b.handle, b.handle),
            format!("left {} {ch} room 2 by {} false", c.handle, a.handle),
            format!("left {} {ch} room 1 by {op} true", d.handle),
        ]
    );
}

#[tokio::test]
async fn the_operator_adding_a_member_publishes_joined() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let ch = w.channel(&a, "room", false, &[]).await;
    let mut notices = w.rules.subscribe();
    // The human and an admin agent each add one; adding a member again
    // changes nothing and raises nothing.
    for (actor, agent) in [
        (Actor::Operator(web_page()), &b),
        (sentinel(), &c),
        (Actor::Operator(web_page()), &c),
    ] {
        w.rules
            .change_channel(
                actor,
                ch.clone(),
                ChannelChange::AddMember(agent.handle.clone()),
            )
            .await
            .unwrap();
    }
    let mut joined = Vec::new();
    while let Some(Some(notice)) = notices.next().now_or_never() {
        if let Notice::Joined(x) = notice {
            joined.push(format!(
                "joined {} {} {} {}",
                x.agent, x.channel_id, x.channel_name, x.member_count
            ));
        }
    }
    assert_eq!(
        joined,
        [
            format!("joined {} {ch} room 2", b.handle),
            format!("joined {} {ch} room 3", c.handle),
        ]
    );
}

#[tokio::test]
async fn hiding_a_channel_says_who_did_it() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let mine = w.channel(&a, "mine", false, &[&b]).await;
    let other = w.channel(&a, "other", true, &[&b]).await;
    let mut notices = w.rules.subscribe();
    w.rules.hide_channel(&a, mine.clone()).await.unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            other.clone(),
            ChannelChange::Hide,
        )
        .await
        .unwrap();
    // A refused hide raises nothing.
    assert_eq!(
        code(w.rules.hide_channel(&b, mine.clone()).await),
        Code::NotFound
    );
    let op = names::OPERATOR;
    assert_eq!(
        heard_changes(&mut notices).await,
        [
            format!("channel_hidden {mine} mine by {} false", a.handle),
            format!("channel_hidden {other} other by {op} true"),
        ]
    );
}

#[tokio::test]
async fn unregistering_and_hiding_an_agent_say_who_did_it_and_where() {
    let w = world().await;
    let (a, b, c, d) = (
        w.agent().await,
        w.agent().await,
        w.agent().await,
        w.agent().await,
    );
    let one = w.channel(&a, "one", true, &[&b, &c, &d]).await;
    let two = w.channel(&a, "two", false, &[&b]).await;
    let mut notices = w.rules.subscribe();
    w.rules.unregister(&b).await.unwrap();
    w.rules
        .hide_agent(Actor::Operator(web_page()), c.handle.clone())
        .await
        .unwrap();
    w.clock.advance(Span::hours(3));
    w.rules
        .hide_stale_agents(
            Actor::Operator(web_page()),
            2 * HOUR,
            vec![d.handle.clone()],
        )
        .await
        .unwrap();
    // A refused hide (an owner) raises nothing.
    assert_eq!(
        code(
            w.rules
                .hide_agent(Actor::Operator(web_page()), a.handle.clone())
                .await
        ),
        Code::InvalidArgument
    );
    let op = names::OPERATOR;
    let mut heard = heard_changes(&mut notices).await;
    // The order of an agent's channels is the store's; compare them sorted.
    for line in &mut heard {
        if let Some((head, list)) = line.split_once('[') {
            let mut parts: Vec<&str> = list.trim_end_matches(']').split(' ').collect();
            parts.sort();
            *line = format!("{head}[{}]", parts.join(" "));
        }
    }
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v.join(" ")
    };
    assert_eq!(
        heard,
        [
            format!(
                "agent_hidden {} by {} false [{}]",
                b.handle,
                b.handle,
                sorted(vec![format!("{one}=one=3"), format!("{two}=two=1")])
            ),
            format!("agent_hidden {} by {op} true [{one}=one=2]", c.handle),
            format!("agent_hidden {} by {op} true [{one}=one=1]", d.handle),
        ]
    );
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "bounds the real wait for the spawned wait to end, so a failure ends instead of hanging"
)]
async fn a_waiting_agent_is_not_woken_by_new_agents_or_channels() {
    let w = world().await;
    let a = w.agent().await;
    let waiter = {
        let rules = w.rules.clone();
        let a = a.clone();
        tokio::spawn(async move {
            rules
                .wait_for_messages(
                    &a,
                    WaitQuery {
                        after_id: 0,
                        channel_ids: None,
                        timeout: Some(Duration::from_secs(30)),
                        limit: 10,
                    },
                )
                .await
        })
    };
    until_sleeping(&w.clock).await;
    let b = w.agent().await;
    let ch = w.channel(&b, "elsewhere", true, &[]).await;
    w.rules.join_channel(&a, ch).await.unwrap();
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "the wait ended without a message");
    w.clock.advance(Span::seconds(30));
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the wait should time out")
        .expect("join")
        .expect("wait");
    assert!(got.messages.is_empty());
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a short real pause shows the spawned wait has not ended; a longer one bounds it"
)]
async fn wait_times_out_by_the_injected_clock() {
    let w = world().await;
    let b = w.agent().await;
    let waiter = {
        let rules = w.rules.clone();
        tokio::spawn(async move {
            rules
                .wait_for_messages(
                    &b,
                    WaitQuery {
                        after_id: 0,
                        channel_ids: None,
                        timeout: Some(Duration::from_secs(30)),
                        limit: 10,
                    },
                )
                .await
        })
    };
    until_sleeping(&w.clock).await;
    w.clock.advance(Span::seconds(29));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiter.is_finished(), "the wait ended before its deadline");
    w.clock.advance(Span::seconds(1));
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the wait should time out")
        .expect("join")
        .expect("wait");
    assert!(got.messages.is_empty());
}

#[tokio::test]
async fn create_channel_with_members_and_unknown_member() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let info = w
        .rules
        .create_channel(
            &a,
            NewChannel {
                name: "dm".into(),
                open: false,
                purpose: Some("just us".into()),
                members: vec![b.handle.clone(), a.handle.clone()],
            },
        )
        .await
        .unwrap();
    assert_eq!(info.member_count, 2);
    assert!(!info.open);
    assert!(info.id.starts_with("ch_"));
    let bad = w
        .rules
        .create_channel(
            &a,
            NewChannel {
                name: "x".into(),
                open: true,
                purpose: None,
                members: vec!["ghost".into()],
            },
        )
        .await;
    assert_eq!(code(bad), Code::NotFound);
}

// ---- Provenance ----

#[tokio::test]
async fn new_agents_and_messages_record_the_server_run() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    let sent = w.rules.send(&a, ch, "hello".into()).await.unwrap();
    assert_eq!(sent.run_id, Some(w.run_id));

    let overview = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    assert_eq!(overview.agents[0].info.run_id, Some(w.run_id));
    let current = overview.current_run.expect("the current run");
    assert_eq!((current.id, current.version.as_str()), (w.run_id, TEST_RUN));
    let runs: Vec<(i64, String)> = w
        .rules
        .server_runs(&web_page())
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.id, r.version))
        .collect();
    assert_eq!(runs, [(w.run_id, TEST_RUN.to_string())]);
}

// ---- registration caps and revocation ----

#[tokio::test]
async fn registration_is_rate_limited_across_the_server() {
    let settings = Settings {
        registrations_per_minute: 2,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    w.rules.register().await.expect("first");
    w.rules.register().await.expect("second");
    let err = w.rules.register().await.expect_err("third");
    assert_eq!(err.code, Code::RateLimited);
    w.clock.advance(Span::seconds(30));
    w.rules.register().await.expect("refilled");
}

#[tokio::test]
async fn registration_stops_at_the_agent_cap_and_revoking_frees_room() {
    let settings = Settings {
        max_agents: 2,
        registrations_per_minute: 3,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let a = w.agent().await;
    w.agent().await;
    let err = w.rules.register().await.expect_err("over the cap");
    assert_eq!(err.code, Code::LimitReached);
    w.operator.revoke(a.handle.clone()).await.expect("revoke");
    // The refused registration gave back its use of the rate limit, so this
    // is the third of three.
    w.rules.register().await.expect("room after revoking");
}

#[tokio::test]
async fn a_revoked_token_is_refused_like_an_unknown_one() {
    let w = world().await;
    let reg = w.rules.register().await.expect("register");
    let me = w.rules.authenticate(Some(&reg.token)).await.expect("auth");
    let other = w.agent().await;
    let ch = w.channel(&me, "general", true, &[&other]).await;
    w.rules
        .send(&me, ch.clone(), "before".into())
        .await
        .expect("send");

    let revoked = w.operator.revoke(me.handle.clone()).await.expect("revoke");
    assert_eq!(
        revoked,
        Revoked::Now {
            at: "2026-10-23T14:00:00Z".into()
        }
    );
    let refused = w
        .rules
        .authenticate(Some(&reg.token))
        .await
        .expect_err("revoked");
    let unknown = w
        .rules
        .authenticate(Some("act_00000000000000000000000000"))
        .await
        .expect_err("unknown");
    assert_eq!(refused, unknown);

    // The agent's messages and membership stay for everyone else.
    let page = w
        .rules
        .read(
            &other,
            ReadQuery {
                channel_id: ch.clone(),
                position: Position::Latest,
                from_agent: None,
                limit: 10,
            },
        )
        .await
        .expect("read");
    assert_eq!(page.messages[0].agent, me.handle);
}

#[tokio::test]
async fn revoking_twice_or_an_unknown_agent_is_reported() {
    let w = world().await;
    let a = w.agent().await;
    w.operator.revoke(a.handle.clone()).await.expect("revoke");
    w.clock.advance(Span::minutes(5));
    assert_eq!(
        w.operator.revoke(a.handle.clone()).await.expect("again"),
        Revoked::Already {
            at: "2026-10-23T14:00:00Z".into()
        }
    );
    let err = w
        .operator
        .revoke("no-such-agent".into())
        .await
        .expect_err("unknown");
    assert_eq!(err.code, Code::NotFound);
}

#[tokio::test]
async fn reclaim_rejects_wrong_credentials_and_revoked_avatars_identically() {
    let w = world().await;
    let a = w.rules.register().await.unwrap();
    let b = w.rules.register().await.unwrap();
    let mut errors = vec![];
    for (name, recovery) in [
        (a.handle.clone(), b.recovery_code.clone()),
        ("missing-agent".into(), a.recovery_code.clone()),
        (a.handle.clone(), String::new()),
        (a.handle.clone(), a.token.clone()),
    ] {
        errors.push(w.rules.reclaim(name, recovery).await.unwrap_err());
    }
    // Failed attempts leave the current token usable.
    w.rules.authenticate(Some(&a.token)).await.unwrap();
    w.operator.revoke(a.handle.clone()).await.unwrap();
    errors.push(
        w.rules
            .reclaim(a.handle, a.recovery_code)
            .await
            .unwrap_err(),
    );
    for error in errors {
        assert_eq!(error.code, Code::Unauthenticated);
        assert_eq!(
            error.message,
            "unknown handle or recovery code, or avatar unavailable"
        );
    }
}

#[tokio::test]
async fn recovery_codes_are_hashed_and_rejected_in_messages() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    let channel = w.channel(&me, "general", true, &[]).await;
    let name = reg.handle.clone();
    let recovery = reg.recovery_code.clone();
    assert!(
        w.rules
            .unbound_db()
            .read(move |r| r.agent_by_recovery_hash(&SwarmId::main(), &name, recovery.as_bytes()))
            .await
            .unwrap()
            .is_none()
    );
    let name = reg.handle.clone();
    let hash = auth::hash_token(&reg.recovery_code);
    assert!(
        w.rules
            .unbound_db()
            .read(move |r| r.agent_by_recovery_hash(&SwarmId::main(), &name, &hash))
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        code(w.rules.authenticate(Some(&reg.recovery_code)).await),
        Code::Unauthenticated
    );
    let err = w
        .rules
        .send(&me, channel, format!("recovery: {}", reg.recovery_code))
        .await
        .unwrap_err();
    assert_eq!(err.code, Code::ContainsSecret);
    assert!(!err.message.contains(&reg.recovery_code));
}

#[tokio::test]
async fn legacy_agents_keep_their_token_but_have_no_recovery_code() {
    let w = world().await;
    let run_id = w.run_id;
    w.rules
        .unbound_db()
        .write(move |store| {
            store.insert_agent(
                &SwarmId::main(),
                &NewAgentRecord {
                    name: "legacy-agent".into(),
                    token_hash: auth::hash_token("old-token"),
                    recovery_hash: None,
                    created_at: "old-time".into(),
                    last_seen: None,
                    run_id,
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        w.rules
            .authenticate(Some("old-token"))
            .await
            .unwrap()
            .handle,
        "legacy-agent"
    );
    let reg = w.rules.register().await.unwrap();
    assert_eq!(
        code(
            w.rules
                .reclaim("legacy-agent".into(), reg.recovery_code)
                .await
        ),
        Code::Unauthenticated
    );
    assert_eq!(
        code(w.rules.reclaim("legacy-agent".into(), String::new()).await),
        Code::Unauthenticated
    );
    w.rules.authenticate(Some("old-token")).await.unwrap();
}

#[tokio::test]
async fn channel_notes_are_replaced_cleared_and_attributed_to_the_last_editor() {
    let w = world_with(
        Settings {
            max_note_chars: 4,
            ..Settings::DEFAULT
        },
        Arc::new(SeqEntropy::new(42)),
    )
    .await;
    let a = w.agent().await;
    let b = w.agent().await;
    let ch = w.channel(&a, "notes", true, &[&b]).await;
    let empty = w.rules.get_note(&a, ch.clone()).await.unwrap();
    assert_eq!(empty.text, "");
    assert_eq!(empty.edited_by, None);
    assert_eq!(empty.edited_at, None);
    w.rules
        .set_note(&a, ch.clone(), " 😀\n\t".into())
        .await
        .unwrap();
    let first = w.rules.get_note(&b, ch.clone()).await.unwrap();
    assert_eq!(first.text, " 😀\n\t");
    assert_eq!(first.edited_by.as_deref(), Some(a.handle.as_str()));
    assert_eq!(first.edited_at, Some(clock::to_text(w.clock.now())));
    assert_eq!(
        code(w.rules.set_note(&b, ch.clone(), "😀".repeat(5)).await),
        Code::InvalidArgument
    );
    assert_eq!(
        w.rules.get_note(&b, ch.clone()).await.unwrap().text,
        first.text
    );
    w.clock.advance(Span::seconds(10));
    w.rules
        .set_note(&b, ch.clone(), "next".into())
        .await
        .unwrap();
    let changed = w.rules.get_note(&a, ch.clone()).await.unwrap();
    assert_eq!(changed.text, "next");
    assert_eq!(changed.edited_by.as_deref(), Some(b.handle.as_str()));
    assert_eq!(changed.edited_at, Some(clock::to_text(w.clock.now())));
    let listed = w
        .rules
        .list_channels(
            &a,
            ChannelQuery {
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(listed.items[0].note.text, "next");
    assert_eq!(
        w.rules
            .channel_view(&web_page(), ch.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .channel
            .note
            .text,
        "next"
    );
    w.rules
        .set_note(&a, ch.clone(), String::new())
        .await
        .unwrap();
    let cleared = w.rules.get_note(&b, ch).await.unwrap();
    assert!(cleared.text.is_empty());
    assert_eq!(cleared.edited_by.as_deref(), Some(a.handle.as_str()));
    assert!(cleared.edited_at.is_some());
}

#[tokio::test]
async fn channel_notes_use_channel_visibility_and_membership_rules() {
    let w = world().await;
    let a = w.agent().await;
    let b = w.agent().await;
    let c = w.agent().await;
    for open in [true, false] {
        let ch = w.channel(&a, "notes", open, &[&b]).await;
        w.rules
            .set_note(&b, ch.clone(), "member note".into())
            .await
            .unwrap();
        assert_eq!(
            w.rules.get_note(&a, ch.clone()).await.unwrap().text,
            "member note"
        );
        let expected = if open {
            Code::NotMember
        } else {
            Code::NotFound
        };
        assert_eq!(
            code(w.rules.set_note(&c, ch.clone(), "outsider".into()).await),
            expected
        );
        // Permission checks precede length validation, preserving closed-channel privacy.
        assert_eq!(
            code(w.rules.set_note(&c, ch.clone(), "x".repeat(10001)).await),
            expected
        );
        if open {
            assert_eq!(w.rules.get_note(&c, ch).await.unwrap().text, "member note");
        } else {
            assert_eq!(code(w.rules.get_note(&c, ch).await), Code::NotFound);
        }
    }
    assert_eq!(
        code(w.rules.get_note(&a, "missing".into()).await),
        Code::NotFound
    );
    assert_eq!(
        code(w.rules.set_note(&a, "missing".into(), "x".into()).await),
        Code::NotFound
    );
}

#[tokio::test]
async fn only_the_human_administers_channels_and_every_change_is_announced() {
    let w = world().await;
    let a = w.agent().await;
    let b = w.agent().await;
    let id = w.channel(&a, "admin", false, &[]).await;
    assert_eq!(
        code(
            w.rules
                .change_channel(Actor::from(&a), id.clone(), ChannelChange::SetOpen(true))
                .await
        ),
        Code::OperatorOnly
    );
    assert_eq!(
        code(
            w.rules
                .change_channel(Actor::from(&b), id.clone(), ChannelChange::SetOpen(true))
                .await
        ),
        Code::NotFound
    );
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            id.clone(),
            ChannelChange::SetOpen(true),
        )
        .await
        .unwrap();
    assert!(w.visible_ids(&b).await.contains(&id));
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            id.clone(),
            ChannelChange::AddMember(b.handle.clone()),
        )
        .await
        .unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            id.clone(),
            ChannelChange::SetOpen(false),
        )
        .await
        .unwrap();
    assert!(w.rules.get_note(&b, id.clone()).await.is_ok());
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            id.clone(),
            ChannelChange::RemoveMember(b.handle.clone()),
        )
        .await
        .unwrap();
    assert_eq!(code(w.read_latest(&b, &id).await), Code::NotFound);
    let messages = w.read_latest(&a, &id).await.unwrap().messages;
    assert_eq!(messages.len(), 4);
    assert!(
        messages
            .iter()
            .all(|m| m.agent == names::OPERATOR && m.run_id == Some(w.run_id))
    );
    assert!(messages.last().unwrap().body.contains(&b.handle));
    // A no-op emits nothing, and an invalid member changes nothing.
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            id.clone(),
            ChannelChange::SetOpen(false),
        )
        .await
        .unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            id.clone(),
            ChannelChange::RemoveMember(b.handle.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        code(
            w.rules
                .change_channel(
                    Actor::Operator(web_page()),
                    id.clone(),
                    ChannelChange::AddMember("missing".into())
                )
                .await
        ),
        Code::NotFound
    );
    assert_eq!(w.read_latest(&a, &id).await.unwrap().messages.len(), 4);
}

#[tokio::test]
async fn hiding_a_channel_prevents_all_agent_reads_and_keeps_stored_messages() {
    let w = world().await;
    let a = w.agent().await;
    let id = w.channel(&a, "hidden", true, &[]).await;
    let sent = w
        .rules
        .send(&a, id.clone(), "searchable history".into())
        .await
        .unwrap();
    w.rules
        .change_channel(Actor::Operator(web_page()), id.clone(), ChannelChange::Hide)
        .await
        .unwrap();
    assert!(!w.visible_ids(&a).await.contains(&id));
    assert_eq!(code(w.read_latest(&a, &id).await), Code::NotFound);
    assert_eq!(code(w.rules.get_message(&a, sent.id).await), Code::NotFound);
    assert_eq!(code(w.rules.get_note(&a, id.clone()).await), Code::NotFound);
    assert_eq!(
        code(w.rules.join_channel(&a, id.clone()).await),
        Code::NotFound
    );
    assert_eq!(
        code(w.rules.send(&a, id.clone(), "later".into()).await),
        Code::NotFound
    );
    assert_eq!(
        code(
            w.rules
                .change_channel(
                    Actor::Operator(web_page()),
                    id.clone(),
                    ChannelChange::SetOpen(true)
                )
                .await
        ),
        Code::NotFound
    );
    assert!(
        w.rules
            .search(&a, words("searchable"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(w.rules.activity(&a, 0).await.unwrap().is_empty());
    assert!(
        w.rules
            .wait_for_messages(
                &a,
                WaitQuery {
                    after_id: 0,
                    channel_ids: None,
                    timeout: Some(Duration::ZERO),
                    limit: 20
                }
            )
            .await
            .unwrap()
            .messages
            .is_empty()
    );
    assert!(
        w.rules
            .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .unwrap()
            .channels
            .is_empty()
    );
    let stored_id = id.clone();
    let stored = w
        .rules
        .unbound_db()
        .read(move |s| {
            s.channel_messages(
                &SwarmId::main(),
                &stored_id,
                None,
                MessageRange::Latest,
                100,
            )
        })
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().any(|m| m.body == "searchable history"));
    assert!(stored.iter().any(|m| m.agent == names::OPERATOR
        && m.body == "The operator hid the channel. Its messages are kept."));
}

#[tokio::test]
async fn the_human_can_list_and_read_hidden_channels_that_agents_cannot() {
    let w = world().await;
    let a = w.agent().await;
    let b = w.agent().await;
    let kept = w.channel(&a, "kept", true, &[]).await;
    let gone = w.channel(&a, "gone", false, &[&b]).await;
    w.rules
        .set_note(&a, gone.clone(), "what we learned".into())
        .await
        .unwrap();
    w.rules
        .send(&b, gone.clone(), "last words".into())
        .await
        .unwrap();
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            gone.clone(),
            ChannelChange::Hide,
        )
        .await
        .unwrap();

    // The front page leaves it out; the hidden list has only it.
    let listed: Vec<String> = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap()
        .channels
        .into_iter()
        .map(|c| c.info.id)
        .collect();
    assert_eq!(listed, vec![kept.clone()]);
    let hidden = w
        .rules
        .hidden_channels(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].info.id, gone);
    assert_eq!(hidden[0].message_count, 2);
    assert_eq!(hidden[0].last_speaker.as_deref(), Some(names::OPERATOR));
    assert_eq!(hidden[0].info.member_count, 0);

    // The human reads it: messages, note and owner kept, members gone.
    let view = w
        .rules
        .channel_view(&web_page(), gone.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert!(view.hidden);
    assert_eq!(view.channel.note.text, "what we learned");
    assert_eq!(view.channel.owner.as_deref(), Some(a.handle.as_str()));
    assert!(view.members.is_empty());
    let bodies: Vec<&str> = view.messages.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(
        bodies,
        vec![
            "last words",
            "The operator hid the channel. Its messages are kept."
        ]
    );
    assert!(
        !w.rules
            .channel_view(&web_page(), kept, None, PAGE_MESSAGES)
            .await
            .unwrap()
            .hidden
    );
    assert_eq!(
        code(
            w.rules
                .channel_view(&web_page(), "ch_missing".into(), None, PAGE_MESSAGES)
                .await
        ),
        Code::NotFound
    );

    // A channel its owner hid is listed and read the same way, in name order.
    let owned = w.channel(&b, "owned", true, &[]).await;
    w.rules.hide_channel(&b, owned.clone()).await.unwrap();
    let hidden = w
        .rules
        .hidden_channels(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    let ids: Vec<&str> = hidden.iter().map(|c| c.info.id.as_str()).collect();
    assert_eq!(ids, vec![gone.as_str(), owned.as_str()]);
    assert_eq!(hidden[1].last_speaker.as_deref(), Some(b.handle.as_str()));
    assert!(
        w.rules
            .channel_view(&web_page(), owned, None, PAGE_MESSAGES)
            .await
            .unwrap()
            .hidden
    );

    // Nobody can change it, and agents, even its owner, cannot see it.
    assert_eq!(
        code(
            w.rules
                .set_purpose(Actor::Operator(web_page()), gone.clone(), "back".into())
                .await
        ),
        Code::NotFound
    );
    for agent in [&a, &b] {
        assert_eq!(code(w.read_latest(agent, &gone).await), Code::NotFound);
        assert_eq!(
            code(w.rules.get_note(agent, gone.clone()).await),
            Code::NotFound
        );
        assert!(!w.visible_ids(agent).await.contains(&gone));
    }
}

#[tokio::test]
async fn an_agent_unregisters_itself_as_the_operator_would_hide_it() {
    let w = world().await;
    let registration = w.rules.register().await.unwrap();
    let a = w
        .rules
        .authenticate(Some(&registration.token))
        .await
        .unwrap();
    let b = w.agent().await;
    let first = w.channel(&b, "first", false, &[&a]).await;
    let second = w.channel(&b, "second", true, &[&a]).await;
    let elsewhere = w.channel(&b, "elsewhere", true, &[]).await;
    let sent = w
        .rules
        .send(&a, first.clone(), "kept agent message".into())
        .await
        .unwrap();
    let mut notices = w.rules.subscribe();

    w.rules.unregister(&a).await.unwrap();

    // Its token and recovery code no longer work, and it is gone from lists.
    assert_eq!(
        code(w.rules.authenticate(Some(&registration.token)).await),
        Code::Unauthenticated
    );
    assert_eq!(
        code(
            w.rules
                .reclaim(a.handle.clone(), registration.recovery_code)
                .await
        ),
        Code::Unauthenticated
    );
    assert!(
        !w.rules
            .list_agents(&b, None, None, 100, None)
            .await
            .unwrap()
            .items
            .iter()
            .any(|x| x.handle == a.handle)
    );
    // It left each channel it was in, saying so itself.
    for id in [&first, &second] {
        let view = w
            .rules
            .channel_view(&web_page(), id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap();
        assert!(!view.members.iter().any(|m| m.handle == a.handle));
        let last = view.messages.last().unwrap();
        assert_eq!(last.agent, a.handle);
        assert_eq!(
            last.body,
            format!(
                "{} unregistered and left the channel. Its messages are kept.",
                a.handle
            )
        );
        assert_eq!(last.run_id, Some(w.run_id));
    }
    // A channel it was not in hears nothing.
    let view = w
        .rules
        .channel_view(&web_page(), elsewhere, None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert!(view.messages.iter().all(|m| m.agent != a.handle));
    // Waiting agents hear each notice like any message; the web page then
    // hears that the agent is gone, and nothing more. `unregister`
    // publishes before it returns, so all are ready now.
    let mut heard = Vec::new();
    for _ in 0..2 {
        match notices.next().now_or_never() {
            Some(Some(Notice::Message(m))) => heard.push((m.channel_id, m.agent)),
            _ => panic!("expected a ready message notice"),
        }
    }
    assert!(matches!(
        notices.next().now_or_never(),
        Some(Some(Notice::AgentHidden(h))) if h.agent == a.handle
    ));
    assert!(notices.next().now_or_never().is_none());
    heard.sort();
    let mut expected = vec![(first, a.handle.clone()), (second, a.handle.clone())];
    expected.sort();
    assert_eq!(heard, expected);
    // Its past messages stay.
    assert_eq!(
        w.rules.get_message(&b, sent.id).await.unwrap().body,
        "kept agent message"
    );
}

// ---- Handoff: memory and new recovery codes ----

#[tokio::test]
async fn an_entry_is_remembered_recalled_replaced_and_forgotten_by_its_avatar() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let empty = w.rules.list_memories(&a, 10, None).await.unwrap();
    assert_eq!((empty.entries, empty.chars), (0, 0));
    assert!(empty.page.items.is_empty());

    let saved = w
        .rules
        .remember(&a, "handoff".into(), "  Next: the parser.\n".into())
        .await
        .unwrap();
    assert!(!saved.replaced);
    assert_eq!((saved.entries, saved.chars), (1, 20));
    assert_eq!(saved.max_chars, Settings::DEFAULT.max_memory_chars);
    let entry = w.rules.recall(&a, "handoff".into()).await.unwrap();
    // Kept exactly as written; kkt doesn't define what goes in it.
    assert_eq!(entry.text, "  Next: the parser.\n");
    assert_eq!(entry.saved_at, w.rules.now_text());
    assert_eq!(entry.handle, a.handle);
    // Each avatar reads only its own.
    assert_eq!(
        code(w.rules.recall(&b, "handoff".into()).await),
        Code::NotFound
    );
    assert_eq!(
        w.rules.list_memories(&b, 10, None).await.unwrap().entries,
        0
    );
    // The human sees it on the agent's page.
    let view = w
        .rules
        .agent_view(&web_page(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(view.memory, vec![entry]);

    w.clock.advance(Span::seconds(30));
    let replaced = w
        .rules
        .remember(&a, "handoff".into(), "Done.".into())
        .await
        .unwrap();
    assert!(replaced.replaced);
    assert_eq!((replaced.entries, replaced.chars), (1, 5));
    let entry = w.rules.recall(&a, "handoff".into()).await.unwrap();
    assert_eq!(
        (entry.text.as_str(), entry.saved_at),
        ("Done.", w.rules.now_text())
    );

    w.rules.forget(&a, "handoff".into()).await.unwrap();
    assert_eq!(
        code(w.rules.recall(&a, "handoff".into()).await),
        Code::NotFound
    );
    assert_eq!(
        code(w.rules.forget(&a, "handoff".into()).await),
        Code::NotFound,
        "forgetting what is not there says so"
    );
}

#[tokio::test]
async fn entry_names_follow_the_naming_rules_and_text_is_not_empty() {
    let w = world().await;
    let a = w.agent().await;
    let long = "a".repeat(kkt_names::MAX_MEMORY_NAME_CHARS + 1);
    for bad in [
        "",
        "Handoff",
        "two words",
        "trailing-",
        "a--b",
        long.as_str(),
    ] {
        assert_eq!(
            code(w.rules.remember(&a, bad.into(), "x".into()).await),
            Code::InvalidArgument,
            "{bad:?}"
        );
        assert_eq!(
            code(w.rules.recall(&a, bad.into()).await),
            Code::InvalidArgument
        );
        assert_eq!(
            code(w.rules.forget(&a, bad.into()).await),
            Code::InvalidArgument
        );
    }
    assert_eq!(
        code(w.rules.remember(&a, "empty".into(), String::new()).await),
        Code::InvalidArgument
    );
    let longest = "a".repeat(kkt_names::MAX_MEMORY_NAME_CHARS);
    w.rules
        .remember(&a, longest.clone(), "x".into())
        .await
        .unwrap();
    w.rules
        .remember(&a, "plan-2".into(), "x".into())
        .await
        .unwrap();
    let names: Vec<_> = w
        .rules
        .list_memories(&a, 10, None)
        .await
        .unwrap()
        .page
        .items
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, [longest, "plan-2".to_string()]);
}

#[tokio::test]
async fn reclaim_lists_the_memory_entries() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    let fresh = w
        .rules
        .reclaim(reg.handle.clone(), reg.recovery_code.clone())
        .await
        .unwrap();
    assert!(fresh.memory.is_empty());
    w.rules
        .remember(&me, "rules".into(), "Ask first.".into())
        .await
        .unwrap();
    w.rules
        .remember(&me, "handoff".into(), "Next: the parser.".into())
        .await
        .unwrap();
    let taken = w
        .rules
        .reclaim(reg.handle.clone(), reg.recovery_code)
        .await
        .unwrap();
    let listed: Vec<_> = taken
        .memory
        .iter()
        .map(|e| (e.name.as_str(), e.chars))
        .collect();
    assert_eq!(listed, [("handoff", 17), ("rules", 10)]);
}

#[tokio::test]
async fn a_new_recovery_code_replaces_the_old_and_hands_over_the_avatar() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    w.rules
        .remember(&me, "handoff".into(), "state for my successor".into())
        .await
        .unwrap();

    let code_ = w
        .rules
        .new_recovery_code(&web_page(), reg.handle.clone())
        .await
        .unwrap();
    assert_ne!(code_, reg.recovery_code);
    // The old code stops working; the current session keeps going.
    assert_eq!(
        code(
            w.rules
                .reclaim(reg.handle.clone(), reg.recovery_code.clone())
                .await
        ),
        Code::Unauthenticated
    );
    w.rules.authenticate(Some(&reg.token)).await.unwrap();

    // The new session takes over with the new code and reads the memory.
    let taken = w.rules.reclaim(reg.handle.clone(), code_).await.unwrap();
    assert_eq!(
        code(w.rules.authenticate(Some(&reg.token)).await),
        Code::Unauthenticated
    );
    assert_eq!(taken.memory.len(), 1);
    let successor = w.rules.authenticate(Some(&taken.token)).await.unwrap();
    assert_eq!(
        w.rules
            .recall(&successor, "handoff".into())
            .await
            .unwrap()
            .text,
        "state for my successor"
    );
}

#[tokio::test]
async fn no_recovery_code_for_a_missing_or_revoked_agent() {
    let w = world().await;
    let a = w.agent().await;
    assert_eq!(
        code(
            w.rules
                .new_recovery_code(&web_page(), "nobody-here".into())
                .await
        ),
        Code::NotFound
    );
    w.operator.revoke(a.handle.clone()).await.unwrap();
    assert_eq!(
        code(
            w.rules
                .new_recovery_code(&web_page(), a.handle.clone())
                .await
        ),
        Code::InvalidArgument
    );
}
#[tokio::test]
async fn only_the_human_can_save_and_the_character_limit_is_enforced() {
    let settings = Settings {
        max_server_announcement_chars: 2,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let agent = w.agent().await;
    assert!(
        w.rules
            .save_server_announcement(Actor::from(&agent), "x".into())
            .await
            .is_err()
    );
    let saved = w
        .rules
        .save_server_announcement(Actor::Operator(web_page()), "😀é".into())
        .await
        .unwrap();
    assert!(
        w.rules
            .save_server_announcement(Actor::Operator(web_page()), "😀éx".into())
            .await
            .is_err()
    );
    assert_eq!(
        w.rules
            .server_announcement_history(&web_page(),)
            .await
            .unwrap(),
        vec![saved]
    );
}

#[tokio::test]
async fn hiding_stale_agents_hides_the_chosen_and_keeps_owners_and_the_active() {
    let w = world().await;
    let owner = w.agent().await;
    let quiet = w.rules.register().await.unwrap();
    let quiet_agent = w.rules.authenticate(Some(&quiet.token)).await.unwrap();
    let caller = w.rules.register().await.unwrap();
    let caller_agent = w.rules.authenticate(Some(&caller.token)).await.unwrap();
    let unchosen = w.agent().await;
    let room = w
        .channel(
            &owner,
            "room",
            false,
            &[&quiet_agent, &caller_agent, &unchosen],
        )
        .await;
    let other = w.channel(&unchosen, "other", true, &[&quiet_agent]).await;
    let sent = w
        .rules
        .send(&quiet_agent, room.clone(), "kept message".into())
        .await
        .unwrap();
    w.clock.advance(Span::hours(3));
    w.rules.authenticate(Some(&caller.token)).await.unwrap();
    let mut live = w.rules.subscribe();

    // Only the human may do this.
    assert_eq!(
        code(
            w.rules
                .hide_stale_agents(
                    Actor::from(&caller_agent),
                    2 * HOUR,
                    vec![quiet_agent.handle.clone()]
                )
                .await
        ),
        Code::OperatorOnly
    );
    assert_eq!(
        code(
            w.rules
                .hide_stale_agents(
                    Actor::Operator(web_page()),
                    Duration::ZERO,
                    vec![quiet_agent.handle.clone()]
                )
                .await
        ),
        Code::InvalidArgument
    );
    w.rules
        .agent_view(&web_page(), quiet_agent.handle.clone())
        .await
        .unwrap();

    let outcome = w
        .rules
        .hide_stale_agents(
            Actor::Operator(web_page()),
            2 * HOUR,
            vec![
                quiet_agent.handle.clone(),
                owner.handle.clone(),
                caller_agent.handle.clone(),
                "nobody-here".into(),
                quiet_agent.handle.clone(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(outcome.hidden, std::slice::from_ref(&quiet_agent.handle));
    assert_eq!(
        outcome.kept,
        [
            Kept {
                handle: owner.handle.clone(),
                reason: KeptReason::Owner
            },
            Kept {
                handle: caller_agent.handle.clone(),
                reason: KeptReason::Active
            },
            Kept {
                handle: "nobody-here".into(),
                reason: KeptReason::Gone
            },
        ]
    );

    // Hidden as hide_agent hides: no token, no recovery, no page, and each
    // channel it was in says so; its messages are kept.
    assert_eq!(
        code(w.rules.authenticate(Some(&quiet.token)).await),
        Code::Unauthenticated
    );
    assert_eq!(
        code(
            w.rules
                .reclaim(quiet_agent.handle.clone(), quiet.recovery_code)
                .await
        ),
        Code::Unauthenticated
    );
    assert_eq!(
        code(
            w.rules
                .agent_view(&web_page(), quiet_agent.handle.clone())
                .await
        ),
        Code::NotFound
    );
    let said = format!(
        "The operator hid {} and revoked its access. Its messages are kept.",
        quiet_agent.handle
    );
    for id in [&room, &other] {
        let view = w
            .rules
            .channel_view(&web_page(), id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap();
        assert!(!view.members.iter().any(|m| m.handle == quiet_agent.handle));
        let last = view.messages.last().unwrap();
        assert_eq!(last.agent, names::OPERATOR);
        assert_eq!(last.body, said);
    }
    let mut published = Vec::new();
    while let Some(Some(Notice::Message(m))) = live.next().now_or_never() {
        published.push(m.channel_id);
    }
    published.sort();
    let mut expected = vec![room.clone(), other.clone()];
    expected.sort();
    assert_eq!(published, expected);
    assert_eq!(
        w.rules.get_message(&owner, sent.id).await.unwrap().body,
        "kept message"
    );

    // The others are untouched: the owner still owns the channel.
    let view = w
        .rules
        .channel_view(&web_page(), room.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.channel.owner.as_deref(), Some(owner.handle.as_str()));
    for kept in [&owner, &caller_agent, &unchosen] {
        w.rules
            .agent_view(&web_page(), kept.handle.clone())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn the_admin_lists_every_channel_closed_and_hidden_ones_included() {
    let a = admin_world().await;
    let page =
        a.w.rules
            .admin_list_channels(&web_page(), None, 100, None)
            .await
            .unwrap();
    let seen: Vec<(String, bool)> = page
        .items
        .iter()
        .map(|c| (c.info.id.clone(), c.hidden))
        .collect();
    for (id, hidden) in [(&a.open, false), (&a.closed, false), (&a.hidden, true)] {
        assert!(seen.contains(&(id.clone(), hidden)), "{seen:?}");
    }
    // Agents still find neither the hidden channel nor, outside it, the
    // closed one.
    let theirs = a.w.visible_ids(&a.outsider).await;
    assert!(theirs.contains(&a.open));
    assert!(!theirs.contains(&a.closed) && !theirs.contains(&a.hidden));
    assert!(!a.w.visible_ids(&a.owner).await.contains(&a.hidden));

    // Pages follow name order, and `query` matches names.
    let first =
        a.w.rules
            .admin_list_channels(&web_page(), None, 1, None)
            .await
            .unwrap();
    let rest =
        a.w.rules
            .admin_list_channels(&web_page(), None, 10, first.next_cursor)
            .await
            .unwrap();
    assert_eq!(first.items.len() + rest.items.len(), 3);
    let found =
        a.w.rules
            .admin_list_channels(&web_page(), Some("secr".into()), 10, None)
            .await
            .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].info.id, a.closed);
}

#[tokio::test]
async fn the_admin_reads_closed_and_hidden_channels_that_agents_cannot() {
    let a = admin_world().await;
    for id in [&a.closed, &a.hidden] {
        let page =
            a.w.rules
                .admin_read(
                    &web_page(),
                    ReadQuery {
                        channel_id: id.clone(),
                        position: Position::Latest,
                        from_agent: None,
                        limit: 20,
                    },
                )
                .await
                .unwrap();
        assert!(!page.messages.is_empty(), "{id}");
        assert_eq!(code(a.w.read_latest(&a.outsider, id).await), Code::NotFound);
    }
    // Hiding is announced in the channel, after the message from before.
    let hidden =
        a.w.rules
            .admin_message(&web_page(), a.hidden_message)
            .await
            .unwrap();
    assert_eq!(hidden.body, "before hiding");
    assert_eq!(
        code(a.w.rules.get_message(&a.owner, a.hidden_message).await),
        Code::NotFound
    );
    assert_eq!(
        code(a.w.rules.admin_message(&web_page(), 999_999).await),
        Code::NotFound
    );
    assert_eq!(
        code(
            a.w.rules
                .admin_channel(&web_page(), "ch_missing".into())
                .await
        ),
        Code::NotFound
    );
}

#[tokio::test]
async fn the_admin_sees_a_channels_details_members_owner_and_note() {
    let a = admin_world().await;
    let closed =
        a.w.rules
            .admin_channel(&web_page(), a.closed.clone())
            .await
            .unwrap();
    assert!(!closed.hidden && !closed.info.open);
    assert_eq!(closed.info.owner.as_deref(), Some(a.owner.handle.as_str()));
    let members =
        a.w.rules
            .admin_list_members(&web_page(), a.closed.clone(), 10, None)
            .await
            .unwrap();
    assert_eq!(members.owner.as_deref(), Some(a.owner.handle.as_str()));
    let names: Vec<_> = members
        .page
        .items
        .iter()
        .map(|m| m.handle.clone())
        .collect();
    assert_eq!(names, std::slice::from_ref(&a.owner.handle));
    let note =
        a.w.rules
            .admin_note(&web_page(), a.closed.clone())
            .await
            .unwrap();
    assert_eq!(note.text, "the closed note");
    assert_eq!(note.edited_by.as_deref(), Some(a.owner.handle.as_str()));
    assert_eq!(
        code(a.w.rules.get_note(&a.outsider, a.closed.clone()).await),
        Code::NotFound
    );

    // A hidden channel keeps its details; hiding removed its members.
    let hidden =
        a.w.rules
            .admin_channel(&web_page(), a.hidden.clone())
            .await
            .unwrap();
    assert!(hidden.hidden);
    let members =
        a.w.rules
            .admin_list_members(&web_page(), a.hidden.clone(), 10, None)
            .await
            .unwrap();
    assert_eq!(members.total, 0);
}

#[tokio::test]
async fn the_admin_sees_new_messages_in_every_channel_without_waiting() {
    let a = admin_world().await;
    let rows = a.w.rules.admin_activity(&web_page(), 0).await.unwrap();
    let ids: Vec<_> = rows.iter().map(|r| r.channel_id.clone()).collect();
    for id in [&a.open, &a.closed, &a.hidden] {
        assert!(ids.contains(id), "{ids:?}");
    }
    let all =
        a.w.rules
            .admin_new_messages(&web_page(), 0, None, 100)
            .await
            .unwrap();
    let mut channels: Vec<_> = all.messages.iter().map(|m| m.channel_id.clone()).collect();
    channels.dedup();
    assert!(channels.contains(&a.hidden) && channels.contains(&a.closed));
    assert!(all.messages.windows(2).all(|p| p[0].id < p[1].id));
    assert_eq!(all.more_waiting, 0);

    // A limit leaves the rest counted; a filter keeps to its channels.
    let first =
        a.w.rules
            .admin_new_messages(&web_page(), 0, None, 1)
            .await
            .unwrap();
    assert_eq!(first.messages.len(), 1);
    assert_eq!(first.more_waiting, all.messages.len() as i64 - 1);
    let closed =
        a.w.rules
            .admin_new_messages(&web_page(), 0, Some(vec![a.closed.clone()]), 100)
            .await
            .unwrap();
    assert!(closed.messages.iter().all(|m| m.channel_id == a.closed));
    assert!(!closed.messages.is_empty());
    let last = all.messages.last().unwrap().id;
    let none =
        a.w.rules
            .admin_new_messages(&web_page(), last, None, 100)
            .await
            .unwrap();
    assert!(none.messages.is_empty());
    assert_eq!(
        code(
            a.w.rules
                .admin_new_messages(&web_page(), 0, Some(vec![]), 10)
                .await
        ),
        Code::InvalidArgument
    );
    assert_eq!(
        code(
            a.w.rules
                .admin_new_messages(&web_page(), 0, Some(vec!["ch_missing".into()]), 10)
                .await
        ),
        Code::NotFound
    );
}

#[tokio::test]
async fn the_admin_lists_agents() {
    let a = admin_world().await;
    let page =
        a.w.rules
            .admin_list_agents(&web_page(), None, None, 10, None)
            .await
            .unwrap();
    let names: Vec<_> = page.items.iter().map(|i| i.handle.clone()).collect();
    assert!(names.contains(&a.owner.handle) && names.contains(&a.outsider.handle));
    let one =
        a.w.rules
            .admin_list_agents(&web_page(), None, None, 1, None)
            .await
            .unwrap();
    assert_eq!(one.items.len(), 1);
    assert!(one.next_cursor.is_some());
}

#[tokio::test]
async fn the_admin_reads_an_agents_details_as_the_web_page_does() {
    let a = admin_world().await;
    a.w.rules
        .remember(&a.owner, "handoff".into(), "Next: the parser.".into())
        .await
        .unwrap();
    let view =
        a.w.rules
            .admin_agent(&web_page(), a.owner.handle.clone())
            .await
            .unwrap();
    let web =
        a.w.rules
            .agent_view(&web_page(), a.owner.handle.clone())
            .await
            .unwrap();
    assert_eq!(view.memory.len(), 1);
    assert_eq!(view.memory[0].text, "Next: the parser.");
    // Three sent and one announcing the hiding; hiding removed the hidden
    // channel's members.
    assert_eq!(view.message_count, 4);
    let ids: Vec<_> = view.channels.iter().map(|c| c.id.clone()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&a.open) && ids.contains(&a.closed), "{ids:?}");
    assert_eq!(view.message_count, web.message_count);
    assert_eq!(view.memory, web.memory);
    let web_ids: Vec<_> = web.channels.iter().map(|c| c.id.clone()).collect();
    assert_eq!(ids, web_ids);

    let entry =
        a.w.rules
            .admin_recall(a.owner.handle.clone(), "handoff".into())
            .await
            .unwrap();
    assert_eq!(entry.text, "Next: the parser.");
    let list =
        a.w.rules
            .admin_list_memories(a.owner.handle.clone(), 10, None)
            .await
            .unwrap();
    assert_eq!(list.entries, 1);
    assert_eq!(list.page.items[0].name, "handoff");
    // An agent's memory stays its own: another agent finds nothing there.
    assert_eq!(
        code(a.w.rules.recall(&a.outsider, "handoff".into()).await),
        Code::NotFound
    );
    for missing in [
        code(
            a.w.rules
                .admin_agent(&web_page(), "nobody-here".into())
                .await,
        ),
        code(
            a.w.rules
                .admin_recall("nobody-here".into(), "handoff".into())
                .await,
        ),
        code(
            a.w.rules
                .admin_recall(a.owner.handle.clone(), "nothing".into())
                .await,
        ),
        code(
            a.w.rules
                .admin_list_memories("nobody-here".into(), 10, None)
                .await,
        ),
    ] {
        assert_eq!(missing, Code::NotFound);
    }
}

#[tokio::test]
async fn an_admin_key_without_a_name_is_refused_every_change_and_may_read() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let room = w.channel(&a, "room", false, &[&b]).await;
    let said = w
        .rules
        .send(&a, room.clone(), "hello".into())
        .await
        .unwrap();
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "t"))
        .await
        .unwrap();
    let before = format!(
        "{:?}",
        w.rules
            .admin_channel(&web_page(), room.clone())
            .await
            .unwrap()
    );
    let unnamed = || Actor::Operator(unnamed_admin());

    // Every change the admin tools make, each refused alike.
    let mut refused = Vec::new();
    for change in [
        ChannelChange::SetOpen(true),
        ChannelChange::AddMember(a.handle.clone()),
        ChannelChange::RemoveMember(b.handle.clone()),
        ChannelChange::SetOwner(Some(b.handle.clone())),
        ChannelChange::SetOwner(None),
        ChannelChange::Hide,
    ] {
        refused.push(
            w.rules
                .change_channel(unnamed(), room.clone(), change)
                .await
                .map(drop),
        );
    }
    let new = NewChannel {
        name: "new".into(),
        open: true,
        purpose: None,
        members: vec![],
    };
    refused.push(
        w.rules
            .create_channel_as_operator(unnamed(), new)
            .await
            .map(drop),
    );
    refused.push(
        w.rules
            .set_purpose(unnamed(), room.clone(), "x".into())
            .await
            .map(drop),
    );
    refused.push(w.rules.hide_agent(unnamed(), b.handle.clone()).await);
    refused.push(
        w.rules
            .hide_stale_agents(unnamed(), HOUR, vec![b.handle.clone()])
            .await
            .map(drop),
    );
    refused.push(
        w.rules
            .save_server_announcement(unnamed(), "x".into())
            .await
            .map(drop),
    );
    refused.push(
        w.rules
            .save_lane(unnamed(), lane("writer", "d", "t"))
            .await
            .map(drop),
    );
    refused.push(
        w.rules
            .create_lane(unnamed(), lane("writer", "d", "t"))
            .await
            .map(drop),
    );
    refused.push(
        w.rules
            .update_lane(unnamed(), lane("reviewer", "d", "other"))
            .await
            .map(drop),
    );
    refused.push(
        w.rules
            .retire_lane(unnamed(), "reviewer".into())
            .await
            .map(drop),
    );
    assert_eq!(refused.len(), 15);
    for (i, result) in refused.into_iter().enumerate() {
        let e = result.expect_err("an unnamed admin's change");
        // The admin endpoint words this refusal for an unnamed key, word
        // for word as before (kkt-mcp's tests and tests/admin.rs).
        assert_eq!(e.code, Code::Unauthenticated, "change {i}");
        assert_eq!(
            e.refusal,
            Some(Refusal::LacksPermission(Permission::Change)),
            "change {i}"
        );
    }

    // Nothing changed.
    assert_eq!(
        format!(
            "{:?}",
            w.rules
                .admin_channel(&web_page(), room.clone())
                .await
                .unwrap()
        ),
        before
    );
    assert!(
        w.rules
            .current_server_announcement()
            .await
            .unwrap()
            .is_none()
    );
    let reviewer = w.rules.lane(&web_page(), "reviewer".into()).await.unwrap();
    assert!(reviewer.retired.is_none());
    assert_eq!(reviewer.current.text, "t");
    assert_eq!(
        code(w.rules.lane(&web_page(), "writer".into()).await),
        Code::NotFound
    );
    assert_eq!(
        w.rules
            .admin_list_channels(&web_page(), None, 10, None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );

    // Every read the admin tools make works.
    w.rules
        .admin_list_channels(&web_page(), None, 10, None)
        .await
        .unwrap();
    w.rules
        .admin_channel(&web_page(), room.clone())
        .await
        .unwrap();
    w.rules
        .admin_list_members(&web_page(), room.clone(), 10, None)
        .await
        .unwrap();
    w.rules.admin_note(&web_page(), room.clone()).await.unwrap();
    w.rules
        .admin_read(
            &web_page(),
            ReadQuery {
                channel_id: room.clone(),
                position: Position::Latest,
                from_agent: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    w.rules.admin_message(&web_page(), said.id).await.unwrap();
    w.rules.admin_activity(&web_page(), 0).await.unwrap();
    w.rules
        .admin_new_messages(&web_page(), 0, None, 10)
        .await
        .unwrap();
    w.rules
        .admin_list_agents(&web_page(), None, None, 10, None)
        .await
        .unwrap();
    w.rules
        .admin_agent(&web_page(), a.handle.clone())
        .await
        .unwrap();
    w.rules
        .admin_list_memories(a.handle.clone(), 10, None)
        .await
        .unwrap();
    w.rules.current_server_announcement().await.unwrap();
    w.rules.lanes_page(&web_page(), 10, None).await.unwrap();
    w.rules.lane(&web_page(), "reviewer".into()).await.unwrap();
    w.rules.stale_agents(&web_page(), HOUR).await.unwrap();
}

#[tokio::test]
async fn an_admin_agent_changes_channels_by_the_humans_rules_announced_via_its_name() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let room = w.channel(&a, "room", false, &[&b]).await;
    let via = "The operator (via admin-sentinel)";
    let mut live = w.rules.subscribe();

    // Each change and its announcement, from the operator, naming the admin.
    let changes = [
        (
            ChannelChange::SetOpen(true),
            format!("{via} made the channel open."),
        ),
        (
            ChannelChange::AddMember(c.handle.clone()),
            format!("{via} added {} to the channel.", c.handle),
        ),
        (
            ChannelChange::RemoveMember(c.handle.clone()),
            format!("{via} removed {} from the channel.", c.handle),
        ),
        (
            ChannelChange::SetOwner(Some(b.handle.clone())),
            format!("{via} made {} the owner of the channel.", b.handle),
        ),
        (
            ChannelChange::SetOwner(None),
            format!(
                "{via} removed {} as the owner of the channel. The channel no longer has an owner.",
                b.handle
            ),
        ),
    ];
    for (change, said) in changes {
        w.rules
            .change_channel(sentinel(), room.clone(), change)
            .await
            .unwrap();
        let last = last_said(&w, &room).await;
        assert_eq!(last.agent, names::OPERATOR);
        assert_eq!(last.body, said);
        assert_eq!(last.run_id, Some(w.run_id));
        match live.next().now_or_never() {
            Some(Some(Notice::Message(m))) => assert_eq!(m.body, said),
            other => panic!("expected the announcement, got {other:?}"),
        }
        // Adding or removing a member also says it joined or left, as the
        // human's change does.
        while let Some(Some(other)) = live.next().now_or_never() {
            assert!(
                matches!(other, Notice::Joined(_) | Notice::Left(_)),
                "{other:?}"
            );
        }
    }
    let view = w
        .rules
        .channel_view(&web_page(), room.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert!(view.channel.open);
    assert_eq!(view.channel.owner, None);
    assert!(!view.members.iter().any(|m| m.handle == c.handle));

    // A change that changes nothing says nothing.
    let before = last_said(&w, &room).await.id;
    w.rules
        .change_channel(sentinel(), room.clone(), ChannelChange::SetOpen(true))
        .await
        .unwrap();
    assert_eq!(last_said(&w, &room).await.id, before);

    // Purpose, a new channel, and hiding.
    w.rules
        .set_purpose(sentinel(), room.clone(), "triage".into())
        .await
        .unwrap();
    assert_eq!(
        last_said(&w, &room).await.body,
        format!("{via} changed the purpose to: triage")
    );
    let made = w
        .rules
        .create_channel_as_operator(
            sentinel(),
            NewChannel {
                name: "made".into(),
                purpose: None,
                open: false,
                members: vec![a.handle.clone()],
            },
        )
        .await
        .unwrap();
    assert_eq!(made.created_by, names::OPERATOR);
    assert_eq!(
        last_said(&w, &made.id).await.body,
        format!("{via} created the channel.")
    );
    w.rules
        .change_channel(sentinel(), room.clone(), ChannelChange::Hide)
        .await
        .unwrap();
    assert_eq!(
        last_said(&w, &room).await.body,
        format!("{via} hid the channel. Its messages are kept.")
    );
    // Hidden: nobody may change it, the admin agent included.
    assert_eq!(
        code(
            w.rules
                .change_channel(sentinel(), room.clone(), ChannelChange::SetOpen(false))
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code(
            w.rules
                .set_purpose(sentinel(), room.clone(), "back".into())
                .await
        ),
        Code::NotFound
    );
    assert_eq!(code(w.read_latest(&a, &room).await), Code::NotFound);
}

#[tokio::test]
async fn an_admin_agent_cannot_hide_an_owner_and_hides_others_via_its_name() {
    let w = world().await;
    let (owner, member, quiet) = (w.agent().await, w.agent().await, w.agent().await);
    let room = w.channel(&owner, "room", true, &[&member, &quiet]).await;
    w.clock.advance(Span::hours(3));

    // An owner is refused by either route, and nothing changes.
    assert_eq!(
        code(w.rules.hide_agent(sentinel(), owner.handle.clone()).await),
        Code::InvalidArgument
    );
    let outcome = w
        .rules
        .hide_stale_agents(sentinel(), 2 * HOUR, vec![owner.handle.clone()])
        .await
        .unwrap();
    assert!(outcome.hidden.is_empty());
    assert_eq!(outcome.kept[0].reason, KeptReason::Owner);
    w.rules
        .agent_view(&web_page(), owner.handle.clone())
        .await
        .unwrap();

    // Others are hidden as the human hides them, announced via the name.
    w.rules
        .hide_agent(sentinel(), member.handle.clone())
        .await
        .unwrap();
    assert_eq!(
        last_said(&w, &room).await.body,
        format!(
            "The operator (via admin-sentinel) hid {} and revoked its access. Its messages are kept.",
            member.handle
        )
    );
    let outcome = w
        .rules
        .hide_stale_agents(sentinel(), 2 * HOUR, vec![quiet.handle.clone()])
        .await
        .unwrap();
    assert_eq!(outcome.hidden, std::slice::from_ref(&quiet.handle));
    let last = last_said(&w, &room).await;
    assert_eq!(last.agent, names::OPERATOR);
    assert!(
        last.body
            .starts_with("The operator (via admin-sentinel) hid ")
    );
    for gone in [&member, &quiet] {
        assert_eq!(
            code(w.rules.agent_view(&web_page(), gone.handle.clone()).await),
            Code::NotFound
        );
    }
}

#[tokio::test]
async fn an_admin_agent_saves_server_announcement_and_agents_still_cannot() {
    let w = world().await;
    let agent = w.agent().await;
    let saved = w
        .rules
        .save_server_announcement(sentinel(), "Be brief.".into())
        .await
        .unwrap();
    assert_eq!(saved.version, 1);
    assert_eq!(saved.saved_by_admin.as_deref(), Some("admin-sentinel"));
    assert_eq!(
        w.rules
            .current_server_announcement()
            .await
            .unwrap()
            .map(|v| v.text),
        Some("Be brief.".into())
    );
    assert_eq!(
        code(
            w.rules
                .save_server_announcement(Actor::from(&agent), "x".into())
                .await
        ),
        Code::OperatorOnly
    );
    assert_eq!(
        w.rules
            .server_announcement_history(&web_page(),)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn each_announcement_version_records_whether_the_human_or_an_admin_saved_it() {
    let w = world().await;
    let saves = [
        (Actor::Operator(web_page()), "Be kind."),
        (sentinel(), "Be brief."),
        // Clearing is a version too, recorded the same way.
        (sentinel(), ""),
        (Actor::Operator(web_page()), ""),
        (Actor::Operator(web_page()), "Be quick."),
    ];
    for (actor, text) in saves.clone() {
        w.rules
            .save_server_announcement(actor, text.into())
            .await
            .unwrap();
    }
    let history = w
        .rules
        .server_announcement_history(&web_page())
        .await
        .unwrap();
    let got: Vec<(Option<&str>, &str)> = history
        .iter()
        .rev()
        .map(|v| (v.saved_by_admin.as_deref(), v.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (None, "Be kind."),
            (Some("admin-sentinel"), "Be brief."),
            (Some("admin-sentinel"), ""),
            (None, ""),
            (None, "Be quick."),
        ]
    );
    assert!(history.iter().all(|v| v.run_id == w.run_id));

    // Agents are given who saved the current version.
    w.rules
        .save_server_announcement(sentinel(), "Be brief again.".into())
        .await
        .unwrap();
    let reg = w.rules.register().await.unwrap();
    let current = reg.server_announcement.expect("current announcement");
    assert_eq!(current.text, "Be brief again.");
    assert_eq!(current.saved_by_admin.as_deref(), Some("admin-sentinel"));
}

#[tokio::test]
async fn each_lane_save_is_a_new_version_saying_who_saved_it_when_and_in_which_run() {
    let w = world().await;
    assert!(w.rules.lanes(&web_page(),).await.unwrap().is_empty());
    let first = w
        .rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "  Reviews pull requests.  ", "Read the diff.\n"),
        )
        .await
        .unwrap();
    assert_eq!(first.name, "reviewer");
    assert_eq!(first.current.version, 1);
    // The description is trimmed; the instructions are kept as given.
    assert_eq!(first.current.description, "Reviews pull requests.");
    assert_eq!(first.current.text, "Read the diff.\n");
    assert_eq!(first.current.saved_at, w.rules.now_text());
    assert_eq!(first.current.saved_by_admin, None);
    assert_eq!(first.created_at, w.rules.now_text());
    assert_eq!((first.run_id, first.current.run_id), (w.run_id, w.run_id));
    assert_eq!(first.retired, None);

    w.clock.advance(Span::seconds(10));
    let second = w
        .rules
        .save_lane(
            sentinel(),
            lane("reviewer", "Reviews pull requests.", "Read it twice."),
        )
        .await
        .unwrap();
    assert_eq!(second.current.version, 2);
    assert_eq!(second.current.saved_at, w.rules.now_text());
    assert_eq!(
        second.current.saved_by_admin.as_deref(),
        Some("admin-sentinel")
    );
    assert_eq!(second.created_at, first.created_at);

    let history = w
        .rules
        .lane_history(&web_page(), "reviewer".into())
        .await
        .unwrap();
    assert_eq!(history.lane, second);
    assert_eq!(
        history.versions,
        vec![second.current.clone(), first.current]
    );
    assert_eq!(
        w.rules.lane(&web_page(), "reviewer".into()).await.unwrap(),
        second
    );

    // Lanes list in name order, each with its own version numbers.
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("a-first", "Comes first.", "x"),
        )
        .await
        .unwrap();
    let lanes = w.rules.lanes(&web_page()).await.unwrap();
    assert_eq!(
        lanes
            .iter()
            .map(|l| (l.name.as_str(), l.current.version))
            .collect::<Vec<_>>(),
        [("a-first", 1), ("reviewer", 2)]
    );
}

#[tokio::test]
async fn lane_names_follow_the_handle_rules_and_are_separate_from_handles() {
    let w = world().await;
    let agent = w.agent().await;
    // A lane may share its name with an avatar, and with `operator`.
    for name in [
        agent.handle.as_str(),
        "operator",
        "admin-x",
        "a1-b2",
        &"a".repeat(40),
    ] {
        w.rules
            .save_lane(Actor::Operator(web_page()), lane(name, "d", "t"))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    for bad in [
        "",
        "Reviewer",
        "two  words",
        "a--b",
        "-a",
        "a-",
        "a_b",
        "é",
        &"a".repeat(41),
    ] {
        assert_eq!(
            code(
                w.rules
                    .save_lane(Actor::Operator(web_page()), lane(bad, "d", "t"))
                    .await
            ),
            Code::InvalidArgument,
            "{bad:?}"
        );
    }
    assert_eq!(w.rules.lanes(&web_page(),).await.unwrap().len(), 5);
}

#[tokio::test]
async fn a_lane_needs_a_one_line_description_and_instructions_within_the_limits() {
    let settings = Settings {
        max_lane_chars: 3,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let long_description = "d".repeat(Settings::DEFAULT.max_lane_description_chars + 1);
    for (description, text) in [
        ("", "abc"),
        ("   ", "abc"),
        ("two\nlines", "abc"),
        ("carriage\rreturn", "abc"),
        (long_description.as_str(), "abc"),
        ("d", ""),
        ("d", " \n "),
        ("d", "😀éxy"),
    ] {
        assert_eq!(
            code(
                w.rules
                    .save_lane(
                        Actor::Operator(web_page()),
                        lane("reviewer", description, text)
                    )
                    .await
            ),
            Code::InvalidArgument,
            "{description:?} {text:?}"
        );
    }
    assert!(w.rules.lanes(&web_page(),).await.unwrap().is_empty());
    // The limit counts characters, not bytes.
    let max_description = "😀".repeat(Settings::DEFAULT.max_lane_description_chars);
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", &max_description, "😀éx"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn only_the_operator_saves_or_retires_lanes() {
    let w = world().await;
    let agent = w.agent().await;
    assert_eq!(
        code(
            w.rules
                .save_lane(Actor::from(&agent), lane("reviewer", "d", "t"))
                .await
        ),
        Code::OperatorOnly
    );
    assert!(w.rules.lanes(&web_page(),).await.unwrap().is_empty());
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "t"))
        .await
        .unwrap();
    assert_eq!(
        code(
            w.rules
                .retire_lane(Actor::from(&agent), "reviewer".into())
                .await
        ),
        Code::OperatorOnly
    );
    assert_eq!(
        w.rules
            .lane(&web_page(), "reviewer".into())
            .await
            .unwrap()
            .retired,
        None
    );
}

#[tokio::test]
async fn missing_lanes_are_not_found_and_creating_or_updating_checks_whether_the_lane_exists() {
    let w = world().await;
    assert_eq!(
        code(w.rules.lane(&web_page(), "nope".into()).await),
        Code::NotFound
    );
    assert_eq!(
        code(w.rules.lane_history(&web_page(), "nope".into()).await),
        Code::NotFound
    );
    assert_eq!(
        code(
            w.rules
                .retire_lane(Actor::Operator(web_page()), "nope".into())
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code(
            w.rules
                .update_lane(Actor::Operator(web_page()), lane("nope", "d", "t"))
                .await
        ),
        Code::NotFound
    );
    assert!(w.rules.lanes(&web_page(),).await.unwrap().is_empty());

    w.rules
        .create_lane(Actor::Operator(web_page()), lane("reviewer", "d", "t"))
        .await
        .unwrap();
    assert_eq!(
        code(
            w.rules
                .create_lane(Actor::Operator(web_page()), lane("reviewer", "d", "other"))
                .await
        ),
        Code::InvalidArgument
    );
    let updated = w
        .rules
        .update_lane(Actor::Operator(web_page()), lane("reviewer", "d", "other"))
        .await
        .unwrap();
    assert_eq!(updated.current.version, 2);
    assert_eq!(updated.current.text, "other");
}

#[tokio::test]
async fn an_agent_takes_updates_switches_and_drops_a_lane_and_says_so_where_it_is() {
    let w = world().await;
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read."),
        )
        .await
        .unwrap();
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("writer", "Writes.", "Write."),
        )
        .await
        .unwrap();
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let open = w.channel(&b, "open", true, &[&a]).await;
    let closed = w.channel(&b, "closed", false, &[&a]).await;
    let elsewhere = w.channel(&b, "elsewhere", true, &[&c]).await;
    let before = said(&w, &open).await.len();

    let took = w
        .rules
        .take_lane(&a, Some("reviewer".into()))
        .await
        .unwrap();
    assert_eq!(took.change, LaneChange::Took);
    let held = took.held.expect("held");
    assert_eq!(
        (held.lane.name.as_str(), held.lane.version),
        ("reviewer", 1)
    );
    assert_eq!(held.text, "Read.");
    assert_eq!(held.lane.taken_at, w.rules.now_text());
    let took_line = (
        a.handle.clone(),
        format!("{} took the reviewer lane (version 1).", a.handle),
    );
    assert_eq!(said(&w, &open).await.last(), Some(&took_line));
    assert_eq!(said(&w, &closed).await.last(), Some(&took_line));
    assert!(
        said(&w, &elsewhere)
            .await
            .iter()
            .all(|(who, _)| *who != a.handle)
    );

    // A newer version: the agent keeps the one it took until it asks.
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read twice."),
        )
        .await
        .unwrap();
    let held = w.rules.held_lane(&a).await.unwrap().expect("held");
    assert_eq!((held.lane.version, held.lane.current_version), (1, 2));
    assert!(held.lane.out_of_date());
    assert_eq!(held.text, "Read.");
    let seen = observed(&w, &b, &a.handle).await.lane.expect("lane");
    assert!(seen.out_of_date());

    let updated = w
        .rules
        .take_lane(&a, Some("reviewer".into()))
        .await
        .unwrap();
    assert_eq!(updated.change, LaneChange::Updated { from: 1 });
    assert_eq!(updated.held.expect("held").text, "Read twice.");
    let switched = w.rules.take_lane(&a, Some("writer".into())).await.unwrap();
    assert_eq!(
        switched.change,
        LaneChange::Switched {
            from: "reviewer".into()
        }
    );
    let count = said(&w, &open).await.len();
    let kept = w.rules.take_lane(&a, Some("writer".into())).await.unwrap();
    assert_eq!(kept.change, LaneChange::Unchanged);
    assert_eq!(said(&w, &open).await.len(), count, "nothing to say");
    let dropped = w.rules.take_lane(&a, None).await.unwrap();
    assert_eq!(
        dropped.change,
        LaneChange::Dropped {
            from: "writer".into()
        }
    );
    assert_eq!(dropped.held, None);
    let bodies: Vec<String> = said(&w, &open).await[before..]
        .iter()
        .map(|(_, body)| body.clone())
        .collect();
    assert_eq!(
        bodies,
        [
            format!("{} took the reviewer lane (version 1).", a.handle),
            format!(
                "{} moved from version 1 to the reviewer lane (version 2).",
                a.handle
            ),
            format!(
                "{} switched from the reviewer lane to the writer lane (version 1).",
                a.handle
            ),
            format!("{} dropped the writer lane.", a.handle),
        ]
    );
    // Others' lanes are their own.
    assert_eq!(w.rules.held_lane(&b).await.unwrap(), None);
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a short real wait shows the spawned wait has not ended; a longer one bounds it"
)]
async fn a_wait_says_when_the_lane_it_holds_has_a_newer_version() {
    let w = world().await;
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read."),
        )
        .await
        .unwrap();
    let (a, b) = (w.agent().await, w.agent().await);
    let quick = || WaitQuery {
        after_id: 0,
        channel_ids: None,
        timeout: Some(Duration::ZERO),
        limit: 10,
    };
    // No lane, and a lane at its newest version: nothing to say.
    assert_eq!(
        w.rules
            .wait_for_messages(&a, quick())
            .await
            .unwrap()
            .newer_lane,
        None
    );
    w.rules
        .take_lane(&a, Some("reviewer".into()))
        .await
        .unwrap();
    assert_eq!(
        w.rules
            .wait_for_messages(&a, quick())
            .await
            .unwrap()
            .newer_lane,
        None
    );

    // A version saved while the agent waits does not end the wait; the
    // wait says so when it ends.
    let mut waiter = open_wait(&w, &a, None);
    until_sleeping(&w.clock).await;
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read twice."),
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiter)
            .await
            .is_err(),
        "a new lane version must not end the wait"
    );
    w.clock.advance(Span::seconds(10));
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the wait should time out")
        .expect("join")
        .expect("wait");
    assert!(got.messages.is_empty());
    let newer = got.newer_lane.expect("the lane has a newer version");
    assert_eq!(
        (newer.name.as_str(), newer.version, newer.current_version),
        ("reviewer", 1, 2)
    );

    // It is said with messages too, and only to the agent holding the lane.
    let ch = w.channel(&b, "general", true, &[&a]).await;
    w.rules.send(&b, ch, "hello".into()).await.unwrap();
    let got = w.rules.wait_for_messages(&a, quick()).await.unwrap();
    assert!(!got.messages.is_empty());
    assert!(got.newer_lane.is_some());
    assert_eq!(
        w.rules
            .wait_for_messages(&b, quick())
            .await
            .unwrap()
            .newer_lane,
        None
    );

    // Moving to the newest version ends it.
    w.rules
        .take_lane(&a, Some("reviewer".into()))
        .await
        .unwrap();
    assert_eq!(
        w.rules
            .wait_for_messages(&a, quick())
            .await
            .unwrap()
            .newer_lane,
        None
    );
}

#[tokio::test]
async fn an_unknown_or_retired_lane_is_refused_and_changes_nothing() {
    let settings = Settings {
        registrations_per_minute: 1,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read."),
        )
        .await
        .unwrap();
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("gone", "Gone.", "Old."))
        .await
        .unwrap();
    w.rules
        .retire_lane(Actor::Operator(web_page()), "gone".into())
        .await
        .unwrap();
    for (name, code) in [
        ("nope", Code::LaneUnavailable),
        ("gone", Code::LaneUnavailable),
        ("Not A Lane", Code::InvalidArgument),
    ] {
        let err = w
            .rules
            .register_with_client(None, None, Some(name.into()))
            .await
            .expect_err(name);
        assert_eq!(err.code, code, "{name}: {}", err.message);
        assert!(
            err.message.contains("Nothing was registered."),
            "{}",
            err.message
        );
    }
    // None of them used up the one registration this minute allows.
    let reg = w
        .rules
        .register_with_client(None, None, Some("reviewer".into()))
        .await
        .unwrap();
    assert_eq!(reg.lane.expect("lane").lane.name, "reviewer");
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    let ch = w.channel(&me, "mine", true, &[]).await;
    let count = said(&w, &ch).await.len();
    for name in ["nope", "gone"] {
        let err = w
            .rules
            .take_lane(&me, Some(name.into()))
            .await
            .expect_err(name);
        assert_eq!(err.code, Code::LaneUnavailable, "{name}: {}", err.message);
        assert!(err.message.contains("Nothing changed."), "{}", err.message);
        let err = w
            .rules
            .reclaim_with_client(
                me.handle.clone(),
                reg.recovery_code.clone(),
                None,
                Some(name.into()),
            )
            .await
            .expect_err(name);
        assert_eq!(err.code, Code::LaneUnavailable, "{name}: {}", err.message);
        assert!(err.message.contains("Nothing changed."), "{}", err.message);
    }
    // The token was not replaced, the lane not changed, nothing said.
    assert_eq!(w.rules.authenticate(Some(&reg.token)).await.unwrap(), me);
    let held = w.rules.held_lane(&me).await.unwrap().expect("held");
    assert_eq!(held.lane.name, "reviewer");
    assert_eq!(said(&w, &ch).await.len(), count);

    // An agent holding a lane that is then retired keeps its record.
    w.rules
        .retire_lane(Actor::Operator(web_page()), "reviewer".into())
        .await
        .unwrap();
    assert_eq!(
        w.rules.held_lane(&me).await.unwrap().expect("held").text,
        "Read."
    );
}

#[tokio::test]
async fn registering_with_a_lane_posts_nothing_and_list_agents_filters_by_lane() {
    let w = world().await;
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read."),
        )
        .await
        .unwrap();
    let viewer = w.agent().await;
    let reg = w
        .rules
        .register_with_client(None, None, Some("reviewer".into()))
        .await
        .unwrap();
    let held = reg.lane.expect("lane");
    assert_eq!((held.lane.version, held.text.as_str()), (1, "Read."));
    let page = w
        .rules
        .list_agents(&viewer, None, Some("reviewer".into()), 10, None)
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|a| a.handle.as_str())
            .collect::<Vec<_>>(),
        [reg.handle.as_str()]
    );
    let none = w
        .rules
        .list_agents(&viewer, None, Some("writer".into()), 10, None)
        .await
        .unwrap();
    assert!(none.items.is_empty());
    let view = w
        .rules
        .agent_view(&web_page(), reg.handle.clone())
        .await
        .unwrap();
    assert_eq!(view.agent.lane.expect("lane").name, "reviewer");
    // Reclaiming without a lane returns the one held, and says nothing.
    let reclaimed = w
        .rules
        .reclaim_with_client(reg.handle.clone(), reg.recovery_code, None, None)
        .await
        .unwrap();
    assert_eq!(reclaimed.lane.expect("lane").lane.name, "reviewer");
}

#[tokio::test]
async fn an_import_creates_new_lanes_versions_changed_ones_and_leaves_the_rest() {
    let w = world().await;
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read."),
        )
        .await
        .unwrap();
    w.rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("writer", "Writes.", "Write."),
        )
        .await
        .unwrap();
    w.clock.advance(Span::minutes(1));
    let done = w
        .operator
        .import_lanes(import(
            vec![
                lane("tester", "Tests.", "Test it."),
                lane("reviewer", "Reviews.", "Read twice."),
                lane("writer", "  Writes.  ", "Write."),
            ],
            false,
        ))
        .await
        .unwrap();
    assert_eq!(
        outcomes(&done),
        [
            ("tester", LaneOutcome::Created),
            ("reviewer", LaneOutcome::Updated(2)),
            ("writer", LaneOutcome::Unchanged(1)),
        ]
    );
    // Each imported version records its source, the human operator, the
    // time, and the server run the database belongs to.
    let reviewer = w
        .rules
        .lane_history(&web_page(), "reviewer".into())
        .await
        .unwrap();
    let source = Some(LaneSource {
        folder: "/home/sam/lanes".into(),
        commit: Some("abc123".into()),
    });
    assert_eq!(reviewer.versions[0].source, source);
    assert_eq!(reviewer.versions[0].saved_by_admin, None);
    assert_eq!(reviewer.versions[0].saved_at, "2026-10-23T14:01:00Z");
    assert_eq!(reviewer.versions[0].run_id, w.run_id);
    assert_eq!(reviewer.versions[1].source, None);
    let tester = w.rules.lane(&web_page(), "tester".into()).await.unwrap();
    assert_eq!((tester.current.version, tester.run_id), (1, w.run_id));
    assert_eq!(tester.current.source, source);
    let writer = w
        .rules
        .lane_history(&web_page(), "writer".into())
        .await
        .unwrap();
    assert_eq!(writer.versions.len(), 1);
    assert_eq!(writer.versions[0].source, None);
}

#[tokio::test]
async fn importing_the_same_lanes_twice_changes_nothing_the_second_time() {
    let w = world().await;
    let lanes = vec![
        lane("reviewer", "Reviews.", "Read.\n"),
        lane("writer", "Writes.", "Write."),
    ];
    w.operator
        .import_lanes(import(lanes.clone(), true))
        .await
        .unwrap();
    let before = w.rules.lanes(&web_page()).await.unwrap();
    w.clock.advance(Span::minutes(1));
    let again = w.operator.import_lanes(import(lanes, true)).await.unwrap();
    assert_eq!(
        outcomes(&again),
        [
            ("reviewer", LaneOutcome::Unchanged(1)),
            ("writer", LaneOutcome::Unchanged(1)),
        ]
    );
    assert_eq!(w.rules.lanes(&web_page(),).await.unwrap(), before);
}

#[tokio::test]
async fn missing_lanes_are_retired_only_when_asked_and_retired_ones_are_left_alone() {
    let w = world().await;
    for name in ["reviewer", "writer", "tester"] {
        w.rules
            .save_lane(Actor::Operator(web_page()), lane(name, "d", "t"))
            .await
            .unwrap();
    }
    w.rules
        .retire_lane(Actor::Operator(web_page()), "tester".into())
        .await
        .unwrap();
    let lanes = vec![lane("reviewer", "d", "t"), lane("tester", "d", "changed")];
    let kept = w
        .operator
        .import_lanes(import(lanes.clone(), false))
        .await
        .unwrap();
    assert_eq!(
        outcomes(&kept),
        [
            ("reviewer", LaneOutcome::Unchanged(1)),
            ("tester", LaneOutcome::RetiredSkipped),
        ]
    );
    assert!(
        w.rules
            .lane(&web_page(), "writer".into())
            .await
            .unwrap()
            .retired
            .is_none()
    );
    let tester = w
        .rules
        .lane_history(&web_page(), "tester".into())
        .await
        .unwrap();
    assert_eq!(tester.versions.len(), 1);

    let retired = w.operator.import_lanes(import(lanes, true)).await.unwrap();
    assert_eq!(outcomes(&retired)[2..], [("writer", LaneOutcome::Retired)]);
    let writer = w.rules.lane(&web_page(), "writer".into()).await.unwrap();
    let by = writer.retired.expect("retired");
    assert_eq!((by.by_admin, by.run_id), (None, w.run_id));
}

#[tokio::test]
async fn a_lane_whose_file_was_skipped_is_not_retired_and_is_named() {
    let w = world().await;
    for name in ["notes", "reviewer", "tester", "writer"] {
        w.rules
            .save_lane(Actor::Operator(web_page()), lane(name, "d", "t"))
            .await
            .unwrap();
    }
    w.rules
        .retire_lane(Actor::Operator(web_page()), "tester".into())
        .await
        .unwrap();
    // A skipped file for a lane that is retired, or that does not exist,
    // changes nothing and is not reported.
    let skipped_files = vec!["notes".to_string(), "tester".into(), "gone".into()];
    let kept = w
        .operator
        .import_lanes(LaneImport {
            skipped_files: skipped_files.clone(),
            ..import(vec![lane("reviewer", "d", "t")], false)
        })
        .await
        .unwrap();
    assert_eq!(outcomes(&kept), [("reviewer", LaneOutcome::Unchanged(1))]);

    let done = w
        .operator
        .import_lanes(LaneImport {
            skipped_files,
            ..import(vec![lane("reviewer", "d", "t")], true)
        })
        .await
        .unwrap();
    assert_eq!(
        outcomes(&done),
        [
            ("reviewer", LaneOutcome::Unchanged(1)),
            ("notes", LaneOutcome::FileSkipped),
            ("writer", LaneOutcome::Retired),
        ]
    );
    assert!(
        w.rules
            .lane(&web_page(), "notes".into())
            .await
            .unwrap()
            .retired
            .is_none()
    );
}

#[test]
fn retiring_is_refused_only_for_a_folder_with_no_lanes_but_skipped_files() {
    let error = check_retire_missing("/work/docs", 0, 1).expect_err("refused");
    assert_eq!(error.code, Code::InvalidArgument);
    assert_eq!(
        error.message,
        "/work/docs has no lane files, only skipped .md files, so --retire-missing would \
         retire every lane; name the folder that holds the lanes"
    );
    // A folder with a lane file, or with no .md files at all, may retire.
    for (lanes, skipped) in [(1, 0), (1, 3), (0, 0)] {
        assert_eq!(
            check_retire_missing("/work/lanes", lanes, skipped),
            Ok(()),
            "{lanes} {skipped}"
        );
    }
}

#[tokio::test]
async fn an_import_retiring_with_no_lanes_but_skipped_files_changes_nothing() {
    let w = world().await;
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "t"))
        .await
        .unwrap();
    let before = w.rules.lanes(&web_page()).await.unwrap();
    let error = w
        .operator
        .import_lanes(LaneImport {
            skipped_files: vec!["notes".into()],
            ..import(Vec::new(), true)
        })
        .await
        .expect_err("refused");
    assert_eq!(
        error,
        check_retire_missing("/home/sam/lanes", 0, 1).expect_err("refused")
    );
    assert_eq!(w.rules.lanes(&web_page(),).await.unwrap(), before);
    // Without retire_missing the same import is let through.
    let done = w
        .operator
        .import_lanes(LaneImport {
            skipped_files: vec!["notes".into()],
            ..import(Vec::new(), false)
        })
        .await
        .unwrap();
    assert!(done.is_empty());
}

#[tokio::test]
async fn an_import_with_changes_needs_a_server_run_on_the_database() {
    let clock = Arc::new(FixedClock::new(
        Time::from_parts(2026, 10, 23, 14, 0, 0).unwrap(),
    ));
    let operator = CommandLine::new(memory_db(), SwarmId::main(), clock);
    // Nothing to change needs no run.
    assert!(
        operator
            .import_lanes(import(Vec::new(), true))
            .await
            .unwrap()
            .is_empty()
    );
    let error = operator
        .import_lanes(import(vec![lane("reviewer", "d", "t")], false))
        .await
        .expect_err("no run");
    assert!(
        error.message.contains("start `kkt serve`"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn export_reads_every_lane_that_is_not_retired() {
    let w = world().await;
    for name in ["writer", "reviewer", "tester"] {
        w.rules
            .save_lane(Actor::Operator(web_page()), lane(name, "d", "t"))
            .await
            .unwrap();
    }
    w.rules
        .retire_lane(Actor::Operator(web_page()), "tester".into())
        .await
        .unwrap();
    let names: Vec<String> = w
        .operator
        .current_lanes()
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(names, ["reviewer", "writer"]);
}

/// Listing channels, and finding messages by search, activity and waits,
/// page and count in the store's queries, so they apply `access::decide` as
/// a scope (`access::scope`) that the store turns into SQL. This holds the
/// translation to `decide`: over every kind of channel, for an agent and the
/// human, and for every need, each query returns exactly the channels
/// `decide` allows.
#[tokio::test]
async fn the_stores_channel_filters_agree_with_decide() {
    use crate::{NewerFilter, SearchFilter, SentFilter};
    use access::{Channel, Decision, Need, Viewer, Who, decide};
    use std::collections::BTreeSet;
    const ME: &str = "me-agent";
    const OTHER: &str = "other-agent";
    const AT: &str = "2026-10-23T14:00:00Z";
    // (id, open, hidden, I am a member, I own it): every kind the store
    // tells apart.
    let mut kinds = Vec::new();
    for open in [true, false] {
        for hidden in [false, true] {
            for member in [false, true] {
                for owner in [false, true] {
                    let id = format!(
                        "ch_{}{}{}{}",
                        open as u8, hidden as u8, member as u8, owner as u8
                    );
                    kinds.push((id, open, hidden, member, owner));
                }
            }
        }
    }
    let db = memory_db();
    let setup = kinds.clone();
    db.write(move |w| {
        let run_id = w.insert_server_run(&crate::store::NewServerRunRecord {
            started_at: AT.into(),
            version: TEST_RUN.into(),
            git_commit: TEST_RUN.into(),
            settings: TEST_RUN.into(),
            instructions_sha256: TEST_RUN.into(),
        })?;
        for name in [ME, OTHER] {
            w.insert_agent(
                &SwarmId::main(),
                &NewAgentRecord {
                    name: name.into(),
                    token_hash: name.as_bytes().to_vec(),
                    recovery_hash: None,
                    created_at: AT.into(),
                    last_seen: None,
                    run_id,
                },
            )?;
        }
        for (id, open, hidden, member, owner) in &setup {
            w.insert_channel(
                &SwarmId::main(),
                &NewChannelRecord {
                    id: id.clone(),
                    name: id.clone(),
                    purpose: None,
                    open: *open,
                    created_by: OTHER.into(),
                    created_at: AT.into(),
                    owner: None,
                    run_id,
                },
            )?;
            w.add_member(&SwarmId::main(), id, OTHER, AT)?;
            w.insert_message(
                &SwarmId::main(),
                &NewMessageRecord {
                    channel_id: id.clone(),
                    agent: OTHER.into(),
                    body: "findable".into(),
                    created_at: AT.into(),
                    run_id,
                },
            )?;
            if *hidden {
                w.hide_channel(&SwarmId::main(), id)?;
            }
            // Memberships and ownership are set after hiding, so the hidden
            // filter is tested even for rows the rules never leave behind.
            if *member {
                w.add_member(&SwarmId::main(), id, ME, AT)?;
            }
            w.set_owner(&SwarmId::main(), id, Some(if *owner { ME } else { OTHER }))?;
        }
        Ok::<_, crate::StoreError>(())
    })
    .await
    .unwrap();

    let allowed = |viewer: Viewer, need: Need| -> BTreeSet<String> {
        kinds
            .iter()
            .filter(|(_, open, hidden, member, owner)| {
                let channel = if *hidden {
                    Channel::Hidden
                } else {
                    Channel::Live { open: *open }
                };
                let who = match viewer {
                    Viewer::Agent(_) => Who::Agent {
                        member: *member,
                        owner: *owner,
                    },
                    Viewer::Operator => Who::Operator,
                };
                decide(channel, who, need) == Decision::Allowed
            })
            .map(|(id, ..)| id.clone())
            .collect()
    };
    let me = Viewer::Agent(ME);
    // The cases are not all alike, so the comparison means something.
    let sizes = [
        allowed(me, Need::Ownership).len(),
        allowed(me, Need::Membership).len(),
        allowed(me, Need::See).len(),
        allowed(Viewer::Operator, Need::See).len(),
    ];
    assert!(
        sizes[0] > 0 && sizes.windows(2).all(|w| w[0] < w[1]),
        "{sizes:?}"
    );

    for viewer in [me, Viewer::Operator] {
        for need in [Need::See, Need::Membership, Need::Ownership] {
            let want = allowed(viewer, need);
            let scope = access::scope(&SwarmId::main(), viewer, need);
            let each = kinds.clone();
            let found = db
                .read(move |s| {
                    let listed = s.channel_ids(&ChannelFilter {
                        scope: scope.clone(),
                        shared_with: None,
                        text: None,
                        after: None,
                        limit: 1000,
                    })?;
                    let newer = NewerFilter {
                        scope: scope.clone(),
                        after_id: 0,
                        channel_ids: None,
                    };
                    let waited = s.newer_messages(&newer, 1000)?;
                    let counted = s.count_newer_messages(&newer)?;
                    let searched = s.search(&SearchFilter {
                        scope: scope.clone(),
                        words: vec!["findable".into()],
                        phrase: None,
                        channel_id: None,
                        from_agent: None,
                        after_id: None,
                        before_id: None,
                        since: None,
                        until: None,
                        limit: 1000,
                    })?;
                    let active = s.activity(&scope, 0)?;
                    // An agent's latest message in each channel.
                    let mut sent_in = Vec::new();
                    for (id, ..) in &each {
                        let latest = s.latest_message_from(&SentFilter {
                            agent: OTHER.into(),
                            channel_id: Some(id.clone()),
                            scope: scope.clone(),
                        })?;
                        sent_in.extend(latest.map(|m| m.channel_id));
                    }
                    // And in any channel: the newest in the scope.
                    let sent_anywhere = s.latest_message_from(&SentFilter {
                        agent: OTHER.into(),
                        channel_id: None,
                        scope,
                    })?;
                    Ok::<_, crate::StoreError>((
                        listed,
                        waited,
                        counted,
                        searched,
                        active,
                        sent_in,
                        sent_anywhere,
                    ))
                })
                .await
                .unwrap();
            let (listed, waited, counted, searched, active, sent_in, sent_anywhere) = found;
            let ids = |v: Vec<String>| v.into_iter().collect::<BTreeSet<_>>();
            let case = format!("viewer={viewer:?} need={need:?}");
            assert_eq!(ids(listed), want, "list_channels, {case}");
            assert_eq!(
                ids(waited.into_iter().map(|m| m.channel_id).collect()),
                want,
                "waits, {case}"
            );
            assert_eq!(counted, want.len() as i64, "wait counts, {case}");
            assert_eq!(
                ids(searched.into_iter().map(|h| h.channel_id).collect()),
                want,
                "search, {case}"
            );
            assert_eq!(
                ids(active.into_iter().map(|a| a.channel_id).collect()),
                want,
                "activity, {case}"
            );
            assert_eq!(ids(sent_in), want, "last_sent in each channel, {case}");
            // Messages were sent in `kinds` order, so the newest one in the
            // scope is the last kind in it.
            let newest = kinds
                .iter()
                .rev()
                .find(|(id, ..)| want.contains(id))
                .map(|(id, ..)| id.clone());
            assert_eq!(
                sent_anywhere.map(|m| m.channel_id),
                newest,
                "last_sent in any channel, {case}"
            );
        }
    }

    // The operator's waits read every channel, hidden ones included: what
    // `decide` lets the human see.
    let any = db
        .read(|s| s.any_newer_messages(&SwarmId::main(), 0, None, 1000))
        .await
        .unwrap();
    assert_eq!(
        any.into_iter()
            .map(|m| m.channel_id)
            .collect::<BTreeSet<_>>(),
        allowed(Viewer::Operator, Need::See),
        "the operator's waits"
    );
}

#[tokio::test]
async fn rules_bound_to_another_swarm_find_nothing_of_this_one() {
    let w = world().await;
    let other = other_swarm(&w).await;
    let reg = w.rules.register().await.unwrap();
    let mine = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    let open = w.channel(&mine, "open talk", true, &[]).await;
    let closed = w.channel(&mine, "closed talk", false, &[]).await;
    w.rules
        .send(&mine, open.clone(), "hello from home".into())
        .await
        .unwrap();

    let theirs_reg = other.register().await.unwrap();
    let theirs = other.authenticate(Some(&theirs_reg.token)).await.unwrap();

    // This swarm's token and recovery code mean nothing there.
    assert_eq!(
        code(other.authenticate(Some(&reg.token)).await),
        Code::Unauthenticated
    );
    assert_eq!(
        code(w.rules.authenticate(Some(&theirs_reg.token)).await),
        Code::Unauthenticated
    );
    let refused = code(
        other
            .reclaim(reg.handle.clone(), reg.recovery_code.clone())
            .await,
    );
    let unknown = code(other.reclaim("no-such-avatar".into(), "arc_x".into()).await);
    assert_eq!(refused, unknown);

    // Its channels and avatars do not exist there.
    for id in [&open, &closed] {
        assert_eq!(
            code(other.join_channel(&theirs, id.clone()).await),
            Code::NotFound
        );
        assert_eq!(
            code(other.send(&theirs, id.clone(), "hi".into()).await),
            Code::NotFound
        );
    }
    let query = ChannelQuery {
        query: None,
        member_only: false,
        with_agent: None,
        after_id: None,
        limit: 100,
        cursor: None,
    };
    let listed = other.list_channels(&theirs, query.clone()).await.unwrap();
    assert!(listed.items.is_empty(), "{:?}", listed.items);
    let agents = other
        .list_agents(&theirs, None, None, 100, None)
        .await
        .unwrap();
    let handles: Vec<_> = agents.items.iter().map(|a| a.handle.as_str()).collect();
    assert_eq!(handles, [theirs.handle.as_str()]);
    let hits = other
        .search(
            &theirs,
            SearchQuery {
                query: Some("hello".into()),
                phrase: None,
                channel_id: None,
                from_agent: None,
                after_id: None,
                before_id: None,
                since: None,
                until: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(hits.is_empty(), "{hits:?}");

    // Handles are unique on the server, so a handle taken in one swarm is
    // taken in every swarm: the one thing a swarm learns of another.
    let taken = other
        .register_with_client(None, Some(reg.handle.clone()), None)
        .await;
    assert_eq!(code(taken), Code::HandleTaken);

    // And the home swarm still sees its own.
    let listed = w.rules.list_channels(&mine, query).await.unwrap();
    assert_eq!(listed.items.len(), 2);
}

#[tokio::test]
async fn the_human_of_another_swarm_finds_a_hidden_channel_as_missing() {
    let w = world().await;
    let other = other_swarm(&w).await;
    let mine = w.agent().await;
    let hidden = w.channel(&mine, "hidden talk", true, &[]).await;
    w.rules.hide_channel(&mine, hidden.clone()).await.unwrap();
    let missing = "ch_neverwas".to_string();
    let same_refusal = |a: RuleError, b: RuleError| {
        assert_eq!(a.code, b.code);
        assert_eq!(
            a.message.replace(&hidden, "ID"),
            b.message.replace(&missing, "ID")
        );
    };
    same_refusal(
        other
            .channel_view(&web_page(), hidden.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap_err(),
        other
            .channel_view(&web_page(), missing.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap_err(),
    );
    same_refusal(
        other
            .set_purpose(Actor::Operator(web_page()), hidden.clone(), "x".into())
            .await
            .unwrap_err(),
        other
            .set_purpose(Actor::Operator(web_page()), missing.clone(), "x".into())
            .await
            .unwrap_err(),
    );
    // In its own swarm the human tells it apart: it can read it.
    assert!(
        w.rules
            .channel_view(&web_page(), hidden, None, PAGE_MESSAGES)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn an_operator_without_read_is_refused_every_operator_read() {
    let w = world().await;
    let a = w.agent().await;
    let room = w.channel(&a, "room", true, &[]).await;
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "t"))
        .await
        .unwrap();
    let blind = Operator::new(
        Permissions::of(&[Permission::Change, Permission::IssueRecoveryCodes]),
        Audit::WebPage,
    );
    let r = &w.rules;
    let o = &blind;
    let read = ReadQuery {
        channel_id: room.clone(),
        position: Position::Latest,
        from_agent: None,
        limit: 10,
    };
    let refused: Vec<Result<()>> = vec![
        r.overview(o, ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .map(drop),
        r.hidden_channels(o, ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .map(drop),
        r.server_runs(o).await.map(drop),
        r.channel_view(o, room.clone(), None, PAGE_MESSAGES)
            .await
            .map(drop),
        r.agent_view(o, a.handle.clone()).await.map(drop),
        r.stale_agents(o, Duration::from_secs(3600)).await.map(drop),
        r.lanes(o).await.map(drop),
        r.lanes_page(o, 10, None).await.map(drop),
        r.lane(o, "reviewer".into()).await.map(drop),
        r.lane_history(o, "reviewer".into()).await.map(drop),
        r.server_announcement_history(o).await.map(drop),
        r.admin_list_channels(o, None, 10, None).await.map(drop),
        r.admin_channel(o, room.clone()).await.map(drop),
        r.admin_list_members(o, room.clone(), 10, None)
            .await
            .map(drop),
        r.admin_note(o, room.clone()).await.map(drop),
        r.admin_read(o, read).await.map(drop),
        r.admin_message(o, 1).await.map(drop),
        r.admin_activity(o, 0).await.map(drop),
        r.admin_new_messages(o, 0, None, 10).await.map(drop),
        r.admin_list_agents(o, None, None, 10, None).await.map(drop),
        r.admin_agent(o, a.handle.clone()).await.map(drop),
    ];
    for result in refused {
        lacks(result, Permission::Read);
    }
    // With read, the same operator sees.
    assert!(
        r.overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn only_an_operator_that_may_issue_recovery_codes_issues_one() {
    let w = world().await;
    let a = w.agent().await;
    let named =
        admin_named(names::AdminName::new("sentinel", kkt_names::MAX_ADMIN_NAME_CHARS).unwrap());
    lacks(
        w.rules
            .new_recovery_code(&named, a.handle.clone())
            .await
            .map(drop),
        Permission::IssueRecoveryCodes,
    );
    lacks(
        w.rules
            .new_recovery_code(&unnamed_admin(), a.handle.clone())
            .await
            .map(drop),
        Permission::IssueRecoveryCodes,
    );
    let code = w
        .rules
        .new_recovery_code(&web_page(), a.handle.clone())
        .await
        .unwrap();
    assert!(code.starts_with("arc_"));
}

#[tokio::test]
async fn hiding_an_owner_is_refused_naming_the_channels_for_each_edge_to_word() {
    let w = world().await;
    let a = w.agent().await;
    let first = w.channel(&a, "first", true, &[]).await;
    let second = w.channel(&a, "second", false, &[]).await;
    let e = w
        .rules
        .hide_agent(Actor::Operator(web_page()), a.handle.clone())
        .await
        .unwrap_err();
    assert_eq!(e.code, Code::InvalidArgument);
    let mut owned = vec![first, second];
    owned.sort();
    let Some(Refusal::OwnsChannels {
        handle,
        mut channels,
    }) = e.refusal
    else {
        panic!("not an ownership refusal: {e:?}");
    };
    channels.sort();
    assert_eq!((handle, channels), (a.handle.clone(), owned));
    assert!(
        e.message.ends_with("or clear the owner, first"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn a_subscription_hears_only_its_own_swarm() {
    let w = world().await;
    let other = other_swarm(&w).await;
    let mut home = w.rules.subscribe();
    let mut away = other.subscribe();
    let theirs = agent_in(&other).await;
    let ch = open_channel_in(&other, &theirs, "away talk").await;
    other
        .send(&theirs, ch, "not for home".into())
        .await
        .unwrap();
    let mine = w.agent().await;

    // Home hears its own agent first, not the other swarm's agent, channel
    // or message.
    assert!(
        matches!(home.next().await, Some(Notice::Agent(a)) if a.handle == mine.handle),
        "home heard another swarm"
    );
    // The other swarm hears its own, in order, and not home's agent.
    assert!(matches!(away.next().await, Some(Notice::Agent(a)) if a.handle == theirs.handle));
    assert!(matches!(away.next().await, Some(Notice::Channel(c)) if c.name == "away talk"));
    assert!(matches!(away.next().await, Some(Notice::Message(m)) if m.body == "not for home"));
}

#[tokio::test]
async fn a_closed_subscription_ends() {
    let w = world().await;
    let mut sub = w.rules.subscribe();
    let closer = sub.closer();
    closer.close();
    w.agent().await;
    assert!(sub.next().await.is_none());
    // Others are not closed with it.
    let mut other = w.rules.subscribe();
    let mine = w.agent().await;
    assert!(matches!(other.next().await, Some(Notice::Agent(a)) if a.handle == mine.handle));
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "bounds the real wait for the spawned wait to end, so a failure ends instead of hanging"
)]
async fn a_wait_hears_nothing_from_another_swarm() {
    let w = world().await;
    let other = other_swarm(&w).await;
    let b = w.agent().await;
    let waiter = {
        let rules = w.rules.clone();
        let b = b.clone();
        tokio::spawn(async move {
            rules
                .wait_for_messages(
                    &b,
                    WaitQuery {
                        after_id: 0,
                        channel_ids: None,
                        timeout: Some(Duration::from_secs(30)),
                        limit: 10,
                    },
                )
                .await
        })
    };
    until_sleeping(&w.clock).await;
    let theirs = agent_in(&other).await;
    let ch = open_channel_in(&other, &theirs, "away talk").await;
    other.send(&theirs, ch, "not for b".into()).await.unwrap();
    w.clock.advance(Span::seconds(30));
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the wait should time out")
        .expect("join")
        .expect("wait");
    assert!(got.messages.is_empty(), "{:?}", got.messages);
}

/// The operator's notices, through kkt-api-operator's contract: they hear
/// every channel of the swarm, closed ones included, and nothing of
/// another swarm; an operator who may not read hears nothing.
#[tokio::test]
async fn the_operators_notices_hear_every_channel_of_its_swarm_only() {
    use kkt_api_operator::{Notice as Heard, Notices, Views};
    let w = world().await;
    let other = other_swarm(&w).await;
    let home: &dyn Views = &w.rules;
    let away: &dyn Views = &other;
    let mut home_notices: Box<dyn Notices> = home.subscribe(&web_page()).await.expect("home");
    let mut away_notices: Box<dyn Notices> = away.subscribe(&web_page()).await.expect("away");
    let no_read = Operator::new(Permissions::of(&[]), Audit::WebPage);
    assert_eq!(
        home.subscribe(&no_read).await.err().map(|e| e.code),
        Some(Code::Unauthenticated)
    );

    let theirs = agent_in(&other).await;
    let ch = open_channel_in(&other, &theirs, "away talk").await;
    other
        .send(&theirs, ch, "not for home".into())
        .await
        .unwrap();
    let mine = w.agent().await;
    let closed = w
        .rules
        .create_channel(
            &mine,
            NewChannel {
                name: "closed room".into(),
                open: false,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("create");
    w.rules
        .send(&mine, closed.id.clone(), "for home".into())
        .await
        .unwrap();

    assert!(
        matches!(home_notices.next().await, Some(Heard::Agent(a)) if a.handle == mine.handle),
        "home heard another swarm"
    );
    assert!(
        matches!(home_notices.next().await, Some(Heard::Channel(c)) if c.id == closed.id && !c.open)
    );
    assert!(
        matches!(home_notices.next().await, Some(Heard::Message(m)) if m.body == "for home" && m.channel_id == closed.id)
    );
    assert!(
        matches!(away_notices.next().await, Some(Heard::Agent(a)) if a.handle == theirs.handle)
    );
    assert!(matches!(away_notices.next().await, Some(Heard::Channel(c)) if c.name == "away talk"));
    assert!(
        matches!(away_notices.next().await, Some(Heard::Message(m)) if m.body == "not for home")
    );
}

/// An operator's notices that fall behind say so, with no count, and go on
/// with what is new.
#[tokio::test]
async fn the_operators_notices_say_they_fell_behind_without_a_count() {
    use kkt_api_operator::{Notice as Heard, Views};
    let w = world().await;
    let rules = Rules::new(Deps {
        db: w.rules.unbound_db().clone(),
        swarm: SwarmId::main(),
        events: Events::new(kkt_events::Settings { buffer: 1 }),
        clock: w.clock.clone(),
        entropy: Arc::new(SeqEntropy::new(7)),
        run_id: w.run_id,
        settings: Settings::DEFAULT,
        wording: Arc::new(DefaultWording),
    });
    let views: &dyn Views = &rules;
    let mut notices = views.subscribe(&web_page()).await.expect("subscribe");
    let mut handles = Vec::new();
    for _ in 0..3 {
        handles.push(rules.register().await.expect("register").handle);
    }
    assert_eq!(notices.next().await, Some(Heard::Missed));
    assert!(
        matches!(notices.next().await, Some(Heard::Agent(a)) if Some(&a.handle) == handles.last())
    );
}

/// The rules meet kkt-api-operator's contract through `dyn`, and each
/// operation gives what the rules' own method gives, permission checks
/// included.
#[tokio::test]
async fn the_operator_contract_is_the_rules_own_operations() {
    use kkt_api_operator::{Lanes, Server, Views};
    let w = world().await;
    let web = web_page();
    let server: &dyn Server = &w.rules;
    let runs = server.server_runs(&web).await.expect("runs");
    let own = w.rules.server_runs(&web).await.expect("runs");
    assert_eq!(
        runs.iter().map(|r| r.id).collect::<Vec<_>>(),
        own.iter().map(|r| r.id).collect::<Vec<_>>()
    );
    let lanes: &dyn Lanes = &w.rules;
    assert_eq!(
        lanes.lanes(&web).await.expect("lanes"),
        w.rules.lanes(&web).await.expect("lanes")
    );
    let views: &dyn Views = &w.rules;
    let no_read = Operator::new(Permissions::of(&[]), Audit::WebPage);
    assert_eq!(
        code(views.agent_view(&no_read, "nobody".into()).await),
        Code::Unauthenticated
    );
}

/// The rules meet kkt-api-agent's contract through `dyn`: `in_swarm` gives
/// the operations in that swarm, `limits` reports the settings' limits,
/// and each operation answers as the rules' own method does.
#[tokio::test]
async fn the_agent_contract_is_the_rules_own_operations() {
    use kkt_api_agent::Api;
    let w = world().await;
    other_swarm(&w).await;
    let api: &dyn Api = &w.rules;
    assert_eq!(api.limits(), w.rules.settings().limits());
    let reg = w.rules.register().await.unwrap();
    let me = api.authenticate(Some(&reg.token)).await.expect("my token");
    assert_eq!(me.handle, reg.handle);
    let elsewhere = api.in_swarm(SwarmId::new("sw_other"));
    assert_eq!(
        code(elsewhere.authenticate(Some(&reg.token)).await),
        Code::Unauthenticated,
        "another swarm does not know this avatar"
    );
    let channel = api
        .create_channel(
            &me,
            NewChannel {
                name: "through the contract".into(),
                open: true,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("a channel");
    let sent = api
        .send(&me, channel.id.clone(), "hello".into())
        .await
        .expect("sent");
    assert_eq!(
        api.get_message(&me, sent.id).await.expect("read back").body,
        "hello"
    );
}

/// The rules meet kkt-api-admin's contracts through `dyn`: `create_swarm`
/// makes a swarm, `in_swarm_admin` acts in it, and the admin reads answer
/// as the rules' own methods do.
#[tokio::test]
async fn the_admin_contract_is_the_rules_own_operations() {
    use kkt_api_admin::{Admin, AdminApi, Swarms};
    let w = world().await;
    let swarms: &dyn Swarms = &w.rules;
    swarms
        .create_swarm(SwarmId::new("sw_made"))
        .await
        .expect("a new swarm");
    let admin: &dyn AdminApi = &w.rules;
    let made = admin.in_swarm_admin(SwarmId::new("sw_made"));
    let reg = made.register_with_client(None, None, None).await.unwrap();
    let web = web_page();
    let agents = made
        .admin_list_agents(&web, None, None, 10, None)
        .await
        .expect("the new swarm's agents");
    assert_eq!(
        agents
            .items
            .iter()
            .map(|a| a.handle.as_str())
            .collect::<Vec<_>>(),
        [reg.handle.as_str()]
    );
    let home: &dyn Admin = &w.rules;
    let own = home
        .admin_list_agents(&web, None, None, 10, None)
        .await
        .expect("this swarm's agents");
    assert!(
        own.items.iter().all(|a| a.handle != reg.handle),
        "an agent of the new swarm is not in this one"
    );
}

/// The scenarios the program also runs on the real backend, here on the
/// stand-in.
#[tokio::test(flavor = "multi_thread")]
async fn the_shared_scenarios_hold_on_the_stand_in() {
    run_all(fast_backend).await;
}
