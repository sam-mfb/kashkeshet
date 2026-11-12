//! The rules' scenarios that run on the real backend too: those whose
//! answers rest on how a database answers (refusals from its rules, paging
//! and order, search, time windows, waits, failed writes) and that no
//! end-to-end or contract test asserts there (11.3). They run here, on
//! kkt-db's fast stand-in, from the crate's own tests, and on kkt-db-sqlite
//! from the program's (`tests/rules_on_sqlite.rs`), through
//! [`run_all`]. With them are the helpers every rules test uses.
//!
//! Each scenario makes its own world over a new database from the backend
//! [`run_all`] is given; outside it, the world is on the stand-in.

// The helpers serve the crate's own tests too; built for `testing` alone,
// some are not used.
#![cfg_attr(not(test), allow(dead_code))]

use super::*;
use futures::FutureExt;
use kkt_clock::testing::FixedClock;
use kkt_entropy::testing::{SameHandleEntropy, SeqEntropy};
use kkt_events::Notice;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

tokio::task_local! {
    /// Where the scenario running now makes its databases.
    static BACKEND: Arc<dyn Fn() -> Arc<dyn kkt_db::Backend> + Send + Sync>;
}

/// A scenario, as [`run_all`] runs it.
type Scenario = fn() -> Pin<Box<dyn Future<Output = ()> + Send>>;

macro_rules! scenarios {
    ($($name:ident),* $(,)?) => {
        [$((stringify!($name), (|| Box::pin($name())) as Scenario)),*]
    };
}

/// Runs every scenario, each over new databases that `new_backend` makes
/// with the core's tables, as a database the program opens is. A failing
/// scenario panics, naming itself.
pub async fn run_all(new_backend: impl Fn() -> Arc<dyn kkt_db::Backend> + Send + Sync + 'static) {
    let new_backend: Arc<dyn Fn() -> Arc<dyn kkt_db::Backend> + Send + Sync> =
        Arc::new(new_backend);
    for (name, scenario) in scenarios![
        name_clashes_get_a_number,
        a_proposed_name_any_agent_has_or_had_is_taken_whatever_its_case,
        a_badly_shaped_or_reserved_name_is_refused_and_registers_nothing,
        last_seen_records_every_authenticated_call,
        list_agents_pages_by_name,
        an_unregistered_owner_leaves_no_owner_and_the_human_still_removes,
        the_web_channel_page_holds_web_page_messages_and_pages_back,
        the_front_page_summarizes_each_channel_and_agent,
        recent_counts_are_exact_for_every_channel,
        recent_counts_do_not_assume_times_follow_message_order,
        list_channels_filters_and_pages,
        list_channels_counts_new_messages,
        a_failed_send_does_not_use_up_the_allowance,
        read_pages_both_ways,
        read_filters_by_agent,
        search_covers_only_my_channels,
        search_phrase_needs_words_together_and_in_order,
        search_needs_query_or_phrase,
        search_id_ranges_exclude_their_edges,
        search_time_ranges_include_since_and_exclude_until,
        search_refuses_times_it_cannot_read,
        search_ranges_never_reach_closed_channels_i_am_not_in,
        activity_reports_channels_with_newer_messages,
        wait_returns_at_once_when_messages_exist,
        a_wait_longer_than_max_wait_is_cut_to_max_wait,
        a_wait_with_no_timeout_lasts_default_wait,
        the_operator_lists_agents_with_their_revocations,
        reclaim_preserves_the_avatar_history_membership_and_provenance,
        listening_covers_selected_channels_and_hides_closed_channels,
        normal_wait_completion_has_grace_but_aborted_wait_does_not,
        overlapping_waits_and_current_memberships_are_independent,
        a_new_wait_supersedes_grace_and_disconnect_clears_it,
        hiding_an_agent_revokes_it_announces_each_membership_and_keeps_its_name_reserved,
        administration_rolls_back_when_the_operator_message_cannot_be_saved,
        a_hidden_channel_id_is_skipped_when_entropy_draws_it_again,
        an_entry_over_the_entry_cap_is_refused_and_the_old_one_kept,
        a_memory_over_the_avatar_cap_is_refused_counting_a_replaced_entry_once,
        two_sessions_saving_different_entries_keep_both,
        memory_lists_page_by_name,
        server_announcement_saves_are_immutable_versions_with_clock_and_run,
        concurrent_announcement_saves_get_distinct_increasing_versions,
        operator_channel_creation_is_atomic_and_never_adds_the_operator,
        stale_agents_are_quiet_longer_than_the_period_and_not_waiting,
        stale_agents_are_listed_longest_quiet_first,
        saving_what_the_lane_already_says_adds_no_version,
        a_retired_lane_is_kept_with_its_history_and_cannot_be_changed,
        concurrent_lane_saves_get_distinct_increasing_versions,
        list_lanes_for_agents_leaves_out_retired_lanes_and_pages_by_name,
        one_bad_lane_or_a_lane_given_twice_stops_the_import_with_nothing_changed,
        last_sent_counts_only_messages_the_viewer_can_see,
    ] {
        let ran = std::panic::AssertUnwindSafe(BACKEND.scope(new_backend.clone(), scenario()))
            .catch_unwind()
            .await;
        if let Err(failure) = ran {
            let said = failure
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| failure.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            panic!("scenario {name} failed: {said}");
        }
    }
}

/// The web page's page size, as kkt-web's default.
pub(crate) const PAGE_MESSAGES: usize = 50;
/// The activity window the web page asks the overview for, as kkt-web's.
pub(crate) const ACTIVITY_WINDOW: Duration = Duration::from_secs(5 * 60);
/// The activity bucket the web page asks the overview for, as kkt-web's.
pub(crate) const ACTIVITY_BUCKET: Duration = Duration::from_secs(10);

/// The web page's operator: every permission.
pub(crate) fn web_page() -> Operator {
    Operator::new(
        Permissions::of(&[
            Permission::Read,
            Permission::Change,
            Permission::IssueRecoveryCodes,
        ]),
        Audit::WebPage,
    )
}

/// An admin agent under a named admin key: read and change.
pub(crate) fn admin_named(name: names::AdminName) -> Operator {
    Operator::new(
        Permissions::of(&[Permission::Read, Permission::Change]),
        Audit::AdminEndpoint {
            key_name: Some(name.to_string()),
        },
    )
}

/// An admin agent under an admin key with no name: read only.
pub(crate) fn unnamed_admin() -> Operator {
    Operator::new(
        Permissions::of(&[Permission::Read]),
        Audit::AdminEndpoint { key_name: None },
    )
}

pub(crate) struct World {
    pub(crate) rules: Rules,
    pub(crate) clock: Arc<FixedClock>,
    pub(crate) run_id: i64,
    /// The operator, on the same database.
    pub(crate) operator: CommandLine,
    /// The world's database, to make a change to it fail on cue.
    pub(crate) faults: kkt_db::testing::FaultyBackend,
}

/// The run every test world records, so stamped rows can be checked.
pub(crate) const TEST_RUN: &str = "test-run";

/// A fresh database in memory, with the core's tables.
pub(crate) fn memory_db() -> Db {
    Db::new(memory_backend())
}

pub(crate) fn memory_backend() -> Arc<dyn kkt_db::Backend> {
    BACKEND
        .try_with(|new_backend| new_backend())
        .unwrap_or_else(|_| fast_backend())
}

/// A new database on kkt-db's fast stand-in, with the core's tables.
pub(crate) fn fast_backend() -> Arc<dyn kkt_db::Backend> {
    let place = kkt_db::testing::FastPlace::new().expect("an in-memory database");
    place.open(&[crate::schema()]).expect("the core's tables")
}

pub(crate) async fn world_with(settings: Settings, entropy: Arc<dyn Entropy>) -> World {
    let clock = Arc::new(FixedClock::new(
        Time::from_parts(2026, 10, 23, 14, 0, 0).unwrap(),
    ));
    let faults = kkt_db::testing::FaultyBackend::new(memory_backend());
    let db = Db::new(Arc::new(faults.clone()));
    let run_id = db
        .write(|w| {
            w.insert_server_run(&crate::store::NewServerRunRecord {
                started_at: "2026-10-23T14:00:00Z".into(),
                version: TEST_RUN.into(),
                git_commit: TEST_RUN.into(),
                settings: TEST_RUN.into(),
                instructions_sha256: TEST_RUN.into(),
            })
        })
        .await
        .expect("record run");
    let operator = CommandLine::new(db.clone(), SwarmId::main(), clock.clone());
    let rules = Rules::new(Deps {
        db,
        swarm: SwarmId::main(),
        events: Events::new(kkt_events::Settings::DEFAULT),
        clock: clock.clone(),
        entropy,
        run_id,
        settings,
        wording: Arc::new(DefaultWording),
    });
    World {
        rules,
        clock,
        run_id,
        operator,
        faults,
    }
}

pub(crate) async fn world() -> World {
    world_with(Settings::DEFAULT, Arc::new(SeqEntropy::new(42))).await
}

impl World {
    pub(crate) async fn agent(&self) -> Agent {
        let reg = self.rules.register().await.expect("register");
        self.rules
            .authenticate(Some(&reg.token))
            .await
            .expect("auth")
    }

