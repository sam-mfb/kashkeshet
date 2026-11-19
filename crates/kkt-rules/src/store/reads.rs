//! Every read the store offers, asked many ways on a given database, one
//! answer per line: for the program's test that a database an older
//! release left answers every read as that release did
//! (`tests/migrations/v0_10_0.rs`, its answers pinned in
//! `v0_10_0_reads.txt`). The store's questions stay the rules' own: this is
//! the one way a test outside kkt-rules reads them, and it names none of
//! them.

use super::*;
use std::fmt::Debug;
use std::fmt::Write as _;

/// What to ask about: the avatars, channels and message IDs of the
/// database, some missing on purpose, and the tokens and recovery codes of
/// some avatars, as written, whose hashes the store keeps.
#[derive(Clone, Debug)]
pub struct Probes {
    pub agents: Vec<String>,
    pub channels: Vec<String>,
    pub message_ids: Vec<i64>,
    /// An avatar's handle and its token.
    pub tokens: Vec<(String, String)>,
    /// An avatar's handle and its recovery code.
    pub recovery: Vec<(String, String)>,
}

/// Every read the store offers, asked on `db` with `probes`, one answer per
/// line, in one state of the database. Answers whose order the store does
/// not promise are sorted.
pub async fn every_store_read(db: &Db, probes: &Probes) -> Result<String, StoreError> {
    let probes = probes.clone();
    db.read(move |s| answers(s, &probes)).await
}

/// Scopes as the rules build them, by name, for one viewer.
fn named_scopes(viewer: Option<&str>) -> Vec<(&'static str, ChannelScope)> {
    let v = || viewer.map(str::to_string);
    vec![
        (
            "every",
            ChannelScope::new(kkt_api_agent::SwarmId::main(), v(), |_| true),
        ),
        (
            "none",
            ChannelScope::new(kkt_api_agent::SwarmId::main(), v(), |_| false),
        ),
        (
            "visible",
            ChannelScope::new(kkt_api_agent::SwarmId::main(), v(), |k| {
                !k.hidden && (k.open || k.member)
            }),
        ),
        (
            "joined",
            ChannelScope::new(kkt_api_agent::SwarmId::main(), v(), |k| {
                !k.hidden && k.member
            }),
        ),
        (
            "hidden",
            ChannelScope::new(kkt_api_agent::SwarmId::main(), v(), |k| k.hidden),
        ),
    ]
}

/// One scope per channel kind, for one viewer.
fn kind_scopes(viewer: Option<&str>) -> Vec<(String, ChannelScope)> {
    ChannelKind::all()
        .map(|kind| {
            (
                format!(
                    "kind(hidden={},open={},member={},owner={})",
                    kind.hidden, kind.open, kind.member, kind.owner
                ),
                ChannelScope::new(
                    kkt_api_agent::SwarmId::main(),
                    viewer.map(str::to_string),
                    move |k| k == kind,
                ),
            )
        })
        .collect()
}

fn viewers(probes: &Probes) -> Vec<Option<&str>> {
    std::iter::once(None)
        .chain(probes.agents.iter().map(|a| Some(a.as_str())))
        .collect()
}

