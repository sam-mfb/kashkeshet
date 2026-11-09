//! Store tests: the methods that hold logic of their own (filters, paging
//! bounds, quoting), and that a failed write changes nothing. Files, copies
//! and reopening are the program's tests (`tests/database_files.rs`).

use super::*;

/// A fresh database in memory, with the core's tables.
fn memory() -> Db {
    let place = kkt_db_sqlite::MemoryPlace::new().expect("an in-memory database");
    Db::new(place.open(&[schema()]).expect("the core's tables"))
}

const T: &str = "2026-10-23T14:00:00Z";
/// The ID of the one server run [`world`] records.
const WORLD_RUN: i64 = 1;

fn run(version: &str) -> NewServerRunRecord {
    NewServerRunRecord {
        started_at: T.into(),
        version: version.into(),
        git_commit: "abc123".into(),
        settings: "default_limit=20".into(),
        instructions_sha256: "00".into(),
    }
}

/// Agents `a`, `b` and `c`; an open channel `ch_open` ("beta", purpose
/// "parser work") with `a` and `b`; a closed channel `ch_closed` ("alpha")
/// with `a` only; and one message per (channel, agent) pair listed.
async fn world(messages: &[(&'static str, &'static str, &'static str)]) -> Db {
    let db = memory();
    let messages = messages.to_vec();
    db.write(move |w| {
        let run_id = w.insert_server_run(&run("1.0.0"))?;
        for name in ["a", "b", "c"] {
            w.insert_agent(
                &SwarmId::main(),
                &NewAgentRecord {
                    name: name.into(),
                    token_hash: name.as_bytes().to_vec(),
                    recovery_hash: None,
                    created_at: T.into(),
                    last_seen: None,
                    run_id,
                },
            )?;
        }
        for (id, name, purpose, open) in [
            ("ch_open", "beta", Some("parser work"), true),
            ("ch_closed", "alpha", None, false),
        ] {
            w.insert_channel(
                &SwarmId::main(),
                &NewChannelRecord {
                    id: id.into(),
                    name: name.into(),
                    purpose: purpose.map(str::to_string),
                    open,
                    created_by: "a".into(),
                    created_at: T.into(),
                    owner: Some("a".into()),
                    run_id,
                },
            )?;
        }
        w.add_member(&SwarmId::main(), "ch_open", "a", T)?;
        w.add_member(&SwarmId::main(), "ch_open", "b", T)?;
        w.add_member(&SwarmId::main(), "ch_closed", "a", T)?;
        for (channel_id, agent, body) in messages {
            w.insert_message(
                &SwarmId::main(),
                &NewMessageRecord {
                    channel_id: channel_id.into(),
                    agent: agent.into(),
                    body: body.into(),
                    created_at: T.into(),
                    run_id,
                },
            )?;
        }
        Ok::<_, StoreError>(())
    })
    .await
    .expect("seed");
    db
}

/// Live channels that are open or that `agent` is in. The rules decide what
/// a scope holds; these two stand in for what they ask for most.
fn visible(agent: &str) -> ChannelScope {
    ChannelScope::new(SwarmId::main(), Some(agent.into()), |k| {
        !k.hidden && (k.open || k.member)
    })
}

/// Live channels `agent` is in.
fn joined(agent: &str) -> ChannelScope {
    ChannelScope::new(SwarmId::main(), Some(agent.into()), |k| {
        !k.hidden && k.member
    })
}

fn filter(viewer: &str) -> ChannelFilter {
    ChannelFilter {
        scope: visible(viewer),
        shared_with: None,
        text: None,
        after: None,
        limit: 100,
    }
}

async fn channel_ids(db: &Db, f: ChannelFilter) -> Vec<String> {
    db.read(move |r| r.channel_ids(&f))
        .await
        .expect("channel_ids")
}

#[tokio::test]
async fn channel_ids_show_closed_channels_to_members_only() {
    let db = world(&[]).await;
    assert_eq!(
        channel_ids(&db, filter("a")).await,
        ["ch_closed", "ch_open"]
    );
    assert_eq!(channel_ids(&db, filter("c")).await, ["ch_open"]);
}

/// A scope selects a channel exactly when its kind is allowed: one channel of
/// each kind, and a scope that allows only that kind, finds only it. With no
/// viewer, no channel counts as joined or owned.
#[tokio::test]
async fn a_scope_selects_exactly_the_kinds_it_allows() {
    let db = world(&[]).await;
    let kinds: Vec<ChannelKind> = ChannelKind::all().collect();
    assert_eq!(kinds.len(), 16);
    let id = |k: &ChannelKind| {
        format!(
            "ch_{}{}{}{}",
            k.hidden as u8, k.open as u8, k.member as u8, k.owner as u8
        )
    };
    let setup: Vec<(String, ChannelKind)> = kinds.iter().map(|k| (id(k), *k)).collect();
    db.write(move |w| {
        for (id, k) in &setup {
            w.insert_channel(
                &SwarmId::main(),
                &NewChannelRecord {
                    id: id.clone(),
                    name: id.clone(),
                    purpose: None,
                    open: k.open,
                    created_by: "a".into(),
                    created_at: T.into(),
                    owner: Some(if k.owner { "c" } else { "a" }.into()),
                    run_id: WORLD_RUN,
                },
            )?;
            if k.hidden {
                w.hide_channel(&SwarmId::main(), id)?;
            }
            if k.member {
                w.add_member(&SwarmId::main(), id, "c", T)?;
            }
        }
        Ok::<_, StoreError>(())
    })
    .await
    .expect("seed");
    let in_world = ["ch_open", "ch_closed"];
    for want in &kinds {
        let only = |viewer: Option<&str>| ChannelFilter {
            scope: ChannelScope::new(SwarmId::main(), viewer.map(str::to_string), |k| k == *want),
            ..filter("c")
        };
        let mut found = channel_ids(&db, only(Some("c"))).await;
        // The world's two channels are live and `c` is not in them.
        found.retain(|id| !in_world.contains(&id.as_str()));
        assert_eq!(found, [id(want)], "{want:?}");
        let mut unseen = channel_ids(&db, only(None)).await;
        unseen.retain(|id| !in_world.contains(&id.as_str()));
        let expected: Vec<String> = if want.member || want.owner {
            vec![]
        } else {
            let mut ids: Vec<String> = kinds
                .iter()
                .filter(|k| k.hidden == want.hidden && k.open == want.open)
                .map(id)
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(unseen, expected, "no viewer, {want:?}");
    }
}

#[tokio::test]
async fn channel_ids_filter_by_membership_sharing_and_text() {
    let db = world(&[]).await;
    let member_only = ChannelFilter {
        scope: joined("b"),
        ..filter("b")
    };
    assert_eq!(channel_ids(&db, member_only).await, ["ch_open"]);
    let shared = ChannelFilter {
        shared_with: Some("b".into()),
        ..filter("a")
    };
    assert_eq!(channel_ids(&db, shared).await, ["ch_open"]);
    // Text matches the purpose as well as the name, and `%` is literal.
    let by_purpose = ChannelFilter {
        text: Some("parser".into()),
        ..filter("a")
    };
    assert_eq!(channel_ids(&db, by_purpose).await, ["ch_open"]);
    let wildcard = ChannelFilter {
        text: Some("%".into()),
        ..filter("a")
    };
    assert!(channel_ids(&db, wildcard).await.is_empty());
}

#[tokio::test]
async fn channel_ids_continue_after_a_name_and_id() {
    let db = world(&[]).await;
    let first = ChannelFilter {
        limit: 1,
        ..filter("a")
    };
    assert_eq!(channel_ids(&db, first).await, ["ch_closed"]);
    let rest = ChannelFilter {
        after: Some(("alpha".into(), "ch_closed".into())),
        ..filter("a")
    };
    assert_eq!(channel_ids(&db, rest).await, ["ch_open"]);
}

#[tokio::test]
async fn channel_messages_respect_range_bounds_and_order() {
    let db = world(&[
        ("ch_open", "a", "1"),
        ("ch_open", "b", "2"),
        ("ch_open", "a", "3"),
        ("ch_open", "b", "4"),
    ])
    .await;
    let ids = |range: MessageRange, from: Option<&'static str>, limit: usize| {
        let db = db.clone();
        async move {
            db.read(move |r| r.channel_messages(&SwarmId::main(), "ch_open", from, range, limit))
                .await
                .expect("messages")
                .into_iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(ids(MessageRange::Latest, None, 2).await, [4, 3]);
    assert_eq!(ids(MessageRange::After(2), None, 10).await, [3, 4]);
    assert_eq!(ids(MessageRange::Before(3), None, 10).await, [2, 1]);
    assert_eq!(ids(MessageRange::Latest, Some("a"), 10).await, [3, 1]);
}

#[tokio::test]
async fn newer_messages_cover_only_my_channels_and_the_ones_listed() {
    let db = world(&[
        ("ch_open", "a", "1"),
        ("ch_closed", "a", "2"),
        ("ch_open", "b", "3"),
    ])
    .await;
    let newer = |member: &'static str, channel_ids: Option<Vec<String>>| {
        let db = db.clone();
        async move {
            db.read(move |r| {
                let f = NewerFilter {
                    scope: joined(member),
                    after_id: 1,
                    channel_ids,
                };
                Ok::<_, StoreError>((r.newer_messages(&f, 10)?, r.count_newer_messages(&f)?))
            })
            .await
            .expect("newer")
        }
    };
    let (messages, count) = newer("a", None).await;
    assert_eq!(messages.iter().map(|m| m.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(count, 2);
    let (messages, count) = newer("a", Some(vec!["ch_open".into()])).await;
    assert_eq!(messages.iter().map(|m| m.id).collect::<Vec<_>>(), [3]);
    assert_eq!(count, 1);
    // `b` is not in the closed channel, so message 2 is not theirs to see.
    let (messages, _) = newer("b", None).await;
    assert_eq!(messages.iter().map(|m| m.id).collect::<Vec<_>>(), [3]);
}

#[tokio::test]
async fn runs_are_listed_newest_first_and_stamp_what_they_create() {
    let db = world(&[("ch_open", "a", "hello")]).await;
    let second = db
        .write(|w| w.insert_server_run(&run("2.0.0")))
        .await
        .expect("second run");
    let runs = db.read(|r| r.server_runs()).await.expect("runs");
    assert_eq!(
        runs.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(),
        ["2.0.0", "1.0.0"]
    );
    assert_eq!(runs[0].id, second);
    let first = runs[1].id;
    let agent = db
        .read(|r| r.agent(&SwarmId::main(), "a"))
        .await
        .expect("agent")
        .expect("a");
    assert_eq!(agent.run_id, Some(first));
    let message = db
        .read(|r| r.message(&SwarmId::main(), 1))
        .await
        .expect("message")
        .expect("1");
    assert_eq!(message.run_id, Some(first));
}

#[tokio::test]
async fn name_pages_start_after_the_cursor() {
    let db = world(&[]).await;
    let names = |text: Option<&'static str>, after: Option<&'static str>, limit: usize| {
        let db = db.clone();
        async move {
            db.read(move |r| r.agents(&SwarmId::main(), text, None, after, limit))
                .await
                .expect("agents")
                .into_iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(names(None, Some("a"), 1).await, ["b"]);
    assert_eq!(names(None, Some("c"), 10).await, Vec::<String>::new());
    // `_` is literal, not a one-character wildcard.
    assert_eq!(names(Some("_"), None, 10).await, Vec::<String>::new());
    let members = db
        .read(|r| r.members_by_name(&SwarmId::main(), "ch_open", Some("a"), 10))
        .await
        .expect("members");
    assert_eq!(
        members.into_iter().map(|a| a.name).collect::<Vec<_>>(),
        ["b"]
    );
}

#[tokio::test]
async fn agent_query_matches_descriptions_with_literal_patterns_and_name_cursors() {
    let db = world(&[]).await;
    db.write(|w| {
        w.set_description(&SwarmId::main(), "a", Some("a worker: #152 100%_\\"))?;
        w.set_description(&SwarmId::main(), "b", Some("WORKER: #152"))?;
        w.set_description(&SwarmId::main(), "c", Some("worker: #152"))?;
        w.hide_agent(&SwarmId::main(), "c", T)
    })
    .await
    .unwrap();
    let first = db
        .read(|r| r.agents(&SwarmId::main(), Some("worker: #152"), None, None, 1))
        .await
        .unwrap();
    assert_eq!(
        first.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
    let second = db
        .read(|r| r.agents(&SwarmId::main(), Some("worker: #152"), None, Some("a"), 10))
        .await
        .unwrap();
    assert_eq!(
        second.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["b"]
    );
    for query in ["%_\\", "a"] {
        let rows = db
            .read(move |r| r.agents(&SwarmId::main(), Some(query), None, None, 10))
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
    }
    db.write(|w| w.set_description(&SwarmId::main(), "a", None))
        .await
        .unwrap();
    assert!(
        db.read(|r| r.agents(&SwarmId::main(), Some("100%"), None, None, 10))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn clients_are_replaced_cleared_and_listed_with_members() {
    let db = world(&[]).await;
    let run_id = db.read(|r| r.server_runs()).await.unwrap()[0].id;
    let client = AgentClient {
        name: "Codex".into(),
        version: "1.2".into(),
        recorded_at: T.into(),
        run_id,
    };
    let saved = client.clone();
    db.write(move |w| w.set_agent_client(&SwarmId::main(), "b", Some(&saved)))
        .await
        .unwrap();
    let members = db
        .read(|r| r.members_by_name(&SwarmId::main(), "ch_open", None, 10))
        .await
        .unwrap();
    assert_eq!(members[0].client, None);
    assert_eq!(members[1].client, Some(client));
    assert!(
        db.write(|w| w.set_agent_client(
            &SwarmId::main(),
            "b",
            Some(&AgentClient {
                name: "bad".into(),
                version: "0".into(),
                recorded_at: T.into(),
                run_id: 999
            })
        ))
        .await
        .is_err()
    );
    assert_eq!(
        db.read(|r| r.agent(&SwarmId::main(), "b"))
            .await
            .unwrap()
            .unwrap()
            .client
            .unwrap()
            .name,
        "Codex"
    );
    db.write(|w| w.set_agent_client(&SwarmId::main(), "b", None))
        .await
        .unwrap();
    assert_eq!(
        db.read(|r| r.agent(&SwarmId::main(), "b"))
            .await
            .unwrap()
            .unwrap()
            .client,
        None
    );
}

#[tokio::test]
async fn descriptions_are_set_cleared_and_listed_with_members() {
    let db = world(&[]).await;
    db.write(|w| w.set_description(&SwarmId::main(), "b", Some("parser bot")))
        .await
        .expect("set");
    let members = db
        .read(|r| r.members_by_name(&SwarmId::main(), "ch_open", None, 10))
        .await
        .expect("members");
    let described: Vec<_> = members
        .iter()
        .map(|a| (a.name.as_str(), a.description.as_deref()))
        .collect();
    assert_eq!(described, [("a", None), ("b", Some("parser bot"))]);
    db.write(|w| w.set_description(&SwarmId::main(), "b", None))
        .await
        .expect("clear");
    let b = db
        .read(|r| r.agent(&SwarmId::main(), "b"))
        .await
        .expect("agent")
        .expect("b");
    assert_eq!(b.description, None);
}

#[tokio::test]
async fn search_treats_query_syntax_as_words() {
    let db = world(&[
        ("ch_open", "a", "parser OR lexer"),
        ("ch_open", "b", "just the parser"),
        ("ch_closed", "a", "parser in alpha"),
    ])
    .await;
    let hits = |member: &'static str, words: &'static [&'static str]| {
        let db = db.clone();
        async move {
            db.read(move |r| {
                r.search(&SearchFilter {
                    scope: joined(member),
                    words: words.iter().map(|w| w.to_string()).collect(),
                    phrase: None,
                    channel_id: None,
                    from_agent: None,
                    after_id: None,
                    before_id: None,
                    since: None,
                    until: None,
                    limit: 10,
                })
            })
            .await
            .expect("search")
            .into_iter()
            .map(|h| h.message_id)
            .collect::<Vec<_>>()
        }
    };
    // `OR` is a word to find, not an operator.
    assert_eq!(hits("a", &["parser", "OR"]).await, [1]);
    assert_eq!(hits("a", &["parser"]).await, [3, 2, 1]);
    // `b` is not in the closed channel.
    assert_eq!(hits("b", &["parser"]).await, [2, 1]);
    // A stray quote or a lone operator does not break the query.
    assert!(hits("a", &["pars\"er"]).await.is_empty());
    assert!(hits("a", &["NOT"]).await.is_empty());
}

#[tokio::test]
async fn a_failed_write_changes_nothing() {
    let db = world(&[]).await;
    let failed = db
        .write(|w| {
            w.insert_channel(
                &SwarmId::main(),
                &NewChannelRecord {
                    id: "ch_x".into(),
                    name: "x".into(),
                    purpose: None,
                    open: true,
                    created_by: "a".into(),
                    created_at: T.into(),
                    owner: None,
                    run_id: WORLD_RUN,
                },
            )?;
            w.add_member(&SwarmId::main(), "ch_x", "a", T)?;
            Err::<(), _>(StoreError::Database("stop".into()))
        })
        .await;
    assert!(failed.is_err());
    let found = db
        .read(|r| r.channel(&SwarmId::main(), "ch_x"))
        .await
        .expect("read");
    assert_eq!(found, None);
    let mine = ChannelFilter {
        scope: joined("a"),
        ..filter("a")
    };
    assert_eq!(channel_ids(&db, mine).await, ["ch_closed", "ch_open"]);
}

#[tokio::test]
async fn a_taken_name_is_found_whatever_its_case() {
    let db = world(&[]).await;
    db.write(|w| {
        let run_id = w.insert_server_run(&run("case"))?;
        w.insert_agent(
            &SwarmId::main(),
            &NewAgentRecord {
                name: "Mixed-Case".into(),
                token_hash: vec![21],
                recovery_hash: None,
                created_at: T.into(),
                last_seen: None,
                run_id,
            },
        )
    })
    .await
    .unwrap();
    db.read(|r| {
        for taken in ["a", "A", "Mixed-Case", "mixed-case", "MIXED-CASE"] {
            assert!(r.agent_name_taken(taken)?, "{taken}");
        }
        for free in ["d", "mixed", "mixed-case-2", "%", "_"] {
            assert!(!r.agent_name_taken(free)?, "{free}");
        }
        Ok::<_, StoreError>(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn recovery_lookup_requires_both_name_and_hash_and_token_replacement_is_local() {
    let db = world(&[]).await;
    db.write(|w| {
        let run_id = w.insert_server_run(&run("recovery"))?;
        w.insert_agent(
            &SwarmId::main(),
            &NewAgentRecord {
                name: "recoverable".into(),
                token_hash: vec![11],
                recovery_hash: Some(vec![42]),
                created_at: T.into(),
                last_seen: None,
                run_id,
            },
        )
    })
    .await
    .unwrap();
    db.write(|w| {
        assert!(
            w.agent_by_recovery_hash(&SwarmId::main(), "recoverable", &[42])?
                .is_some()
        );
        assert!(
            w.agent_by_recovery_hash(&SwarmId::main(), "recoverable", &[41])?
                .is_none()
        );
        assert!(
            w.agent_by_recovery_hash(&SwarmId::main(), "a", &[42])?
                .is_none()
        );
        assert!(
            w.agent_by_recovery_hash(&SwarmId::main(), "a", &[])?
                .is_none()
        );
        w.set_token_hash(&SwarmId::main(), "recoverable", &[12])?;
        Ok::<_, StoreError>(())
    })
    .await
    .unwrap();
    db.read(|r| {
        assert!(r.agent_by_token_hash(&SwarmId::main(), &[11])?.is_none());
        assert_eq!(
            r.agent_by_token_hash(&SwarmId::main(), &[12])?
                .unwrap()
                .name,
            "recoverable"
        );
        assert!(r.agent_by_token_hash(&SwarmId::main(), b"a")?.is_some());
        assert!(
            r.agent_by_recovery_hash(&SwarmId::main(), "recoverable", &[42])?
                .is_some()
        );
        Ok::<_, StoreError>(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn channel_use_counts_messages_and_names_the_last_speaker() {
    let db = world(&[
        ("ch_open", "a", "one"),
        ("ch_open", "b", "two"),
        ("ch_open", "a", "three"),
        ("ch_closed", "a", "solo"),
    ])
    .await;
    // A channel with no messages is listed too.
    db.write(|w| {
        w.insert_channel(
            &SwarmId::main(),
            &NewChannelRecord {
                id: "ch_quiet".into(),
                name: "quiet".into(),
                purpose: None,
                open: true,
                created_by: "c".into(),
                created_at: T.into(),
                owner: None,
                run_id: WORLD_RUN,
            },
        )?;
        w.insert_message(
            &SwarmId::main(),
            &NewMessageRecord {
                channel_id: "ch_open".into(),
                agent: "b".into(),
                body: "four".into(),
                created_at: "2026-10-23T14:05:00Z".into(),
                run_id: 1,
            },
        )?;
        Ok::<_, StoreError>(())
    })
    .await
    .expect("seed");
    let mut used = db
        .read(|r| r.channel_use(&SwarmId::main()))
        .await
        .expect("read");
    used.sort_by(|x, y| x.channel_id.cmp(&y.channel_id));
    let at = |s: &str| Some(s.to_string());
    assert_eq!(
        used,
        [
            ChannelUseRecord {
                channel_id: "ch_closed".into(),
                message_count: 1,
                last_message_at: at(T),
                last_speaker: at("a"),
            },
            ChannelUseRecord {
                channel_id: "ch_open".into(),
                message_count: 4,
                last_message_at: at("2026-10-23T14:05:00Z"),
                last_speaker: at("b"),
            },
            ChannelUseRecord {
                channel_id: "ch_quiet".into(),
                message_count: 0,
                last_message_at: None,
                last_speaker: None,
            },
        ]
    );
}

#[tokio::test]
async fn memberships_list_every_agent_in_every_channel() {
    let db = world(&[]).await;
    let all = db
        .read(|r| r.memberships(&SwarmId::main()))
        .await
        .expect("read");
    let pairs: Vec<(String, String, String)> = all
        .into_iter()
        .map(|m| (m.channel_id, m.channel_name, m.agent))
        .collect();
    let pair = |c: &str, n: &str, a: &str| (c.to_string(), n.to_string(), a.to_string());
    assert_eq!(
        pairs,
        [
            pair("ch_open", "beta", "a"),
            pair("ch_open", "beta", "b"),
            pair("ch_closed", "alpha", "a"),
        ]
    );
}

#[tokio::test]
async fn message_times_since_finds_every_message_in_the_window() {
    let db = world(&[("ch_open", "a", "old one"), ("ch_closed", "a", "old two")]).await;
    db.write(|w| {
        for (channel_id, at) in [
            ("ch_open", "2026-10-23T14:10:00Z"),
            ("ch_closed", "2026-10-23T14:11:00Z"),
            ("ch_open", "2026-10-23T14:12:00Z"),
            // Stored after the others but stamped earlier, as when the clock
            // was read before the write got its turn, or the clock moved
            // back. It must not hide the rest.
            ("ch_closed", "2026-10-23T13:00:00Z"),
        ] {
            w.insert_message(
                &SwarmId::main(),
                &NewMessageRecord {
                    channel_id: channel_id.into(),
                    agent: "a".into(),
                    body: "new".into(),
                    created_at: at.into(),
                    run_id: 1,
                },
            )?;
        }
        Ok::<_, StoreError>(())
    })
    .await
    .expect("seed");
    let times = db
        .read(|r| r.message_times_since(&SwarmId::main(), "2026-10-23T14:10:00Z"))
        .await
        .expect("read");
    let pairs: Vec<(&str, &str)> = times
        .iter()
        .map(|t| (t.channel_id.as_str(), t.created_at.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("ch_open", "2026-10-23T14:10:00Z"),
            ("ch_closed", "2026-10-23T14:11:00Z"),
            ("ch_open", "2026-10-23T14:12:00Z"),
        ]
    );
    let none = db
        .read(|r| r.message_times_since(&SwarmId::main(), "2026-10-23T15:00:00Z"))
        .await
        .expect("read");
    assert!(none.is_empty());
}

#[tokio::test]
async fn two_swarms_stay_apart() {
    let db = memory();
    let outcome = db
        .write(|w| Ok::<_, StoreError>(testing::swarms_stay_apart(w)))
        .await
        .expect("write");
    if let Err(leak) = outcome {
        panic!("{leak}");
    }
}

/// A scope over more channels than SQLite takes parameters in one
/// statement (32,766) still answers: the scope's channels are found inside
/// the question, not listed into it.
#[tokio::test]
async fn a_scope_over_more_channels_than_sqlite_takes_parameters() {
    const CHANNELS: usize = 33_000;
    let db = world(&[("ch_open", "a", "hello")]).await;
    db.write(|w| {
        for i in 0..CHANNELS {
            w.insert_channel(
                &SwarmId::main(),
                &NewChannelRecord {
                    id: format!("ch_many_{i}"),
                    name: format!("many {i}"),
                    purpose: None,
                    open: true,
                    created_by: "a".into(),
                    created_at: T.into(),
                    owner: None,
                    run_id: WORLD_RUN,
                },
            )?;
        }
        Ok::<_, StoreError>(())
    })
    .await
    .expect("many open channels");
    let scope = visible("c");
    let activity = db
        .read(move |r| r.activity(&scope, 0))
        .await
        .expect("activity over every open channel");
    assert_eq!(
        activity
            .iter()
            .map(|a| a.channel_id.as_str())
            .collect::<Vec<_>>(),
        ["ch_open"]
    );
    let newer = db
        .read(|r| {
            r.count_newer_messages(&NewerFilter {
                scope: visible("c"),
                after_id: 0,
                channel_ids: None,
            })
        })
        .await
        .expect("newer messages over every open channel");
    assert_eq!(newer, 1);
}