    pub(crate) async fn channel(
        &self,
        owner: &Agent,
        name: &str,
        open: bool,
        members: &[&Agent],
    ) -> String {
        self.rules
            .create_channel(
                owner,
                NewChannel {
                    name: name.into(),
                    open,
                    purpose: None,
                    members: members.iter().map(|a| a.handle.clone()).collect(),
                },
            )
            .await
            .expect("create")
            .id
    }

    pub(crate) async fn visible_ids(&self, me: &Agent) -> Vec<String> {
        self.rules
            .list_channels(
                me,
                ChannelQuery {
                    limit: 100,
                    ..Default::default()
                },
            )
            .await
            .expect("list")
            .items
            .into_iter()
            .map(|c| c.id)
            .collect()
    }

    pub(crate) async fn read_latest(&self, me: &Agent, ch: &str) -> Result<MessagePage> {
        self.rules
            .read(
                me,
                ReadQuery {
                    channel_id: ch.into(),
                    position: Position::Latest,
                    from_agent: None,
                    limit: 20,
                },
            )
            .await
    }
}

pub(crate) fn code<T: std::fmt::Debug>(r: Result<T>) -> Code {
    r.expect_err("expected an error").code
}

pub(crate) async fn register_as(w: &World, name: &str) -> Result<Registration> {
    w.rules
        .register_with_client(None, Some(name.into()), None)
        .await
}

/// A search for `query` alone, with nothing else narrowing it.
pub(crate) fn words(query: &str) -> SearchQuery {
    SearchQuery {
        query: Some(query.into()),
        phrase: None,
        channel_id: None,
        from_agent: None,
        after_id: None,
        before_id: None,
        since: None,
        until: None,
        limit: 10,
    }
}

/// The IDs a search finds, newest first.
pub(crate) async fn found(w: &World, me: &Agent, q: SearchQuery) -> Vec<i64> {
    w.rules
        .search(me, q)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.message_id)
        .collect()
}

/// Waits (in real time, briefly) until a wait has started sleeping on the test clock.
#[expect(
    clippy::disallowed_methods,
    reason = "bounds a real wait for the spawned wait to start, so a failure ends instead of hanging"
)]
pub(crate) async fn until_sleeping(clock: &FixedClock) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while clock.sleepers() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the wait should start sleeping");
}

/// Checks that a wait asked to last `timeout`, with nothing arriving, ends
/// when the test clock has moved `lasts` seconds and not a second sooner.
/// The server waits 5 seconds by default and at most 9.
#[expect(
    clippy::disallowed_methods,
    reason = "a short real wait shows the spawned wait has not ended; a longer one bounds it"
)]
pub(crate) async fn wait_lasts(timeout: Option<Duration>, lasts: i64) {
    let settings = Settings {
        default_wait: Duration::from_secs(5),
        max_wait: Duration::from_secs(9),
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let a = w.agent().await;
    let mut waiter = {
        let rules = w.rules.clone();
        let a = a.clone();
        tokio::spawn(async move {
            rules
                .wait_for_messages(
                    &a,
                    WaitQuery {
                        after_id: 0,
                        channel_ids: None,
                        timeout,
                        limit: 10,
                    },
                )
                .await
        })
    };
    until_sleeping(&w.clock).await;
    w.clock.advance(Span::seconds(lasts - 1));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiter)
            .await
            .is_err(),
        "the wait ended before {lasts} seconds"
    );
    w.clock.advance(Span::seconds(1));
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the wait should end")
        .expect("join")
        .expect("wait");
    assert!(got.messages.is_empty());
}

/// The notices other than messages heard so far, one line each, saying who
/// did what.
#[expect(
    clippy::disallowed_methods,
    reason = "notices have no clock; a short real wait shows nothing more is coming"
)]
pub(crate) async fn heard_changes(notices: &mut kkt_events::Subscription) -> Vec<String> {
    let mut heard = Vec::new();
    while let Ok(Some(notice)) =
        tokio::time::timeout(Duration::from_millis(50), notices.next()).await
    {
        match notice {
            Notice::Message(_) | Notice::Agent(_) | Notice::Channel(_) | Notice::Joined(_) => {}
            Notice::Left(x) => heard.push(format!(
                "left {} {} {} {} by {} {}",
                x.agent, x.channel_id, x.channel_name, x.member_count, x.by, x.by_operator
            )),
            Notice::ChannelHidden(x) => heard.push(format!(
                "channel_hidden {} {} by {} {}",
                x.channel_id, x.channel_name, x.by, x.by_operator
            )),
            Notice::AgentHidden(x) => {
                let channels: Vec<String> = x
                    .channels
                    .iter()
                    .map(|c| format!("{}={}={}", c.channel_id, c.channel_name, c.member_count))
                    .collect();
                heard.push(format!(
                    "agent_hidden {} by {} {} [{}]",
                    x.agent,
                    x.by,
                    x.by_operator,
                    channels.join(" ")
                ));
            }
            Notice::Missed => panic!("missed some"),
        }
    }
    heard
}

pub(crate) async fn observed(w: &World, viewer: &Agent, name: &str) -> AgentInfo {
    w.rules
        .list_agents(viewer, Some(name.into()), None, 10, None)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|a| a.handle == name)
        .unwrap()
}
pub(crate) fn open_wait(
    w: &World,
    agent: &Agent,
    channels: Option<Vec<String>>,
) -> tokio::task::JoinHandle<Result<WaitResult>> {
    let rules = w.rules.clone();
    let agent = agent.clone();
    tokio::spawn(async move {
        rules
            .wait_for_messages(
                &agent,
                WaitQuery {
                    after_id: 0,
                    channel_ids: channels,
                    timeout: Some(Duration::from_secs(10)),
                    limit: 10,
                },
            )
            .await
    })
}

// ---- Stale agents ----

pub(crate) const HOUR: Duration = Duration::from_secs(60 * 60);

pub(crate) fn stale_names(stale: &[StaleAgent]) -> Vec<String> {
    let mut names: Vec<String> = stale.iter().map(|s| s.info.handle.clone()).collect();
    names.sort();
    names
}

// ---- The admin agent ----

/// An owner, an outsider, and an open, a closed and a hidden channel of the
/// owner's, each with one message from it.
pub(crate) struct AdminWorld {
    pub(crate) w: World,
    pub(crate) owner: Agent,
    pub(crate) outsider: Agent,
    pub(crate) open: String,
    pub(crate) closed: String,
    pub(crate) hidden: String,
    pub(crate) hidden_message: i64,
}

pub(crate) async fn admin_world() -> AdminWorld {
    let w = world().await;
    let owner = w.agent().await;
    let outsider = w.agent().await;
    let open = w.channel(&owner, "lobby", true, &[]).await;
    let closed = w.channel(&owner, "secret", false, &[]).await;
    let hidden = w.channel(&owner, "gone", false, &[]).await;
    for id in [&open, &closed] {
        w.rules
            .send(&owner, id.clone(), format!("hello {id}"))
            .await
            .unwrap();
    }
    w.rules
        .set_note(&owner, closed.clone(), "the closed note".into())
        .await
        .unwrap();
    let hidden_message = w
        .rules
        .send(&owner, hidden.clone(), "before hiding".into())
        .await
        .unwrap()
        .id;
    w.rules.hide_channel(&owner, hidden.clone()).await.unwrap();
    AdminWorld {
        w,
        owner,
        outsider,
        open,
        closed,
        hidden,
        hidden_message,
    }
}

/// The admin agent named `admin-sentinel`.
pub(crate) fn sentinel() -> Actor {
    Actor::Operator(admin_named(
        names::AdminName::new("sentinel", kkt_names::MAX_ADMIN_NAME_CHARS).unwrap(),
    ))
}