/// Every read the store offers, asked many ways, one answer per line.
/// Answers whose order the store does not promise are sorted.
fn answers(s: &Reader<'_>, probes: &Probes) -> Result<String, StoreError> {
    let mut out = String::new();
    let mut line = |question: String, answer: &dyn Debug| {
        writeln!(out, "{question} = {answer:?}").expect("write to a string");
    };

    // The server announcement.
    line(
        "current_server_announcement()".into(),
        &s.current_server_announcement()?,
    );
    line(
        "server_announcement_history()".into(),
        &s.server_announcement_history()?,
    );

    // Lanes.
    line("lanes()".into(), &s.lanes()?);
    for after in [None, Some("old-lane"), Some("reviewer"), Some("zzz")] {
        for limit in [1, 100] {
            line(
                format!("lanes_after({after:?}, {limit})"),
                &s.lanes_after(after, limit)?,
            );
            line(
                format!("takeable_lanes_after({after:?}, {limit})"),
                &s.takeable_lanes_after(after, limit)?,
            );
        }
    }
    for lane in ["reviewer", "old-lane", "writer", "no-lane"] {
        line(format!("lane({lane:?})"), &s.lane(lane)?);
        line(format!("lane_history({lane:?})"), &s.lane_history(lane)?);
        for version in [0, 1, 2, 3] {
            line(
                format!("lane_version({lane:?}, {version})"),
                &s.lane_version(lane, version)?,
            );
        }
    }

    // Agents.
    for agent in probes.agents.iter().map(String::as_str) {
        line(
            format!("agent({agent:?})"),
            &s.agent(&kkt_api_agent::SwarmId::main(), agent)?,
        );
        line(
            format!("agent_name_taken({agent:?})"),
            &s.agent_name_taken(agent)?,
        );
        let upper = agent.to_uppercase();
        line(
            format!("agent_name_taken({upper:?})"),
            &s.agent_name_taken(&upper)?,
        );
        line(
            format!("message_count_from({agent:?})"),
            &s.message_count_from(&kkt_api_agent::SwarmId::main(), agent)?,
        );
        line(
            format!("memory_entries({agent:?})"),
            &s.memory_entries(&kkt_api_agent::SwarmId::main(), agent)?,
        );
        for entry in ["handoff", "open-questions", "nothing"] {
            line(
                format!("memory_entry({agent:?}, {entry:?})"),
                &s.memory_entry(&kkt_api_agent::SwarmId::main(), agent, entry)?,
            );
        }
        line(
            format!("owned_channel_ids({agent:?})"),
            &s.owned_channel_ids(&kkt_api_agent::SwarmId::main(), agent)?,
        );
        line(
            format!("all_channel_ids(Some({agent:?}))"),
            &s.all_channel_ids(&kkt_api_agent::SwarmId::main(), Some(agent))?,
        );
    }
    for (agent, token) in probes
        .tokens
        .iter()
        .map(|(a, t)| (a.as_str(), t.as_str()))
        .chain([("nobody-here", "act_unknown")])
    {
        let found = s.agent_by_token_hash(
            &kkt_api_agent::SwarmId::main(),
            &kkt_hash::sha256(token.as_bytes()),
        )?;
        line(format!("agent_by_token_hash(token of {agent})"), &found);
    }
    for agent in probes.agents.iter().map(String::as_str) {
        for (owner, code) in probes
            .recovery
            .iter()
            .map(|(a, c)| (a.as_str(), c.as_str()))
            .chain([("nobody-here", "arc_x")])
        {
            let found = s.agent_by_recovery_hash(
                &kkt_api_agent::SwarmId::main(),
                agent,
                &kkt_hash::sha256(code.as_bytes()),
            )?;
            line(
                format!("agent_by_recovery_hash({agent:?}, code of {owner})"),
                &found,
            );
        }
    }
    for text in [None, Some("falcon"), Some("FALCON"), Some("zzz")] {
        for lane in [None, Some("reviewer"), Some("old-lane"), Some("writer")] {
            line(
                format!("agents({text:?}, {lane:?}, None, 100)"),
                &s.agents(&kkt_api_agent::SwarmId::main(), text, lane, None, 100)?,
            );
        }
    }
    for after in [None, Some("amber-falcon"), Some("old-wren")] {
        for limit in [1, 100] {
            line(
                format!("agents(None, None, {after:?}, {limit})"),
                &s.agents(&kkt_api_agent::SwarmId::main(), None, None, after, limit)?,
            );
        }
    }
    line(
        "active_agent_count()".into(),
        &s.active_agent_count(&kkt_api_agent::SwarmId::main())?,
    );
    line(
        "agents_by_last_seen()".into(),
        &s.agents_by_last_seen(&kkt_api_agent::SwarmId::main())?,
    );

    // Channels.
    line(
        "all_channel_ids(None)".into(),
        &s.all_channel_ids(&kkt_api_agent::SwarmId::main(), None)?,
    );
    line(
        "hidden_channel_ids()".into(),
        &s.hidden_channel_ids(&kkt_api_agent::SwarmId::main())?,
    );
    for id in probes.channels.iter().map(String::as_str) {
        line(
            format!("channel({id:?})"),
            &s.channel(&kkt_api_agent::SwarmId::main(), id)?,
        );
        line(
            format!("channel_id_taken({id:?})"),
            &s.channel_id_taken(id)?,
        );
        line(
            format!("hidden_channel_summary({id:?})"),
            &s.hidden_channel_summary(&kkt_api_agent::SwarmId::main(), id)?,
        );
        line(
            format!("member_count({id:?})"),
            &s.member_count(&kkt_api_agent::SwarmId::main(), id)?,
        );
        for limit in [1, 100] {
            line(
                format!("members_by_join({id:?}, {limit})"),
                &s.members_by_join(&kkt_api_agent::SwarmId::main(), id, limit)?,
            );
            for after in [None, Some("amber-falcon"), Some("brave-otter")] {
                line(
                    format!("members_by_name({id:?}, {after:?}, {limit})"),
                    &s.members_by_name(&kkt_api_agent::SwarmId::main(), id, after, limit)?,
                );
            }
        }
        for agent in probes.agents.iter().map(String::as_str) {
            line(
                format!("is_member({id:?}, {agent:?})"),
                &s.is_member(&kkt_api_agent::SwarmId::main(), id, agent)?,
            );
        }
        for viewer in viewers(probes) {
            for after in [None, Some(8)] {
                line(
                    format!("channel_summary({id:?}, {viewer:?}, {after:?})"),
                    &s.channel_summary(&kkt_api_agent::SwarmId::main(), id, viewer, after)?,
                );
            }
        }
        line(
            format!("any_channel({id:?})"),
            &s.any_channel(&kkt_api_agent::SwarmId::main(), id)?,
        );
    }
    // Every scope, for every viewer, with the simplest filters.
    for viewer in viewers(probes) {
        let scopes = kind_scopes(viewer).into_iter().chain(
            named_scopes(viewer)
                .into_iter()
                .map(|(n, s)| (n.to_string(), s)),
        );
        for (name, scope) in scopes {
            let q = |what: &str| format!("{what}({viewer:?} {name})");
            line(q("activity after 0"), &s.activity(&scope, 0)?);
            line(q("activity after 41"), &s.activity(&scope, 41)?);
            line(
                q("channel_ids"),
                &s.channel_ids(&ChannelFilter {
                    scope: scope.clone(),
                    shared_with: None,
                    text: None,
                    after: None,
                    limit: 100,
                })?,
            );
            line(
                q("count_newer_messages after 0"),
                &s.count_newer_messages(&NewerFilter {
                    scope: scope.clone(),
                    after_id: 0,
                    channel_ids: None,
                })?,
            );
            line(
                q("latest_message_from amber-falcon"),
                &s.latest_message_from(&SentFilter {
                    agent: "amber-falcon".into(),
                    channel_id: None,
                    scope: scope.clone(),
                })?
                .map(|m| m.id),
            );
            line(
                q("search hello"),
                &s.search(&SearchFilter {
                    scope,
                    words: vec!["hello".into()],
                    phrase: None,
                    channel_id: None,
                    from_agent: None,
                    after_id: None,
                    before_id: None,
                    since: None,
                    until: None,
                    limit: 100,
                })?
                .iter()
                .map(|h| h.message_id)
                .collect::<Vec<_>>(),
            );
        }
    }

    // Every filter, in an agent's view and in the view of everything.
    let views = [
        (
            "brave-otter visible",
            named_scopes(Some("brave-otter")).remove(2).1,
        ),
        ("no viewer every", named_scopes(None).remove(0).1),
    ];
    for (name, scope) in views {
        for shared_with in [None, Some("amber-falcon"), Some("old-wren")] {
            for text in [None, Some("design"), Some("EVERY")] {
                for after in [None, Some(("design".to_string(), "ch_closed".to_string()))] {
                    for limit in [1, 100] {
                        let filter = ChannelFilter {
                            scope: scope.clone(),
                            shared_with: shared_with.map(str::to_string),
                            text: text.map(str::to_string),
                            after: after.clone(),
                            limit,
                        };
                        line(
                            format!(
                                "channel_ids({name}, shared {shared_with:?}, text {text:?}, after {after:?}, {limit})"
                            ),
                            &s.channel_ids(&filter)?,
                        );
                    }
                }
            }
        }
        for after_id in [0, 8, 101] {
            for channel_ids in [
                None,
                Some(vec!["ch_general".to_string(), "ch_hidden".to_string()]),
            ] {
                let filter = NewerFilter {
                    scope: scope.clone(),
                    after_id,
                    channel_ids: channel_ids.clone(),
                };
                for limit in [2, 100] {
                    line(
                        format!("newer_messages({name}, {after_id}, {channel_ids:?}, {limit})"),
                        &s.newer_messages(&filter, limit)?,
                    );
                }
                line(
                    format!("count_newer_messages({name}, {after_id}, {channel_ids:?})"),
                    &s.count_newer_messages(&filter)?,
                );
            }
        }
        for agent in probes.agents.iter().map(String::as_str) {
            for channel_id in [None, Some("ch_general"), Some("ch_hidden")] {
                let filter = SentFilter {
                    agent: agent.to_string(),
                    channel_id: channel_id.map(str::to_string),
                    scope: scope.clone(),
                };
                line(
                    format!("latest_message_from({name}, {agent:?}, {channel_id:?})"),
                    &s.latest_message_from(&filter)?,
                );
            }
        }
        for (words, phrase) in [
            (vec!["hello"], None),
            (vec!["falcon"], None),
            (vec!["builds", "green"], None),
            (vec![], Some("builds are green")),
            (vec!["design"], Some("the falcon")),
            (vec!["plans"], None),
            (vec!["AND", "OR"], None),
        ] {
            let base = SearchFilter {
                scope: scope.clone(),
                words: words.iter().map(|w| w.to_string()).collect(),
                phrase: phrase.map(str::to_string),
                channel_id: None,
                from_agent: None,
                after_id: None,
                before_id: None,
                since: None,
                until: None,
                limit: 100,
            };
            let variants = [
                ("plain", base.clone()),
                (
                    "limit 1",
                    SearchFilter {
                        limit: 1,
                        ..base.clone()
                    },
                ),
                (
                    "in general",
                    SearchFilter {
                        channel_id: Some("ch_general".into()),
                        ..base.clone()
                    },
                ),
                (
                    "from otter",
                    SearchFilter {
                        from_agent: Some("brave-otter".into()),
                        ..base.clone()
                    },
                ),
                (
                    "ids 3..100",
                    SearchFilter {
                        after_id: Some(3),
                        before_id: Some(100),
                        ..base.clone()
                    },
                ),
                (
                    "in october",
                    SearchFilter {
                        since: Some("2026-10-01T00:00:00Z".into()),
                        until: Some("2026-11-01T00:00:00Z".into()),
                        ..base.clone()
                    },
                ),
            ];
            for (variant, filter) in variants {
                line(
                    format!("search({name}, {words:?}, {phrase:?}, {variant})"),
                    &s.search(&filter)?,
                );
            }
        }
    }

    // Messages.
    for id in probes.message_ids.iter().copied() {
        line(
            format!("message({id})"),
            &s.message(&kkt_api_agent::SwarmId::main(), id)?,
        );
    }
    for id in probes.channels.iter().map(String::as_str) {
        for from in [None, Some("amber-falcon"), Some("operator")] {
            for range in [
                MessageRange::Latest,
                MessageRange::After(0),
                MessageRange::After(8),
                MessageRange::Before(100),
                MessageRange::Before(1),
            ] {
                for limit in [1, 100] {
                    line(
                        format!("channel_messages({id:?}, {from:?}, {range:?}, {limit})"),
                        &s.channel_messages(
                            &kkt_api_agent::SwarmId::main(),
                            id,
                            from,
                            range,
                            limit,
                        )?,
                    );
                }
            }
        }
    }
    for limit in [1, 5, 100] {
        line(
            format!("recent_messages({limit})"),
            &s.recent_messages(&kkt_api_agent::SwarmId::main(), limit)?,
        );
    }
    let mut channel_use = s.channel_use(&kkt_api_agent::SwarmId::main())?;
    channel_use.sort_by(|a, b| a.channel_id.cmp(&b.channel_id));
    line("channel_use() sorted".into(), &channel_use);
    line(
        "memberships()".into(),
        &s.memberships(&kkt_api_agent::SwarmId::main())?,
    );
    for since in ["", "2026-10-01T00:00:00Z", "2027-01-01T00:00:00Z"] {
        line(
            format!("message_times_since({since:?})"),
            &s.message_times_since(&kkt_api_agent::SwarmId::main(), since)?,
        );
    }

    // Server runs.
    line("server_runs()".into(), &s.server_runs()?);

    // Everything, for the admin endpoint.
    for text in [None, Some("design"), Some("secret")] {
        for after in [None, Some(("design".to_string(), "ch_closed".to_string()))] {
            for limit in [1, 100] {
                line(
                    format!("any_channel_ids({text:?}, {after:?}, {limit})"),
                    &s.any_channel_ids(
                        &kkt_api_agent::SwarmId::main(),
                        text,
                        after.as_ref(),
                        limit,
                    )?,
                );
            }
        }
    }
    for after_id in [0, 8, 101] {
        line(
            format!("any_activity({after_id})"),
            &s.any_activity(&kkt_api_agent::SwarmId::main(), after_id)?,
        );
        for channel_ids in [
            None,
            Some(vec!["ch_general".to_string(), "ch_hidden".to_string()]),
        ] {
            for limit in [2, 100] {
                line(
                    format!("any_newer_messages({after_id}, {channel_ids:?}, {limit})"),
                    &s.any_newer_messages(
                        &kkt_api_agent::SwarmId::main(),
                        after_id,
                        channel_ids.as_deref(),
                        limit,
                    )?,
                );
            }
            line(
                format!("count_any_newer_messages({after_id}, {channel_ids:?})"),
                &s.count_any_newer_messages(
                    &kkt_api_agent::SwarmId::main(),
                    after_id,
                    channel_ids.as_deref(),
                )?,
            );
        }
    }
    Ok(out)
}