/// The last message in channel `id`, hidden or not, as the human reads it.
pub(crate) async fn last_said(w: &World, id: &str) -> Message {
    let view = w
        .rules
        .channel_view(&web_page(), id.to_string(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    view.messages.last().cloned().expect("a message")
}

// ---- Lanes ----

pub(crate) fn lane(name: &str, description: &str, text: &str) -> LaneSave {
    LaneSave {
        name: name.into(),
        description: description.into(),
        text: text.into(),
    }
}

// ---- Taking lanes ----

pub(crate) async fn said(w: &World, id: &str) -> Vec<(String, String)> {
    let view = w
        .rules
        .channel_view(&web_page(), id.to_string(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    view.messages
        .into_iter()
        .map(|m| (m.agent, m.body))
        .collect()
}

// ---- Importing lanes ----

pub(crate) fn import(lanes: Vec<LaneSave>, retire_missing: bool) -> LaneImport {
    LaneImport {
        lanes,
        source: LaneSource {
            folder: "/home/sam/lanes".into(),
            commit: Some("abc123".into()),
        },
        retire_missing,
        skipped_files: Vec::new(),
        max_lane_chars: Settings::DEFAULT.max_lane_chars,
    }
}

pub(crate) fn outcomes(done: &[ImportedLane]) -> Vec<(&str, LaneOutcome)> {
    done.iter()
        .map(|l| (l.name.as_str(), l.outcome.clone()))
        .collect()
}

// ---- Swarms ----

/// `w`'s rules in another swarm, `sw_other`: [`Rules::in_swarm`], sharing
/// the database, the bus, the waits and the rate limiters.
pub(crate) async fn other_swarm(w: &World) -> Rules {
    let other = SwarmId::new("sw_other");
    let at = clock::to_text(w.clock.now());
    let (swarm, run_id) = (other.clone(), w.run_id);
    w.rules
        .unbound_db()
        .write(move |s| s.insert_swarm(&swarm, &at, run_id))
        .await
        .expect("a second swarm");
    w.rules.in_swarm(other)
}

// ---- The operator's permissions ----

/// The refusal an operator gets for lacking `permission`, as rules give it.
pub(crate) fn lacks(result: Result<()>, permission: Permission) {
    let e = result.expect_err("refused for lacking a permission");
    assert_eq!(e.code, Code::Unauthenticated);
    assert_eq!(e.refusal, Some(Refusal::LacksPermission(permission)));
}

/// A new agent in the swarm `rules` act in.
pub(crate) async fn agent_in(rules: &Rules) -> Agent {
    let reg = rules.register().await.expect("register");
    rules.authenticate(Some(&reg.token)).await.expect("auth")
}

/// A new open channel `owner` makes in the swarm `rules` act in.
pub(crate) async fn open_channel_in(rules: &Rules, owner: &Agent, name: &str) -> String {
    rules
        .create_channel(
            owner,
            NewChannel {
                name: name.into(),
                open: true,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("create")
        .id
}

async fn name_clashes_get_a_number() {
    let w = world_with(
        Settings::DEFAULT,
        Arc::new(SameHandleEntropy(SeqEntropy::new(3))),
    )
    .await;
    let a = w.rules.register().await.unwrap();
    let b = w.rules.register().await.unwrap();
    let c = w.rules.register().await.unwrap();
    assert_eq!(b.handle, format!("{}-2", a.handle));
    assert_eq!(c.handle, format!("{}-3", a.handle));
}

async fn a_proposed_name_any_agent_has_or_had_is_taken_whatever_its_case() {
    let w = world().await;
    let live = register_as(&w, "reviewer").await.unwrap();
    let mixed = register_as(&w, "Mixed_Case-2").await.unwrap();
    let gone = w
        .rules
        .authenticate(Some(&register_as(&w, "gone").await.unwrap().token))
        .await
        .unwrap();
    w.rules.unregister(&gone).await.unwrap();
    let hidden = register_as(&w, "hidden").await.unwrap();
    w.rules
        .hide_agent(Actor::Operator(web_page()), hidden.handle.clone())
        .await
        .unwrap();
    let drawn = w.agent().await;
    let me = w.agent().await;
    let count = |w: &World| {
        let (rules, me) = (w.rules.clone(), me.clone());
        async move {
            rules
                .list_agents(&me, None, None, 100, None)
                .await
                .unwrap()
                .items
                .len()
        }
    };
    let before = count(&w).await;
    for taken in [
        live.handle.as_str(),
        "Reviewer",
        "gone",
        "hidden",
        "GONE",
        "hidden",
        drawn.handle.as_str(),
        &drawn.handle.to_uppercase(),
        mixed.handle.as_str(),
        "mixed_case-2",
        "MIXED_CASE-2",
    ] {
        let err = register_as(&w, taken).await.expect_err(taken);
        assert_eq!(err.code, Code::HandleTaken, "{taken}: {}", err.message);
    }
    assert_eq!(count(&w).await, before, "nothing registered");
    // The old avatar keeps its token; no suffix was added for anyone.
    let still = w.rules.authenticate(Some(&live.token)).await.unwrap();
    assert_eq!(still.handle, "reviewer");
}

async fn a_badly_shaped_or_reserved_name_is_refused_and_registers_nothing() {
    let settings = Settings {
        registrations_per_minute: 1,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let too_long = "a".repeat(kkt_names::MAX_HANDLE_CHARS + 1);
    for bad in [
        "",
        "api.reviewer",
        "api--reviewer",
        "-api",
        too_long.as_str(),
        "operator",
        "Operator",
        "OPERATOR",
        "admin-sentinel",
        "Admin-sentinel",
        "ADMIN-X_Y",
    ] {
        let err = register_as(&w, bad).await.expect_err(bad);
        assert_eq!(err.code, Code::InvalidArgument, "{bad:?}: {}", err.message);
    }
    // None of them used up the one registration this minute allows.
    let reg = register_as(&w, "first").await.unwrap();
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    let agents = w
        .rules
        .list_agents(&me, None, None, 100, None)
        .await
        .unwrap();
    assert_eq!(agents.items.len(), 1);
}

async fn last_seen_records_every_authenticated_call() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    let seen = |w: &World| {
        let rules = w.rules.clone();
        async move {
            let me = Agent {
                handle: String::new(),
            };
            rules
                .list_agents(&me, None, None, 10, None)
                .await
                .unwrap()
                .items[0]
                .last_seen
                .clone()
        }
    };
    let first = seen(&w).await;
    w.clock.advance(Span::seconds(30));
    w.rules.authenticate(Some(&reg.token)).await.unwrap();
    assert_ne!(seen(&w).await, first);
    w.clock.advance(Span::seconds(31));
    w.rules.authenticate(Some(&reg.token)).await.unwrap();
    assert_ne!(seen(&w).await, first);
}

async fn list_agents_pages_by_name() {
    let w = world().await;
    let me = w.agent().await;
    for _ in 0..4 {
        w.agent().await;
    }
    let first = w.rules.list_agents(&me, None, None, 3, None).await.unwrap();
    assert_eq!(first.items.len(), 3);
    let rest = w
        .rules
        .list_agents(&me, None, None, 3, first.next_cursor.clone())
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 2);
    assert!(rest.next_cursor.is_none());
    assert!(rest.items[0].handle > first.items[2].handle);
}

// ---- Channels with no owner ----

async fn an_unregistered_owner_leaves_no_owner_and_the_human_still_removes() {
    let w = world().await;
    let (a, b, c) = (w.agent().await, w.agent().await, w.agent().await);
    let by_a = w.channel(&a, "a's", false, &[&b, &c]).await;
    w.rules.unregister(&a).await.unwrap();
    let members = w
        .rules
        .list_members(&c, by_a.clone(), 20, None)
        .await
        .unwrap();
    assert_eq!(members.owner, None);
    // No agent has the owner's powers, but the human keeps every power.
    let d = w.agent().await;
    w.rules
        .add_to_channel(&c, by_a.clone(), vec![d.handle.clone()])
        .await
        .unwrap();
    assert_eq!(
        code(
            w.rules
                .remove_from_channel(&c, by_a.clone(), d.handle.clone())
                .await
        ),
        Code::NotOwner
    );
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            by_a.clone(),
            ChannelChange::RemoveMember(d.handle.clone()),
        )
        .await
        .unwrap();
    assert_eq!(code(w.read_latest(&d, &by_a).await), Code::NotFound);
    // Each channel says why it lost its owner, in the owner's own notice.
    let said = w.read_latest(&c, &by_a).await.unwrap().messages;
    assert!(said.iter().any(|m| m.agent == a.handle
        && m.body
            == format!(
                "{} unregistered and left the channel. Its messages are kept. \
                 The channel no longer has an owner.",
                a.handle
            )));
}

async fn the_web_channel_page_holds_web_page_messages_and_pages_back() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "busy", true, &[]).await;
    let mut ids = Vec::new();
    for i in 0..7 {
        let m = w.rules.send(&a, ch.clone(), format!("m{i}")).await.unwrap();
        ids.push(m.id);
    }
    let shown = |v: &ChannelView| v.messages.iter().map(|m| m.id).collect::<Vec<_>>();

    let latest = w
        .rules
        .channel_view(&web_page(), ch.clone(), None, 3)
        .await
        .unwrap();
    assert_eq!(shown(&latest), ids[4..7]);
    assert_eq!(latest.older_before_id, Some(ids[4]));

    let older = w
        .rules
        .channel_view(&web_page(), ch.clone(), Some(ids[4]), 3)
        .await
        .unwrap();
    assert_eq!(shown(&older), ids[1..4]);
    assert_eq!(older.older_before_id, Some(ids[1]));

    let oldest = w
        .rules
        .channel_view(&web_page(), ch, Some(ids[1]), 3)
        .await
        .unwrap();
    assert_eq!(shown(&oldest), ids[0..1]);
    assert_eq!(oldest.older_before_id, None);
}

async fn the_front_page_summarizes_each_channel_and_agent() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let busy = w.channel(&a, "busy", true, &[&b]).await;
    let quiet = w.channel(&b, "quiet", false, &[]).await;
    w.rules.send(&a, busy.clone(), "one".into()).await.unwrap();
    w.clock.advance(Span::minutes(10));
    w.rules.send(&b, busy.clone(), "two".into()).await.unwrap();
    w.rules
        .send(&a, busy.clone(), "three".into())
        .await
        .unwrap();

    let o = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    assert_eq!(o.now, clock::to_text(w.clock.now()));
    let row = |id: &str| o.channels.iter().find(|c| c.info.id == id).unwrap();
    let busy_row = row(&busy);
    assert_eq!(busy_row.message_count, 3);
    assert_eq!(busy_row.last_speaker.as_deref(), Some(a.handle.as_str()));
    assert_eq!(
        busy_row.last_message_at,
        Some(clock::to_text(w.clock.now()))
    );
    // Only the two messages from the last five minutes are recent.
    let recent = |c: &ChannelActivity| c.recent_activity.iter().map(|a| a.count).sum::<i64>();
    assert_eq!(recent(busy_row), 2);
    let quiet_row = row(&quiet);
    assert_eq!(quiet_row.message_count, 0);
    assert_eq!(quiet_row.last_speaker, None);
    assert!(quiet_row.recent_activity.is_empty());

    let channels_of = |name: &str| {
        let mut ids: Vec<String> = o
            .agents
            .iter()
            .find(|x| x.info.handle == name)
            .unwrap()
            .channels
            .iter()
            .map(|c| c.id.clone())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(channels_of(&a.handle), std::slice::from_ref(&busy));
    let mut both = vec![busy.clone(), quiet.clone()];
    both.sort();
    assert_eq!(channels_of(&b.handle), both);

    // Later, nothing is recent.
    w.clock.advance(Span::minutes(6));
    let later = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    assert!(later.channels.iter().all(|c| c.recent_activity.is_empty()));
}

/// A busy channel does not crowd a quiet one's recent count out.
async fn recent_counts_are_exact_for_every_channel() {
    let settings = Settings {
        sends_per_minute: 100_000,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let a = w.agent().await;
    let quiet = w.channel(&a, "quiet", true, &[]).await;
    let busy = w.channel(&a, "busy", true, &[]).await;
    w.rules
        .send(&a, quiet.clone(), "hello".into())
        .await
        .unwrap();
    for i in 0..1005 {
        w.rules
            .send(&a, busy.clone(), format!("m{i}"))
            .await
            .unwrap();
    }
    let o = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    let recent = |id: &str| {
        o.channels
            .iter()
            .find(|c| c.info.id == id)
            .unwrap()
            .recent_activity
            .iter()
            .map(|a| a.count)
            .sum::<i64>()
    };
    assert_eq!(recent(&quiet), 1);
    assert_eq!(recent(&busy), 1005);
}

/// A message stored later but stamped earlier (the clock moved back) does
/// not hide the recent ones.
async fn recent_counts_do_not_assume_times_follow_message_order() {
    let settings = Settings {
        sends_per_minute: 100_000,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let a = w.agent().await;
    let busy = w.channel(&a, "busy", true, &[]).await;
    for i in 0..1001 {
        w.rules
            .send(&a, busy.clone(), format!("m{i}"))
            .await
            .unwrap();
    }
    w.clock.advance(Span::hours(-1));
    w.rules
        .send(&a, busy.clone(), "from before".into())
        .await
        .unwrap();
    w.clock.advance(Span::hours(1));
    let o = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap();
    let row = o.channels.iter().find(|c| c.info.id == busy).unwrap();
    assert_eq!(
        row.recent_activity.iter().map(|a| a.count).sum::<i64>(),
        1001
    );
    assert_eq!(row.message_count, 1002);
}

// ---- list_channels filters ----

async fn list_channels_filters_and_pages() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let parser = w.channel(&a, "parser", true, &[]).await;
    w.rules
        .set_purpose(&a, parser.clone(), "work on the TOML parser".into())
        .await
        .unwrap();
    let shared = w.channel(&a, "pair", false, &[&b]).await;
    let mine = w.channel(&b, "notes", true, &[]).await;

    let found = w
        .rules
        .list_channels(
            &b,
            ChannelQuery {
                query: Some("toml".into()),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        found.items.iter().map(|c| &c.id).collect::<Vec<_>>(),
        vec![&parser]
    );

    let member_only = w
        .rules
        .list_channels(
            &b,
            ChannelQuery {
                member_only: true,
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ids: Vec<_> = member_only.items.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&shared) && ids.contains(&mine) && !ids.contains(&parser));

    let with = w
        .rules
        .list_channels(
            &b,
            ChannelQuery {
                with_agent: Some(a.handle.clone()),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        with.items.iter().map(|c| &c.id).collect::<Vec<_>>(),
        vec![&shared]
    );

    // Paging one at a time visits every visible channel once.
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = w
            .rules
            .list_channels(
                &b,
                ChannelQuery {
                    limit: 1,
                    cursor,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        seen.extend(page.items.into_iter().map(|c| c.id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 3);
}

async fn list_channels_counts_new_messages() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    let first = w.rules.send(&a, ch.clone(), "one".into()).await.unwrap();
    w.rules.send(&a, ch.clone(), "two".into()).await.unwrap();
    let page = w
        .rules
        .list_channels(
            &a,
            ChannelQuery {
                after_id: Some(first.id),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items[0].new_messages, Some(1));
}

async fn a_failed_send_does_not_use_up_the_allowance() {
    let settings = Settings {
        sends_per_minute: 1,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(1))).await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    // The database fails the next insert of a message, once.
    w.faults.fail_next_change_to("messages");
    assert_eq!(
        code(w.rules.send(&a, ch.clone(), "lost".into()).await),
        Code::Internal
    );
    w.rules.send(&a, ch.clone(), "kept".into()).await.unwrap();
    // The allowance is one a minute, so the refunded send was the only one.
    assert_eq!(
        code(w.rules.send(&a, ch, "too many".into()).await),
        Code::RateLimited
    );
}

async fn read_pages_both_ways() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    let other = w.channel(&a, "other", true, &[]).await;
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            w.rules
                .send(&a, ch.clone(), format!("m{i}"))
                .await
                .unwrap()
                .id,
        );
        // Interleave messages in another channel so IDs have gaps.
        w.rules
            .send(&a, other.clone(), "noise".into())
            .await
            .unwrap();
    }
    let q = |position| ReadQuery {
        channel_id: ch.clone(),
        position,
        from_agent: None,
        limit: 2,
    };
    let latest = w.rules.read(&a, q(Position::Latest)).await.unwrap();
    assert_eq!(
        latest.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        ids[3..5]
    );
    assert_eq!(latest.more, Some(Position::Before(ids[3])));

    let older = w.rules.read(&a, q(Position::Before(ids[3]))).await.unwrap();
    assert_eq!(
        older.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        ids[1..3]
    );

    let after = w.rules.read(&a, q(Position::After(ids[0]))).await.unwrap();
    assert_eq!(
        after.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        ids[1..3]
    );
    assert_eq!(after.more, Some(Position::After(ids[2])));

    let end = w.rules.read(&a, q(Position::After(ids[3]))).await.unwrap();
    assert_eq!(end.messages.len(), 1);
    assert_eq!(end.more, None);
}

async fn read_filters_by_agent() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    w.rules.send(&a, ch.clone(), "from a".into()).await.unwrap();
    w.rules.send(&b, ch.clone(), "from b".into()).await.unwrap();
    let page = w
        .rules
        .read(
            &a,
            ReadQuery {
                channel_id: ch,
                position: Position::Latest,
                from_agent: Some(b.handle.clone()),
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].agent, b.handle);
}

async fn search_covers_only_my_channels() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let mine = w.channel(&a, "mine", true, &[]).await;
    let theirs = w.channel(&b, "theirs", true, &[]).await;
    w.rules
        .send(&a, mine.clone(), "the parser is broken".into())
        .await
        .unwrap();
    w.rules
        .send(&b, theirs, "parser works for me".into())
        .await
        .unwrap();
    let hits = w.rules.search(&a, words("parser")).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].channel_id, mine);
    assert!(hits[0].excerpt.contains("**parser**"));
}

async fn search_phrase_needs_words_together_and_in_order() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    let mut ids = Vec::new();
    for body in [
        "we chose long-poll for transport",
        "a long poll is fine",
        "poll for a long time",
        "long waits, then a poll",
        "longpoll is one word",
        "PDF.js renders it",
    ] {
        ids.push(w.rules.send(&a, ch.clone(), body.into()).await.unwrap().id);
    }
    let phrase = |p: &str| SearchQuery {
        query: None,
        phrase: Some(p.into()),
        ..words("")
    };
    // Punctuation splits words the same way in the phrase and in messages.
    assert_eq!(found(&w, &a, phrase("long-poll")).await, [ids[1], ids[0]]);
    assert_eq!(found(&w, &a, phrase("LONG poll")).await, [ids[1], ids[0]]);
    assert_eq!(found(&w, &a, phrase("poll long")).await, Vec::<i64>::new());
    assert_eq!(found(&w, &a, phrase("pdf js")).await, [ids[5]]);
    // Search syntax in a phrase is only words.
    assert_eq!(
        found(&w, &a, phrase("long\" OR \"poll")).await,
        Vec::<i64>::new()
    );
    // The same words as `query` match in any order and apart.
    assert_eq!(
        found(&w, &a, words("long poll")).await,
        [ids[3], ids[2], ids[1], ids[0]]
    );
    // With both, a message must match each.
    let both = SearchQuery {
        phrase: Some("long poll".into()),
        ..words("transport")
    };
    assert_eq!(found(&w, &a, both).await, [ids[0]]);
}

async fn search_needs_query_or_phrase() {
    let w = world().await;
    let a = w.agent().await;
    let neither = SearchQuery {
        query: None,
        ..words("")
    };
    assert_eq!(
        code(w.rules.search(&a, neither).await),
        Code::InvalidArgument
    );
    let blank = SearchQuery {
        query: None,
        phrase: Some("   ".into()),
        ..words("")
    };
    assert_eq!(code(w.rules.search(&a, blank).await), Code::InvalidArgument);
    // An empty `query` is refused with the message it always had.
    let err = w.rules.search(&a, words(" ")).await.unwrap_err();
    assert_eq!(err.code, Code::InvalidArgument);
    assert_eq!(err.message, "search `query` must not be empty");
}

async fn search_id_ranges_exclude_their_edges() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    let other = w.channel(&a, "other", true, &[]).await;
    let mut ids = Vec::new();
    for (who, channel) in [(&a, &ch), (&b, &ch), (&a, &other), (&b, &ch), (&a, &ch)] {
        let sent = w.rules.send(who, channel.clone(), "note".into()).await;
        ids.push(sent.unwrap().id);
    }
    let after = SearchQuery {
        after_id: Some(ids[1]),
        ..words("note")
    };
    assert_eq!(found(&w, &a, after).await, [ids[4], ids[3], ids[2]]);
    let before = SearchQuery {
        before_id: Some(ids[3]),
        ..words("note")
    };
    assert_eq!(found(&w, &a, before).await, [ids[2], ids[1], ids[0]]);
    let between = SearchQuery {
        after_id: Some(ids[0]),
        before_id: Some(ids[4]),
        ..words("note")
    };
    assert_eq!(
        found(&w, &a, between.clone()).await,
        [ids[3], ids[2], ids[1]]
    );
    let in_channel = SearchQuery {
        channel_id: Some(ch.clone()),
        ..between.clone()
    };
    assert_eq!(found(&w, &a, in_channel).await, [ids[3], ids[1]]);
    let from_b = SearchQuery {
        from_agent: Some(b.handle.clone()),
        ..between
    };
    assert_eq!(found(&w, &a, from_b).await, [ids[3], ids[1]]);
}

async fn search_time_ranges_include_since_and_exclude_until() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let ch = w.channel(&a, "general", true, &[&b]).await;
    let other = w.channel(&a, "other", true, &[]).await;
    let mut ids = Vec::new();
    let mut times = Vec::new();
    for (who, channel) in [(&a, &ch), (&b, &ch), (&a, &other), (&b, &ch)] {
        w.clock.advance(Span::minutes(10));
        times.push(w.clock.now());
        let sent = w.rules.send(who, channel.clone(), "note".into()).await;
        ids.push(sent.unwrap().id);
    }
    let text = |t: Time| Some(clock::to_text(t));
    // The times written out below in other forms.
    assert_eq!(clock::to_text(times[1]), "2026-10-23T14:20:00Z");
    let since = SearchQuery {
        since: text(times[1]),
        ..words("note")
    };
    assert_eq!(found(&w, &a, since).await, [ids[3], ids[2], ids[1]]);
    let until = SearchQuery {
        until: text(times[2]),
        ..words("note")
    };
    assert_eq!(found(&w, &a, until).await, [ids[1], ids[0]]);
    // Another offset names the same moment.
    let offset = SearchQuery {
        since: Some("2026-10-23T16:20:00+02:00".into()),
        until: text(times[3]),
        ..words("note")
    };
    assert_eq!(found(&w, &a, offset.clone()).await, [ids[2], ids[1]]);
    let in_channel = SearchQuery {
        channel_id: Some(ch.clone()),
        ..offset.clone()
    };
    assert_eq!(found(&w, &a, in_channel).await, [ids[1]]);
    let from_a = SearchQuery {
        from_agent: Some(a.handle.clone()),
        ..offset
    };
    assert_eq!(found(&w, &a, from_a).await, [ids[2]]);
    // A fraction of a second after a message's time leaves it out of `since`
    // and in `until`.
    let half = "2026-10-23T14:20:00.500+00:00";
    let late_since = SearchQuery {
        since: Some(half.into()),
        ..words("note")
    };
    assert_eq!(found(&w, &a, late_since).await, [ids[3], ids[2]]);
    let late_until = SearchQuery {
        until: Some(half.into()),
        ..words("note")
    };
    assert_eq!(found(&w, &a, late_until).await, [ids[1], ids[0]]);
}

async fn search_refuses_times_it_cannot_read() {
    let w = world().await;
    let a = w.agent().await;
    for bad in ["yesterday", "2026-10-30", "2026-10-30T14:00:00", ""] {
        let since = SearchQuery {
            since: Some(bad.into()),
            ..words("note")
        };
        let err = w.rules.search(&a, since).await.unwrap_err();
        assert_eq!(err.code, Code::InvalidArgument);
        assert!(err.message.contains("`since`"), "{}", err.message);
        let until = SearchQuery {
            until: Some(bad.into()),
            ..words("note")
        };
        let err = w.rules.search(&a, until).await.unwrap_err();
        assert_eq!(err.code, Code::InvalidArgument);
        assert!(err.message.contains("`until`"), "{}", err.message);
    }
}

async fn search_ranges_never_reach_closed_channels_i_am_not_in() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let secret = w.channel(&b, "secret", false, &[]).await;
    w.rules
        .send(&b, secret, "long poll plans".into())
        .await
        .unwrap();
    let wide = SearchQuery {
        query: None,
        phrase: Some("long poll".into()),
        after_id: Some(0),
        since: Some("2000-01-01T00:00:00Z".into()),
        ..words("")
    };
    assert!(found(&w, &a, wide.clone()).await.is_empty());
    assert_eq!(found(&w, &b, wide).await.len(), 1);
}

async fn activity_reports_channels_with_newer_messages() {
    let w = world().await;
    let (a, b) = (w.agent().await, w.agent().await);
    let one = w.channel(&a, "one", true, &[&b]).await;
    let two = w.channel(&a, "two", true, &[&b]).await;
    let start = w.rules.send(&a, one.clone(), "x".into()).await.unwrap().id;
    w.rules.send(&a, one.clone(), "y".into()).await.unwrap();
    w.rules.send(&a, two.clone(), "z".into()).await.unwrap();
    w.rules.send(&a, two.clone(), "w".into()).await.unwrap();
    let rows = w.rules.activity(&b, start).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].channel_id, two);
    assert_eq!(rows[0].new_messages, 2);
    assert_eq!(rows[1].new_messages, 1);
}

async fn wait_returns_at_once_when_messages_exist() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    for i in 0..3 {
        w.rules.send(&a, ch.clone(), format!("m{i}")).await.unwrap();
    }
    let got = w
        .rules
        .wait_for_messages(
            &a,
            WaitQuery {
                after_id: 0,
                channel_ids: None,
                timeout: Some(Duration::from_secs(60)),
                limit: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(got.messages.len(), 2);
    assert_eq!(got.more_waiting, 1);
}

async fn a_wait_longer_than_max_wait_is_cut_to_max_wait() {
    wait_lasts(Some(Duration::from_secs(120)), 9).await;
}

async fn a_wait_with_no_timeout_lasts_default_wait() {
    wait_lasts(None, 5).await;
}

async fn the_operator_lists_agents_with_their_revocations() {
    let w = world().await;
    let a = w.agent().await;
    w.clock.advance(Span::minutes(5));
    let b = w.agent().await;
    w.operator.revoke(a.handle.clone()).await.expect("revoke");
    let agents = w.operator.agents().await.expect("list");
    let handles: Vec<_> = agents.iter().map(|s| s.handle.as_str()).collect();
    assert_eq!(handles, [b.handle.as_str(), a.handle.as_str()]);
    assert_eq!(agents[0].revoked_at, None);
    assert_eq!(
        agents[1].revoked_at.as_deref(),
        Some("2026-10-23T14:05:00Z")
    );
}

async fn reclaim_preserves_the_avatar_history_membership_and_provenance() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    let me = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    let channel = w.channel(&me, "private", false, &[]).await;
    let sent = w
        .rules
        .send(&me, channel.clone(), "before restart".into())
        .await
        .unwrap();
    let name = reg.handle.clone();
    let before = w
        .rules
        .unbound_db()
        .read(move |r| r.agent(&SwarmId::main(), &name))
        .await
        .unwrap()
        .unwrap();
    w.clock.advance(Span::seconds(75));
    let recovered = w
        .rules
        .reclaim(reg.handle.clone(), reg.recovery_code.clone())
        .await
        .unwrap();
    assert_eq!(recovered.handle, reg.handle);
    assert_ne!(recovered.token, reg.token);
    assert_eq!(
        code(w.rules.authenticate(Some(&reg.token)).await),
        Code::Unauthenticated
    );
    let me = w.rules.authenticate(Some(&recovered.token)).await.unwrap();
    assert_eq!(w.visible_ids(&me).await, [channel]);
    assert_eq!(
        w.rules.get_message(&me, sent.id).await.unwrap().body,
        "before restart"
    );
    let name = reg.handle.clone();
    let after = w
        .rules
        .unbound_db()
        .read(move |r| r.agent(&SwarmId::main(), &name))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.run_id, before.run_id);
    assert_eq!(after.last_seen, Some(w.rules.now_text()));
    // The code still works after a successful recovery (including a lost response).
    let again = w
        .rules
        .reclaim(reg.handle, reg.recovery_code)
        .await
        .unwrap();
    assert_ne!(again.token, recovered.token);
    assert_eq!(
        code(w.rules.authenticate(Some(&recovered.token)).await),
        Code::Unauthenticated
    );
    w.rules.authenticate(Some(&again.token)).await.unwrap();
}
async fn listening_covers_selected_channels_and_hides_closed_channels() {
    let w = world().await;
    let a = w.agent().await;
    let viewer = w.agent().await;
    let public = w.channel(&a, "public", true, &[]).await;
    let secret = w.channel(&a, "secret", false, &[]).await;
    let other = w.channel(&a, "other", true, &[]).await;
    let waiter = open_wait(&w, &a, Some(vec![public.clone(), secret.clone()]));
    until_sleeping(&w.clock).await;
    assert_eq!(
        observed(&w, &viewer, &a.handle).await.waiting_channels,
        vec![public.clone()]
    );
    assert_eq!(observed(&w, &a, &a.handle).await.waiting_channels.len(), 2);
    let members = w
        .rules
        .list_members(&viewer, other, 10, None)
        .await
        .unwrap();
    assert!(!members.page.items[0].listening);
    let members = w
        .rules
        .list_members(&viewer, public, 10, None)
        .await
        .unwrap();
    assert!(members.page.items[0].listening);
    let human = w
        .rules
        .agent_view(&web_page(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(human.agent.waiting_channels.len(), 2);
    assert!(
        w.rules
            .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .unwrap()
            .agents
            .iter()
            .any(|x| x.info.listening)
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(!observed(&w, &a, &a.handle).await.listening);
}
async fn normal_wait_completion_has_grace_but_aborted_wait_does_not() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    let first = open_wait(&w, &a, None);
    until_sleeping(&w.clock).await;
    w.clock.advance(Span::seconds(10));
    first.await.unwrap().unwrap();
    assert!(observed(&w, &a, &a.handle).await.listening);
    w.clock.advance(Span::seconds(2));
    assert!(observed(&w, &a, &a.handle).await.listening);
    let second = open_wait(&w, &a, Some(vec![ch]));
    until_sleeping(&w.clock).await;
    w.clock.advance(Span::seconds(1));
    assert!(observed(&w, &a, &a.handle).await.listening);
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    assert!(!observed(&w, &a, &a.handle).await.listening);
}
async fn overlapping_waits_and_current_memberships_are_independent() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    let first = open_wait(&w, &a, Some(vec![ch.clone()]));
    until_sleeping(&w.clock).await;
    let second = open_wait(&w, &a, None);
    while w.clock.sleepers() < 2 {
        tokio::task::yield_now().await;
    }
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(observed(&w, &a, &a.handle).await.listening);
    let joined = w.channel(&a, "joined", true, &[]).await;
    assert_eq!(observed(&w, &a, &a.handle).await.waiting_channels.len(), 2);
    w.rules.leave_channel(&a, ch).await.unwrap();
    assert_eq!(
        observed(&w, &a, &a.handle).await.waiting_channels,
        vec![joined]
    );
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    assert!(!observed(&w, &a, &a.handle).await.listening);
}

async fn a_new_wait_supersedes_grace_and_disconnect_clears_it() {
    let w = world().await;
    let a = w.agent().await;
    let ch = w.channel(&a, "general", true, &[]).await;
    w.rules
        .wait_for_messages(
            &a,
            WaitQuery {
                after_id: 0,
                channel_ids: None,
                timeout: Some(Duration::ZERO),
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(observed(&w, &a, &a.handle).await.listening);
    let next = open_wait(&w, &a, Some(vec![ch]));
    until_sleeping(&w.clock).await;
    next.abort();
    assert!(next.await.unwrap_err().is_cancelled());
    assert!(!observed(&w, &a, &a.handle).await.listening);
}

async fn hiding_an_agent_revokes_it_announces_each_membership_and_keeps_its_name_reserved() {
    let w = world_with(
        Settings::DEFAULT,
        Arc::new(SameHandleEntropy(SeqEntropy::new(5))),
    )
    .await;
    let registration = w.rules.register().await.unwrap();
    let a = w
        .rules
        .authenticate(Some(&registration.token))
        .await
        .unwrap();
    let b = w.agent().await;
    let first = w.channel(&b, "first", false, &[&a]).await;
    let second = w.channel(&b, "second", true, &[&a]).await;
    let sent = w
        .rules
        .send(&a, first.clone(), "kept agent message".into())
        .await
        .unwrap();
    assert_eq!(
        code(w.rules.hide_agent(Actor::from(&a), b.handle.clone()).await),
        Code::OperatorOnly
    );
    w.rules
        .hide_agent(Actor::Operator(web_page()), a.handle.clone())
        .await
        .unwrap();
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
    assert_eq!(
        code(w.rules.agent_view(&web_page(), a.handle.clone()).await),
        Code::NotFound
    );
    assert_eq!(
        code(
            w.rules
                .new_recovery_code(&web_page(), a.handle.clone())
                .await
        ),
        Code::NotFound
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
    for id in [first, second] {
        let view = w
            .rules
            .channel_view(&web_page(), id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap();
        assert!(!view.members.iter().any(|m| m.handle == a.handle));
        let last = view.messages.last().unwrap();
        assert_eq!(last.agent, names::OPERATOR);
        assert!(last.body.contains(&a.handle));
        assert_eq!(
            code(w.rules.add_to_channel(&b, id, vec![a.handle.clone()]).await),
            Code::NotFound
        );
    }
    assert_eq!(
        w.rules.get_message(&b, sent.id).await.unwrap().body,
        "kept agent message"
    );
    let name = a.handle.clone();
    assert!(
        w.rules
            .unbound_db()
            .read(move |s| s.agent_name_taken(&name))
            .await
            .unwrap()
    );
    let new = w.agent().await;
    assert_ne!(new.handle, a.handle);
    assert!(new.handle.starts_with(&a.handle));
}

async fn administration_rolls_back_when_the_operator_message_cannot_be_saved() {
    let w = world().await;
    let a = w.agent().await;
    let b = w.agent().await;
    let c = w.agent().await;
    let id = w.channel(&a, "atomic-admin", true, &[]).await;
    let other = w.channel(&a, "other", true, &[&c]).await;
    let broken = Rules::new(Deps {
        db: w.rules.unbound_db().clone(),
        swarm: SwarmId::main(),
        events: Events::new(kkt_events::Settings::DEFAULT),
        clock: w.clock.clone(),
        entropy: Arc::new(SeqEntropy::new(15)),
        run_id: i64::MAX,
        settings: Settings::DEFAULT,
        wording: Arc::new(DefaultWording),
    });
    for change in [
        ChannelChange::SetOpen(false),
        ChannelChange::AddMember(b.handle.clone()),
        ChannelChange::SetOwner(None),
        ChannelChange::Hide,
    ] {
        assert_eq!(
            code(
                broken
                    .change_channel(Actor::Operator(web_page()), id.clone(), change)
                    .await
            ),
            Code::Internal
        );
        let view = w
            .rules
            .channel_view(&web_page(), id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap();
        assert!(view.channel.open);
        assert_eq!(view.channel.owner.as_deref(), Some(a.handle.as_str()));
        assert_eq!(view.members.len(), 1);
        assert!(view.messages.is_empty());
    }
    // The owner's hide, too, commits with its message or not at all.
    assert_eq!(
        code(broken.hide_channel(&a, id.clone()).await),
        Code::Internal
    );
    let view = w
        .rules
        .channel_view(&web_page(), id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.members.len(), 1);
    assert!(view.messages.is_empty());
    assert!(w.visible_ids(&a).await.contains(&id));
    assert_eq!(
        code(
            broken
                .hide_agent(Actor::Operator(web_page()), c.handle.clone())
                .await
        ),
        Code::Internal
    );
    assert!(
        w.rules
            .agent_view(&web_page(), c.handle.clone())
            .await
            .is_ok()
    );
    assert_eq!(
        w.rules
            .channel_view(&web_page(), other, None, PAGE_MESSAGES)
            .await
            .unwrap()
            .members
            .len(),
        2
    );
}

async fn a_hidden_channel_id_is_skipped_when_entropy_draws_it_again() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct RepeatFirstId(AtomicUsize);
    impl Entropy for RepeatFirstId {
        fn below(&self, n: usize) -> usize {
            usize::from(self.0.fetch_add(1, Ordering::SeqCst) >= kkt_names::CHANNEL_RANDOM_LEN) % n
        }
    }
    let entropy = Arc::new(RepeatFirstId(AtomicUsize::new(0)));
    let w = world_with(Settings::DEFAULT, entropy.clone()).await;
    let a = w.agent().await;
    entropy.0.store(0, Ordering::SeqCst);
    let first = w.channel(&a, "first", true, &[]).await;
    w.rules
        .change_channel(
            Actor::Operator(web_page()),
            first.clone(),
            ChannelChange::Hide,
        )
        .await
        .unwrap();
    entropy.0.store(0, Ordering::SeqCst);
    let second = w.channel(&a, "second", true, &[]).await;
    assert_ne!(first, second);
    assert_eq!(first, "ch_aaaaaaaa");
    assert_eq!(second, "ch_bbbbbbbb");
}

async fn an_entry_over_the_entry_cap_is_refused_and_the_old_one_kept() {
    let settings = Settings {
        max_memory_entry_chars: 5,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let a = w.agent().await;
    w.rules
        .remember(&a, "note".into(), "héllo".into())
        .await
        .unwrap();
    let refused = w
        .rules
        .remember(&a, "note".into(), "héllo!".into())
        .await
        .unwrap_err();
    assert_eq!(refused.code, Code::InvalidArgument);
    assert!(
        refused.message.contains("max_memory_entry_chars"),
        "{}",
        refused.message
    );
    assert_eq!(
        w.rules.recall(&a, "note".into()).await.unwrap().text,
        "héllo"
    );
}

async fn a_memory_over_the_avatar_cap_is_refused_counting_a_replaced_entry_once() {
    let settings = Settings {
        max_memory_entry_chars: 6,
        max_memory_chars: 10,
        ..Settings::DEFAULT
    };
    let w = world_with(settings, Arc::new(SeqEntropy::new(42))).await;
    let (a, b) = (w.agent().await, w.agent().await);
    w.rules
        .remember(&a, "one".into(), "123456".into())
        .await
        .unwrap();
    let refused = w
        .rules
        .remember(&a, "two".into(), "12345".into())
        .await
        .unwrap_err();
    assert_eq!(refused.code, Code::LimitReached);
    assert!(
        refused.message.contains("max_memory_chars"),
        "{}",
        refused.message
    );
    assert_eq!(
        code(w.rules.recall(&a, "two".into()).await),
        Code::NotFound,
        "nothing was saved"
    );
    // Exactly at the cap is allowed.
    w.rules
        .remember(&a, "two".into(), "1234".into())
        .await
        .unwrap();
    // Replacing an entry counts its new text, not old and new together.
    let saved = w
        .rules
        .remember(&a, "one".into(), "12".into())
        .await
        .unwrap();
    assert_eq!((saved.entries, saved.chars), (2, 6));
    w.rules
        .remember(&a, "two".into(), "12345678".into())
        .await
        .unwrap_err();
    // Another avatar's memory is counted on its own.
    w.rules
        .remember(&b, "one".into(), "123456".into())
        .await
        .unwrap();
}

async fn two_sessions_saving_different_entries_keep_both() {
    let w = world().await;
    let reg = w.rules.register().await.unwrap();
    let first = w.rules.authenticate(Some(&reg.token)).await.unwrap();
    // A second session of the same avatar, each saving its own entry at once.
    let second = first.clone();
    let (one, two) = tokio::join!(
        w.rules
            .remember(&first, "parser".into(), "tokenizer done".into()),
        w.rules
            .remember(&second, "reviews".into(), "bold-magpie reviews".into()),
    );
    one.unwrap();
    two.unwrap();
    let list = w.rules.list_memories(&first, 10, None).await.unwrap();
    let names: Vec<_> = list.page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["parser", "reviews"]);
    assert_eq!(list.chars, 14 + 19);
}

async fn memory_lists_page_by_name() {
    let w = world().await;
    let a = w.agent().await;
    for name in ["c", "a", "b"] {
        w.rules
            .remember(&a, name.into(), format!("text {name}"))
            .await
            .unwrap();
    }
    let first = w.rules.list_memories(&a, 2, None).await.unwrap();
    let names: Vec<_> = first.page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(first.page.next_cursor.as_deref(), Some("b"));
    assert_eq!((first.entries, first.chars), (3, 18));
    assert_eq!(first.page.items[0].chars, 6);
    let rest = w
        .rules
        .list_memories(&a, 2, first.page.next_cursor)
        .await
        .unwrap();
    let names: Vec<_> = rest.page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["c"]);
    assert_eq!(rest.page.next_cursor, None);
}

async fn server_announcement_saves_are_immutable_versions_with_clock_and_run() {
    let w = world().await;
    assert_eq!(w.rules.current_server_announcement().await.unwrap(), None);
    let first = w
        .rules
        .save_server_announcement(Actor::Operator(web_page()), "  First\n😀  ".into())
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    assert_eq!(first.text, "  First\n😀  ");
    assert_eq!(first.saved_at, w.rules.now_text());
    assert_eq!(first.run_id, w.run_id);
    w.clock.advance(Span::seconds(10));
    let clear = w
        .rules
        .save_server_announcement(Actor::Operator(web_page()), String::new())
        .await
        .unwrap();
    assert_eq!(clear.version, 2);
    assert_eq!(clear.saved_at, w.rules.now_text());
    assert_eq!(
        w.rules.current_server_announcement().await.unwrap(),
        Some(clear.clone())
    );
    assert_eq!(
        w.rules
            .server_announcement_history(&web_page(),)
            .await
            .unwrap(),
        vec![clear.clone(), first]
    );
    let registration = w.rules.register().await.unwrap();
    assert_eq!(registration.server_announcement, Some(clear));
}
async fn concurrent_announcement_saves_get_distinct_increasing_versions() {
    let w = world().await;
    let (a, b) = tokio::join!(
        w.rules
            .save_server_announcement(Actor::Operator(web_page()), "a".into()),
        w.rules
            .save_server_announcement(Actor::Operator(web_page()), "b".into())
    );
    assert_ne!(a.unwrap().version, b.unwrap().version);
    let history = w
        .rules
        .server_announcement_history(&web_page())
        .await
        .unwrap();
    assert_eq!(
        history.iter().map(|v| v.version).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

async fn operator_channel_creation_is_atomic_and_never_adds_the_operator() {
    let w = world().await;
    let a = w.agent().await;
    let b = w.agent().await;
    let new = NewChannel {
        name: "  help  ".into(),
        purpose: Some("  help desk  ".into()),
        open: false,
        members: vec![a.handle.clone(), a.handle.clone()],
    };
    assert_eq!(
        code(
            w.rules
                .create_channel_as_operator(Actor::Avatar(a.clone()), new.clone())
                .await
        ),
        Code::OperatorOnly
    );
    let created = w
        .rules
        .create_channel_as_operator(Actor::Operator(web_page()), new.clone())
        .await
        .unwrap();
    assert_eq!(created.name, "help");
    assert_eq!(created.created_by, names::OPERATOR);
    let view = w
        .rules
        .channel_view(&web_page(), created.id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.members.len(), 1);
    assert_eq!(view.members[0].handle, a.handle);
    assert_eq!(view.messages[0].agent, names::OPERATOR);
    assert_eq!(view.messages[0].run_id, Some(w.run_id));
    assert!(w.visible_ids(&a).await.contains(&created.id));
    assert!(!w.visible_ids(&b).await.contains(&created.id));
    let before = w
        .rules
        .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
        .await
        .unwrap()
        .channels
        .len();
    let broken = Rules::new(Deps {
        db: w.rules.unbound_db().clone(),
        swarm: SwarmId::main(),
        events: Events::new(kkt_events::Settings::DEFAULT),
        clock: w.clock.clone(),
        entropy: Arc::new(SeqEntropy::new(15)),
        run_id: i64::MAX,
        settings: Settings::DEFAULT,
        wording: Arc::new(DefaultWording),
    });
    assert_eq!(
        code(
            broken
                .create_channel_as_operator(Actor::Operator(web_page()), new.clone())
                .await
        ),
        Code::Internal
    );
    let mut unknown = new;
    unknown.members.push("missing-agent".into());
    assert_eq!(
        code(
            w.rules
                .create_channel_as_operator(Actor::Operator(web_page()), unknown)
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        w.rules
            .overview(&web_page(), ACTIVITY_WINDOW, ACTIVITY_BUCKET)
            .await
            .unwrap()
            .channels
            .len(),
        before
    );
}

async fn stale_agents_are_quiet_longer_than_the_period_and_not_waiting() {
    let w = world().await;
    let owner = w.agent().await;
    let noted = w.agent().await;
    let caller = w.rules.register().await.unwrap();
    let caller_agent = w.rules.authenticate(Some(&caller.token)).await.unwrap();
    let waiter = w.agent().await;
    let room = w
        .channel(&owner, "room", true, &[&noted, &caller_agent, &waiter])
        .await;
    w.rules
        .remember(&noted, "handoff".into(), "Next: the parser.".into())
        .await
        .unwrap();
    w.clock.advance(Span::hours(3));
    w.rules.authenticate(Some(&caller.token)).await.unwrap();
    let wait = open_wait(&w, &waiter, None);
    until_sleeping(&w.clock).await;

    let stale = w.rules.stale_agents(&web_page(), 2 * HOUR).await.unwrap();
    let mut expected = vec![owner.handle.clone(), noted.handle.clone()];
    expected.sort();
    assert_eq!(stale_names(&stale), expected);
    let find = |name: &str| stale.iter().find(|s| s.info.handle == name).unwrap();
    let o = find(&owner.handle);
    assert_eq!(o.owns.iter().map(|c| &c.id).collect::<Vec<_>>(), [&room]);
    assert_eq!(o.memory_entries, 0);
    let n = find(&noted.handle);
    assert!(n.owns.is_empty());
    assert_eq!(n.memory_entries, 1);
    assert_eq!(
        n.channels.iter().map(|c| &c.id).collect::<Vec<_>>(),
        [&room]
    );

    // Quiet for exactly the period is not longer than it.
    assert!(
        w.rules
            .stale_agents(&web_page(), 3 * HOUR)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        code(w.rules.stale_agents(&web_page(), Duration::ZERO).await),
        Code::InvalidArgument
    );

    // Once the wait ends and its grace passes, the waiter is stale too.
    wait.abort();
    assert!(wait.await.unwrap_err().is_cancelled());
    let stale = w.rules.stale_agents(&web_page(), 2 * HOUR).await.unwrap();
    assert!(stale.iter().any(|s| s.info.handle == waiter.handle));
    assert!(stale.iter().all(|s| s.info.handle != caller_agent.handle));
}

async fn stale_agents_are_listed_longest_quiet_first() {
    let w = world().await;
    let older = w.agent().await;
    w.clock.advance(Span::hours(1));
    let newer = w.agent().await;
    w.clock.advance(Span::hours(3));
    let stale = w.rules.stale_agents(&web_page(), 2 * HOUR).await.unwrap();
    let names: Vec<&str> = stale.iter().map(|s| s.info.handle.as_str()).collect();
    assert_eq!(names, [older.handle.as_str(), newer.handle.as_str()]);
}

async fn saving_what_the_lane_already_says_adds_no_version() {
    let w = world().await;
    let saved = w
        .rules
        .save_lane(
            Actor::Operator(web_page()),
            lane("reviewer", "Reviews.", "Read."),
        )
        .await
        .unwrap();
    w.clock.advance(Span::seconds(10));
    let again = w
        .rules
        .save_lane(sentinel(), lane("reviewer", " Reviews. ", "Read."))
        .await
        .unwrap();
    assert_eq!(again, saved);
    let history = w
        .rules
        .lane_history(&web_page(), "reviewer".into())
        .await
        .unwrap();
    assert_eq!(history.versions.len(), 1);
}

async fn a_retired_lane_is_kept_with_its_history_and_cannot_be_changed() {
    let w = world().await;
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "t"))
        .await
        .unwrap();
    w.clock.advance(Span::seconds(10));
    let retired = w
        .rules
        .retire_lane(sentinel(), "reviewer".into())
        .await
        .unwrap();
    let mark = retired.retired.clone().expect("retired");
    assert_eq!(mark.at, w.rules.now_text());
    assert_eq!(mark.run_id, w.run_id);
    assert_eq!(mark.by_admin.as_deref(), Some("admin-sentinel"));

    // Retiring again changes nothing, not even who retired it.
    w.clock.advance(Span::seconds(10));
    assert_eq!(
        w.rules
            .retire_lane(Actor::Operator(web_page()), "reviewer".into())
            .await
            .unwrap(),
        retired
    );
    assert_eq!(
        code(
            w.rules
                .save_lane(
                    Actor::Operator(web_page()),
                    lane("reviewer", "d", "new text")
                )
                .await
        ),
        Code::InvalidArgument
    );
    let history = w
        .rules
        .lane_history(&web_page(), "reviewer".into())
        .await
        .unwrap();
    assert_eq!(history.lane, retired);
    assert_eq!(history.versions.len(), 1);
    assert_eq!(w.rules.lanes(&web_page(),).await.unwrap(), vec![retired]);
}

async fn concurrent_lane_saves_get_distinct_increasing_versions() {
    let w = world().await;
    w.rules
        .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "first"))
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        w.rules
            .save_lane(Actor::Operator(web_page()), lane("reviewer", "d", "a")),
        w.rules.save_lane(sentinel(), lane("reviewer", "d", "b"))
    );
    assert_ne!(a.unwrap().current.version, b.unwrap().current.version);
    let history = w
        .rules
        .lane_history(&web_page(), "reviewer".into())
        .await
        .unwrap();
    assert_eq!(
        history
            .versions
            .iter()
            .map(|v| v.version)
            .collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
}

async fn list_lanes_for_agents_leaves_out_retired_lanes_and_pages_by_name() {
    let w = world().await;
    for name in ["a-lane", "b-lane", "c-lane", "d-lane"] {
        w.rules
            .save_lane(Actor::Operator(web_page()), lane(name, "d", "t"))
            .await
            .unwrap();
    }
    w.rules
        .retire_lane(Actor::Operator(web_page()), "b-lane".into())
        .await
        .unwrap();
    let first = w.rules.takeable_lanes_page(2, None).await.unwrap();
    let names = |p: &Page<Lane>| p.items.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&first), ["a-lane", "c-lane"]);
    let rest = w
        .rules
        .takeable_lanes_page(2, first.next_cursor.clone())
        .await
        .unwrap();
    assert_eq!(names(&rest), ["d-lane"]);
    assert_eq!(rest.next_cursor, None);
}

async fn one_bad_lane_or_a_lane_given_twice_stops_the_import_with_nothing_changed() {
    let w = world().await;
    let limits_text = "x".repeat(Settings::DEFAULT.max_lane_chars + 1);
    let cases = [
        (lane("Reviewer", "d", "t"), "lane Reviewer:"),
        (lane("reviewer", "", "t"), "needs a description"),
        (lane("reviewer", "d", "  "), "needs instructions"),
        (
            lane("reviewer", "d", &limits_text),
            "at most 8000 characters",
        ),
        (lane("writer", "d", "t"), "the lane writer is given twice"),
    ];
    for (bad, says) in cases {
        let lanes = vec![lane("writer", "d", "t"), bad];
        // The check `kkt setup` makes first refuses it the same way.
        let checked = check_lane_import(&lanes, Settings::DEFAULT.max_lane_chars).expect_err(says);
        let error = w
            .operator
            .import_lanes(import(lanes, true))
            .await
            .expect_err(says);
        assert_eq!(checked, error, "{says}");
        assert_eq!(error.code, Code::InvalidArgument, "{says}");
        assert!(error.message.contains(says), "{}", error.message);
        assert!(
            w.rules.lanes(&web_page(),).await.unwrap().is_empty(),
            "{says}"
        );
    }
    assert_eq!(
        check_lane_import(
            &[lane("writer", "d", "t"), lane("reviewer", "d", "t")],
            Settings::DEFAULT.max_lane_chars
        ),
        Ok(())
    );
}

async fn last_sent_counts_only_messages_the_viewer_can_see() {
    let w = world().await;
    let a = w.agent().await;
    let insider = w.agent().await;
    let viewer = w.agent().await;
    let public = w.channel(&a, "public", true, &[&viewer]).await;
    let secret = w.channel(&a, "secret", false, &[&insider]).await;
    w.clock.advance(Span::minutes(1));
    let in_public = clock::to_text(w.clock.now());
    w.rules.send(&a, public.clone(), "hi".into()).await.unwrap();
    w.clock.advance(Span::minutes(1));
    let in_secret = clock::to_text(w.clock.now());
    w.rules
        .send(&a, secret.clone(), "psst".into())
        .await
        .unwrap();

    let sent = |items: &[AgentInfo], who: &Agent| {
        items
            .iter()
            .find(|i| i.handle == who.handle)
            .unwrap()
            .last_sent
            .clone()
    };
    // An agent outside the closed channel sees only the open one's message.
    assert_eq!(
        observed(&w, &viewer, &a.handle).await.last_sent,
        Some(in_public.clone())
    );
    // A member of the closed channel sees the later message there.
    assert_eq!(
        observed(&w, &insider, &a.handle).await.last_sent,
        Some(in_secret.clone())
    );
    // An agent that never sent has none.
    assert_eq!(observed(&w, &insider, &viewer.handle).await.last_sent, None);
    // A member list counts only that channel, even for a viewer who could
    // see the later message elsewhere.
    let members = w
        .rules
        .list_members(&insider, public.clone(), 10, None)
        .await
        .unwrap();
    assert_eq!(sent(&members.page.items, &a), Some(in_public.clone()));
    let members = w
        .rules
        .list_members(&insider, secret.clone(), 10, None)
        .await
        .unwrap();
    assert_eq!(sent(&members.page.items, &a), Some(in_secret.clone()));
    // Adding a member answers with the same members, as the adder sees them.
    let added = w
        .rules
        .add_to_channel(&viewer, public.clone(), vec![insider.handle.clone()])
        .await
        .unwrap();
    assert_eq!(sent(&added.page.items, &a), Some(in_public.clone()));
    // The human and admin agents see every channel, so the plain value.
    let agents = w
        .rules
        .admin_list_agents(&web_page(), None, None, 10, None)
        .await
        .unwrap();
    assert_eq!(sent(&agents.items, &a), Some(in_secret.clone()));
    let page = w
        .rules
        .agent_view(&web_page(), a.handle.clone())
        .await
        .unwrap();
    assert_eq!(page.agent.last_sent, Some(in_secret.clone()));
    let view = w
        .rules
        .channel_view(&web_page(), public.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(sent(&view.members, &a), Some(in_public));
    let members = w
        .rules
        .admin_list_members(&web_page(), secret, 10, None)
        .await
        .unwrap();
    assert_eq!(sent(&members.page.items, &a), Some(in_secret));
}
