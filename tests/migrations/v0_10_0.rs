//! A populated database at 0.10.0's schema (0021), and what every read of
//! the store answers on it.
//!
//! The answers are pinned in `v0_10_0_reads.txt`, recorded with the store as
//! 0.10.0 released it. A later version must open the same database and give
//! the same answers, so a migration or a change to the store's queries that
//! alters what an existing install sees shows up as a failure here. Rewrite
//! the file only for a change that is meant to alter an answer, with
//! `UPDATE_READS=1 cargo test --test migrations`, and review the diff.

use super::temp_dir;
use kkt_rules::StoreError;
use kkt_rules::testing::{
    self, ChannelFilter, ChannelKind, ChannelScope, MessageRange, NewerFilter, Reader,
    SearchFilter, SentFilter,
};
use rusqlite::Connection;
use std::fmt::Debug;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const UPDATE_VAR: &str = "UPDATE_READS";

/// Every migration 0.10.0 shipped, in order.
const SCHEMA_0021: [&str; 21] = [
    include_str!("sql/0001_init.sql"),
    include_str!("sql/0002_server_runs.sql"),
    include_str!("sql/0003_revoked_agents.sql"),
    include_str!("sql/0004_identity_recovery.sql"),
    include_str!("sql/0005_messages_by_time.sql"),
    include_str!("sql/0006_agent_descriptions.sql"),
    include_str!("sql/0007_channel_notes.sql"),
    include_str!("sql/0008_operator_messages.sql"),
    include_str!("sql/0009_handoff_notes.sql"),
    include_str!("sql/0010_hidden_entries.sql"),
    include_str!("sql/0011_operator_instructions.sql"),
    include_str!("sql/0012_operator_channels.sql"),
    include_str!("sql/0013_channel_owners.sql"),
    include_str!("sql/0014_agent_clients.sql"),
    include_str!("sql/0015_instruction_authors.sql"),
    include_str!("sql/0016_lanes.sql"),
    include_str!("sql/0017_agent_lanes.sql"),
    include_str!("sql/0018_lane_sources.sql"),
    include_str!("sql/0019_memory.sql"),
    include_str!("sql/0020_channel_runs.sql"),
    include_str!("sql/0021_messages_by_agent.sql"),
];

/// The avatars' tokens and recovery codes. The database keeps only their
/// SHA-256 hashes, as every version so far has.
const TOKENS: [(&str, &str); 5] = [
    ("amber-falcon", "act_amberfalcon00000000000000"),
    ("brave-otter", "act_braveotter000000000000000"),
    ("calm-heron", "act_calmheron0000000000000000"),
    ("old-wren", "act_oldwren00000000000000000"),
    ("quiet-lynx", "act_quietlynx0000000000000000"),
];
const RECOVERY: [(&str, &str); 3] = [
    ("amber-falcon", "arc_amberfalcon00000000000000"),
    ("brave-otter", "arc_braveotter000000000000000"),
    ("calm-heron", "arc_calmheron0000000000000000"),
];

/// Avatars, including one that does not exist.
const AGENTS: [&str; 6] = [
    "amber-falcon",
    "brave-otter",
    "calm-heron",
    "old-wren",
    "quiet-lynx",
    "nobody-here",
];

/// Channels, including one that does not exist.
const CHANNELS: [&str; 7] = [
    "ch_general",
    "ch_closed",
    "ch_revoked",
    "ch_ownerless",
    "ch_hidden",
    "ch_empty",
    "ch_missing",
];

/// Every stored message ID, a gap between them, and IDs past the last.
const MESSAGE_IDS: [i64; 13] = [1, 2, 3, 7, 8, 15, 40, 41, 42, 99, 100, 101, 500];

fn hash(secret: &str) -> String {
    kkt_hash::sha256(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Builds the database at `path`: the tables exactly as 0.10.0 left them,
/// with rows of every sort an install may hold. Revoked and hidden avatars,
/// a revoked owner, hidden and ownerless channels, an empty channel, notes
/// edited before and after runs were recorded for them, memory from a
/// handoff note and from `remember`, lanes with a retired one and an avatar
/// pinned to an older version, the announcement's versions, messages from
/// before runs were recorded and from the operator, and message IDs with
/// gaps, as deleted rows and failed inserts leave them.
fn make(path: &Path) {
    let conn = Connection::open(path).expect("open the 0.10.0 database");
    conn.pragma_update(None, "foreign_keys", "OFF")
        .expect("foreign keys off, as the upgrader runs migrations");
    for migration in SCHEMA_0021 {
        conn.execute_batch(migration).expect("0.10.0 schema");
    }
    conn.pragma_update(None, "user_version", 21)
        .expect("schema version");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign keys on");

    let token = |agent: &str| {
        let (_, t) = TOKENS.iter().find(|(a, _)| *a == agent).expect("token");
        hash(t)
    };
    let recovery = |agent: &str| {
        let (_, r) = RECOVERY.iter().find(|(a, _)| *a == agent).expect("code");
        hash(r)
    };
    let sql = format!(
        "INSERT INTO server_runs (id, started_at, version, git_commit, settings, instructions_sha256) VALUES
             (1, '2026-09-01T08:00:00Z', '0.8.0', 'aaaaaaa', 'bind=127.0.0.1:7777', '01ab'),
             (2, '2026-10-01T08:00:00Z', '0.9.0', 'bbbbbbb-dirty', 'bind=127.0.0.1:7777', '02cd'),
             (3, '2026-11-01T08:00:00Z', '0.10.0', 'unknown', 'bind=0.0.0.0:7777', '03ef');

         INSERT INTO operator_instructions (text, saved_at, run_id, saved_by_admin) VALUES
             ('Be kind.', '2026-09-02T00:00:00Z', 1, NULL),
             ('Be kind and brief.', '2026-10-02T00:00:00Z', 2, 'admin-sentinel');

         INSERT INTO lanes (name, created_at, run_id, retired_at, retired_by_admin, retired_run_id) VALUES
             ('reviewer', '2026-09-03T00:00:00Z', 1, NULL, NULL, NULL),
             ('old-lane', '2026-09-04T00:00:00Z', 1, '2026-10-04T00:00:00Z', 'admin-sentinel', 2),
             ('writer', '2026-10-05T00:00:00Z', 2, NULL, NULL, NULL);
         INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id, saved_by_admin, source_folder, source_commit) VALUES
             ('reviewer', 1, 'Reviews code', 'Review every pull request.', '2026-09-03T00:00:00Z', 1, NULL, NULL, NULL),
             ('reviewer', 2, 'Reviews code closely', 'Review every pull request twice.', '2026-10-03T00:00:00Z', 2, 'admin-sentinel', NULL, NULL),
             ('old-lane', 1, 'Old work', 'Do the old work.', '2026-09-04T00:00:00Z', 1, NULL, NULL, NULL),
             ('writer', 1, 'Writes docs', 'Write the docs.', '2026-10-05T00:00:00Z', 2, NULL, '/srv/lanes', 'ccccccc'),
             ('writer', 2, 'Writes docs', 'Write the docs plainly.', '2026-11-02T00:00:00Z', 3, NULL, '/srv/lanes', NULL);

         INSERT INTO agents (name, token_hash, created_at, last_seen, run_id, revoked_at, recovery_hash,
                             description, handoff_note, handoff_note_set_at, hidden,
                             client_name, client_version, client_recorded_at, client_run_id,
                             lane, lane_version, lane_taken_at, lane_run_id) VALUES
             ('old-wren', x'{old_wren}', '2026-08-01T00:00:00Z', NULL, NULL, NULL, NULL,
              NULL, '', NULL, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL),
             ('amber-falcon', x'{amber}', '2026-09-01T09:00:00Z', '2026-11-01T09:30:00Z', 1, NULL, x'{amber_rc}',
              'reviewer for #152', '', NULL, 0, 'Claude Code', '2.1.0', '2026-11-01T09:00:00Z', 3,
              'reviewer', 1, '2026-09-05T00:00:00Z', 1),
             ('brave-otter', x'{brave}', '2026-09-01T10:00:00Z', '2026-10-01T10:00:00Z', 1, '2026-10-10T00:00:00Z', x'{brave_rc}',
              'Builder of things', '', NULL, 0, 'Codex', '0.9.1', '2026-09-01T10:00:00Z', 1,
              NULL, NULL, NULL, NULL),
             ('calm-heron', x'{calm}', '2026-09-02T10:00:00Z', '2026-09-03T10:00:00Z', 1, '2026-10-11T00:00:00Z', x'{calm_rc}',
              NULL, '', NULL, 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL),
             ('quiet-lynx', x'{quiet}', '2026-10-01T11:00:00Z', '2026-11-02T11:00:00Z', 2, NULL, NULL,
              'falcon watcher', '', NULL, 0, NULL, NULL, NULL, NULL,
              'old-lane', 1, '2026-09-20T00:00:00Z', 1);

         INSERT INTO memory_entries (agent, name, text, saved_at, run_id) VALUES
             ('amber-falcon', 'handoff', 'From the old handoff note.', '2026-09-10T00:00:00Z', NULL),
             ('amber-falcon', 'open-questions', 'Why is the sky blue?', '2026-11-01T10:00:00Z', 3),
             ('calm-heron', 'handoff', 'Hidden but remembered.', '2026-09-03T00:00:00Z', 1);

         INSERT INTO channels (id, name, purpose, is_open, created_by, created_at, note, note_edited_by,
                               note_edited_at, hidden, owner, run_id, note_run_id) VALUES
             ('ch_general', 'general', 'Everything', 1, 'amber-falcon', '2026-09-01T09:05:00Z',
              'Read the README first.', 'amber-falcon', '2026-11-01T09:10:00Z', 0, 'amber-falcon', NULL, 3),
             ('ch_closed', 'design', 'Private design talk', 0, 'amber-falcon', '2026-09-01T09:06:00Z',
              'Old note', 'brave-otter', '2026-09-15T00:00:00Z', 0, 'amber-falcon', NULL, NULL),
             ('ch_revoked', 'builds', NULL, 1, 'brave-otter', '2026-10-01T10:05:00Z',
              '', NULL, NULL, 0, 'brave-otter', 2, NULL),
             ('ch_ownerless', 'announcements', 'From the operator', 1, 'operator', '2026-09-01T08:30:00Z',
              '', 'amber-falcon', '2026-10-20T00:00:00Z', 0, NULL, NULL, 2),
             ('ch_hidden', 'secret plans', 'Hidden by its owner', 0, 'amber-falcon', '2026-09-05T00:00:00Z',
              'Do not look', 'amber-falcon', '2026-09-06T00:00:00Z', 1, 'amber-falcon', NULL, NULL),
             ('ch_empty', 'design', 'Nothing yet', 0, 'quiet-lynx', '2026-11-02T11:05:00Z',
              '', NULL, NULL, 0, 'quiet-lynx', 3, NULL);

         INSERT INTO memberships (channel_id, agent, joined_at) VALUES
             ('ch_ownerless', 'old-wren', '2026-09-01T08:31:00Z'),
             ('ch_general', 'amber-falcon', '2026-09-01T09:05:00Z'),
             ('ch_closed', 'amber-falcon', '2026-09-01T09:06:00Z'),
             ('ch_general', 'brave-otter', '2026-09-01T10:01:00Z'),
             ('ch_closed', 'brave-otter', '2026-09-01T10:02:00Z'),
             ('ch_revoked', 'brave-otter', '2026-10-01T10:05:00Z'),
             ('ch_revoked', 'old-wren', '2026-10-01T10:06:00Z'),
             ('ch_general', 'quiet-lynx', '2026-10-01T11:01:00Z'),
             ('ch_empty', 'quiet-lynx', '2026-11-02T11:05:00Z'),
             ('ch_ownerless', 'amber-falcon', '2026-11-01T09:20:00Z');

         INSERT INTO messages (id, channel_id, agent, body, created_at, run_id) VALUES
             (1, 'ch_ownerless', 'operator', 'Welcome to the server, everyone.', '2026-09-01T08:30:00Z', NULL),
             (2, 'ch_ownerless', 'old-wren', 'hello from the oldest avatar', '2026-09-01T08:32:00Z', NULL),
             (3, 'ch_general', 'amber-falcon', 'hello general, the falcon is here', '2026-09-01T09:05:30Z', 1),
             (7, 'ch_closed', 'amber-falcon', 'secret design for the falcon release', '2026-09-01T09:07:00Z', 1),
             (8, 'ch_closed', 'brave-otter', 'the otter agrees with the design', '2026-09-01T10:03:00Z', 1),
             (15, 'ch_hidden', 'amber-falcon', 'plans nobody should find', '2026-09-05T00:01:00Z', 1),
             (40, 'ch_general', 'calm-heron', 'a heron passed through general', '2026-09-03T10:00:00Z', 1),
             (41, 'ch_general', 'brave-otter', 'builds are green again', '2026-10-01T10:04:00Z', 2),
             (42, 'ch_revoked', 'brave-otter', 'builds channel is open', '2026-10-01T10:05:30Z', 2),
             (99, 'ch_revoked', 'old-wren', 'release builds look green', '2026-10-02T00:00:00Z', 2),
             (100, 'ch_general', 'operator', 'The operator made quiet-lynx a member.', '2026-10-01T11:01:00Z', 2),
             (101, 'ch_general', 'quiet-lynx', 'hello falcon, hello otter', '2026-11-02T11:02:00Z', 3);",
        old_wren = token("old-wren"),
        amber = token("amber-falcon"),
        amber_rc = recovery("amber-falcon"),
        brave = token("brave-otter"),
        brave_rc = recovery("brave-otter"),
        calm = token("calm-heron"),
        calm_rc = recovery("calm-heron"),
        quiet = token("quiet-lynx"),
    );
    conn.execute_batch(&sql).expect("0.10.0 rows");
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

fn viewers() -> Vec<Option<&'static str>> {
    std::iter::once(None)
        .chain(AGENTS.iter().map(|a| Some(*a)))
        .collect()
}

/// Every read the store offers, asked many ways, one answer per line.
/// Answers whose order the store does not promise are sorted.
fn every_read(s: &Reader<'_>) -> Result<String, StoreError> {
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
    for agent in AGENTS {
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
    for (agent, token) in TOKENS
        .iter()
        .copied()
        .chain([("nobody-here", "act_unknown")])
    {
        let found = s.agent_by_token_hash(
            &kkt_api_agent::SwarmId::main(),
            &kkt_hash::sha256(token.as_bytes()),
        )?;
        line(format!("agent_by_token_hash(token of {agent})"), &found);
    }
    for agent in AGENTS {
        for (owner, code) in RECOVERY.iter().copied().chain([("nobody-here", "arc_x")]) {
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
    for id in CHANNELS {
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
        for agent in AGENTS {
            line(
                format!("is_member({id:?}, {agent:?})"),
                &s.is_member(&kkt_api_agent::SwarmId::main(), id, agent)?,
            );
        }
        for viewer in viewers() {
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
    for viewer in viewers() {
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
        for agent in AGENTS {
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
    for id in MESSAGE_IDS {
        line(
            format!("message({id})"),
            &s.message(&kkt_api_agent::SwarmId::main(), id)?,
        );
    }
    for id in CHANNELS {
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

fn reads_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/migrations/v0_10_0_reads.txt")
}

/// What every read answers on the 0.10.0 database once this version has
/// opened it.
async fn reads_after_opening(test: &str) -> String {
    let dir = temp_dir(test);
    let path = dir.join("v0_10_0.db");
    make(&path);
    let db = kashkeshet::machine::database::open(&path).expect("open the 0.10.0 database");
    testing::read(&db, every_read).await.expect("every read")
}

#[tokio::test]
async fn a_v0_10_0_database_answers_every_read_as_0_10_0_did() {
    let actual = reads_after_opening("v0_10_0-reads").await;
    let path = reads_path();
    if std::env::var_os(UPDATE_VAR).is_some() {
        std::fs::write(&path, &actual).expect("write the pinned reads");
        return;
    }
    let pinned = std::fs::read_to_string(&path).expect("read the pinned reads");
    if actual != pinned {
        let first = actual
            .lines()
            .zip(pinned.lines())
            .find(|(a, p)| a != p)
            .map(|(a, p)| where_they_differ(a, p))
            .unwrap_or_else(|| "\n  the answers differ in length".into());
        panic!(
            "a read answers differently on the 0.10.0 database; first difference:{first}\n\
             If the change is intended, rewrite {} with {UPDATE_VAR}=1 and review the diff.",
            path.display()
        );
    }
}

/// Where two lines first differ: the question, the column, and some text
/// around it in each.
fn where_they_differ(now: &str, pinned: &str) -> String {
    let question = now.split_once(" = ").map_or(now, |(q, _)| q);
    let at = now
        .char_indices()
        .zip(pinned.chars())
        .find(|((_, a), b)| a != b)
        .map_or(now.len().min(pinned.len()), |((i, _), _)| i);
    let around = |line: &str| {
        let start = line[..at.min(line.len())]
            .char_indices()
            .rev()
            .nth(40)
            .map_or(0, |(i, _)| i);
        line[start..].chars().take(100).collect::<String>()
    };
    format!(
        "\n  {question}\n  at column {at}\n  now:    …{}\n  pinned: …{}",
        around(now),
        around(pinned)
    )
}

/// What a database is, as SQLite holds it: every table, index and
/// trigger with its SQL, the versions recorded, and `user_version`.
type Shape = (Vec<String>, Vec<String>, i64);

fn shape(path: &Path) -> Shape {
    let conn = Connection::open(path).expect("open");
    let objects = strings(
        &conn,
        "SELECT type || ' ' || name || ' on ' || tbl_name || ': ' || coalesce(sql, '')
         FROM sqlite_master ORDER BY type, name",
    );
    let versions = strings(
        &conn,
        "SELECT name || ' ' || version FROM kkt_db_versions ORDER BY name",
    );
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("user_version");
    (objects, versions, version)
}

/// The shape of a database this version makes new.
fn new_shape(test: &str) -> Shape {
    let dir = temp_dir(test);
    let path = dir.join("new.db");
    drop(kashkeshet::machine::database::open(&path).expect("a new database"));
    shape(&path)
}

/// Sam's requirement for the baseline (2026-11-09): an upgraded 0.10.0
/// database and a new one end up the same, table for table, index for
/// index and trigger for trigger, byte for byte.
#[tokio::test]
async fn a_v0_10_0_database_becomes_exactly_what_a_new_one_is() {
    let dir = temp_dir("v0_10_0-shape");
    let path = dir.join("v0_10_0.db");
    make(&path);
    drop(kashkeshet::machine::database::open(&path).expect("open the 0.10.0 database"));
    let adopted = shape(&path);
    assert_eq!(adopted, new_shape("v0_10_0-shape-new"));
    assert!(
        adopted.0.iter().any(|o| o.starts_with("trigger ")),
        "{adopted:?}"
    );
    assert_eq!(adopted.1, ["core 2"]);
}

/// A database made by `next` since swarms (schema 22) is adopted too, and
/// answers every read as the 0.10.0 database does.
#[tokio::test]
async fn a_next_database_with_swarms_is_adopted_too() {
    let dir = temp_dir("next-22");
    let path = dir.join("next.db");
    make(&path);
    {
        let conn = Connection::open(&path).expect("open");
        // As kkt's old migration runner ran it: foreign keys off.
        conn.pragma_update(None, "foreign_keys", "OFF")
            .expect("foreign keys off");
        conn.execute_batch(include_str!("sql/0022_swarms.sql"))
            .expect("next's swarms");
        conn.pragma_update(None, "user_version", 22)
            .expect("version 22");
    }
    let pending = kashkeshet::machine::database::pending_upgrade(&path).expect("pending");
    assert_eq!(
        pending.map(|p| (p.from, p.to)),
        Some(("22".to_string(), "core 2".to_string())),
        "adopting is an upgrade, which the backup before it must see"
    );
    let db = kashkeshet::machine::database::open(&path).expect("open next's database");
    let reads = testing::read(&db, every_read).await.expect("every read");
    drop(db);
    let pinned = std::fs::read_to_string(reads_path()).expect("read the pinned reads");
    assert!(reads == pinned, "{}", where_they_differ(&reads, &pinned));
    assert_eq!(shape(&path), new_shape("next-22-new"));
}

/// Message IDs only go up, across adopting too; and a key whose counter
/// had run past its largest row goes on from the counter.
#[tokio::test]
async fn ids_and_versions_keep_rising_after_adoption() {
    use kkt_api_agent::SwarmId;
    use kkt_rules::testing::NewMessageRecord;
    let dir = temp_dir("v0_10_0-ids");
    let path = dir.join("v0_10_0.db");
    make(&path);
    let (newest, announcement) = {
        let conn = Connection::open(&path).expect("open");
        let newest: i64 = conn
            .query_row("SELECT max(id) FROM messages", [], |r| r.get(0))
            .expect("newest message");
        let version: i64 = conn
            .query_row("SELECT max(version) FROM operator_instructions", [], |r| {
                r.get(0)
            })
            .expect("newest announcement");
        // Versions were used and lost, as a failed save could leave them.
        conn.execute(
            "UPDATE sqlite_sequence SET seq = ?1 WHERE name = 'operator_instructions'",
            [version + 50],
        )
        .expect("a counter past the largest row");
        (newest, version + 50)
    };
    let db = kashkeshet::machine::database::open(&path).expect("open the 0.10.0 database");
    let (id, saved) = testing::write(&db, move |w| {
        let id = w.insert_message(
            &SwarmId::main(),
            &NewMessageRecord {
                channel_id: "ch_general".into(),
                agent: "amber-falcon".into(),
                body: "after the upgrade".into(),
                created_at: "2026-11-09T00:00:00Z".into(),
                run_id: 3,
            },
        )?;
        let saved = w.append_server_announcement("Be brief.", "2026-11-09T00:00:00Z", 3, None)?;
        Ok::<_, StoreError>((id, saved.version))
    })
    .await
    .expect("a message and an announcement");
    assert!(id > newest, "message {id} after message {newest}");
    assert_eq!(saved, announcement + 1);
}

/// How long adopting takes on a database with a million messages. Run it
/// with `cargo test --release --test migrations -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "slow: measures adopting a million messages; run by hand"]
#[expect(
    clippy::disallowed_methods,
    reason = "measures real time taken, which is what this test reports"
)]
async fn adopting_a_million_messages() {
    let dir = temp_dir("v0_10_0-million");
    let path = dir.join("v0_10_0.db");
    make(&path);
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000000)
             INSERT INTO messages (channel_id, agent, body, created_at, run_id)
             SELECT 'ch_general', 'amber-falcon',
                    'message ' || i || ': the parser and the long-poll loop, again and again', '2026-11-01T00:00:00Z', 3
             FROM n;",
        )
        .expect("a million messages");
    }
    let started = std::time::Instant::now();
    drop(kashkeshet::machine::database::open(&path).expect("adopt"));
    println!("adopting a million messages took {:?}", started.elapsed());
}

/// Every row of every table, as text, with only the columns given for each
/// table: what a migration must keep.
fn dump(conn: &Connection, columns: &[(String, Vec<String>)]) -> Vec<String> {
    let mut rows = Vec::new();
    for (table, cols) in columns {
        let list = cols
            .iter()
            .map(|c| format!("quote(\"{c}\")"))
            .collect::<Vec<_>>()
            .join(" || ',' || ");
        let mut stmt = conn
            .prepare(&format!("SELECT {list} FROM \"{table}\" ORDER BY 1"))
            .expect("dump a table");
        let found = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .expect("rows")
            .map(|r| format!("{table}: {}", r.expect("row")));
        rows.extend(found);
    }
    rows
}

/// Every table's name and columns, the full-text index's own tables
/// included.
fn tables(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table'
             AND name NOT IN ('messages_fts', 'sqlite_sequence') ORDER BY name",
        )
        .expect("list tables");
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .expect("tables")
        .map(|r| r.expect("table"))
        .collect();
    names
        .into_iter()
        .map(|table| {
            let mut stmt = conn
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .expect("table info");
            let cols = stmt
                .query_map([], |r| r.get(0))
                .expect("columns")
                .map(|r| r.expect("column"))
                .collect();
            (table, cols)
        })
        .collect()
}

fn strings(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

#[tokio::test]
async fn a_v0_10_0_database_gains_one_swarm_holding_every_avatar_and_channel() {
    let dir = temp_dir("v0_10_0-swarms");
    let path = dir.join("v0_10_0.db");
    make(&path);
    // Every row the baseline keeps: the full-text index is made again
    // from the messages, and the two handoff-note columns are left behind.
    let (before_tables, before) = {
        let conn = Connection::open(&path).expect("open");
        let t: Vec<(String, Vec<String>)> = tables(&conn)
            .into_iter()
            .filter(|(name, _)| !name.starts_with("messages_fts"))
            .map(|(name, cols)| {
                let cols = cols
                    .into_iter()
                    .filter(|c| !c.starts_with("handoff_note"))
                    .collect();
                (name, cols)
            })
            .collect();
        let rows = dump(&conn, &t);
        (t, rows)
    };

    let db = kashkeshet::machine::database::open(&path).expect("open the 0.10.0 database");
    drop(db);

    let conn = Connection::open(&path).expect("reopen");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("version");
    assert_eq!(version, kkt_db_sqlite::MANAGED);
    assert_eq!(
        strings(&conn, "SELECT name || ' ' || version FROM kkt_db_versions"),
        ["core 2"]
    );
    // Every row of every table is kept, column for column.
    assert_eq!(dump(&conn, &before_tables), before);
    assert_eq!(
        strings(
            &conn,
            "SELECT id || ' ' || created_at || ' ' || run_id FROM swarms"
        ),
        ["sw_main 1970-01-01T00:00:00Z 0"]
    );
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT swarm FROM agents UNION ALL SELECT DISTINCT swarm FROM channels"
        ),
        ["sw_main", "sw_main"]
    );
    let broken: Vec<String> = strings(&conn, "SELECT \"table\" FROM pragma_foreign_key_check");
    assert!(broken.is_empty(), "foreign keys broken in {broken:?}");

    // No avatar or channel moves to another swarm, and no swarm is deleted.
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign keys");
    conn.execute(
        "INSERT INTO swarms (id, created_at, run_id) VALUES ('sw_other', '2026-11-08T00:00:00Z', 3)",
        [],
    )
    .expect("a second swarm");
    for (sql, refusal) in [
        (
            "UPDATE agents SET swarm = 'sw_other' WHERE name = 'amber-falcon'",
            "kkt-db: Fixed agents.swarm",
        ),
        (
            "UPDATE channels SET swarm = 'sw_other' WHERE id = 'ch_general'",
            "kkt-db: Fixed channels.swarm",
        ),
        (
            "DELETE FROM swarms WHERE id = 'sw_other'",
            "kkt-db: Immutable swarms",
        ),
        (
            "UPDATE swarms SET id = 'sw_moved' WHERE id = 'sw_other'",
            "kkt-db: Immutable swarms",
        ),
    ] {
        let err = conn.execute(sql, []).expect_err(sql).to_string();
        assert!(err.contains(refusal), "{sql}: {err}");
    }
    // Setting the swarm it already has is no move.
    conn.execute("UPDATE agents SET swarm = 'sw_main'", [])
        .expect("an unchanged swarm");
    // An avatar and a channel need a swarm that exists.
    for sql in [
        "INSERT INTO agents (name, token_hash, created_at, swarm) VALUES ('lost-one', x'99', 't', 'sw_none')",
        "INSERT INTO channels (id, name, is_open, created_by, created_at, swarm)
             VALUES ('ch_lost', 'lost', 1, 'operator', 't', 'sw_none')",
    ] {
        let err = conn.execute(sql, []).expect_err(sql).to_string();
        assert!(
            err.contains("FOREIGN KEY constraint failed"),
            "{sql}: {err}"
        );
    }
}

#[tokio::test]
async fn a_new_database_has_the_one_swarm_and_new_rows_join_it() {
    use kkt_rules::testing::{NewAgentRecord, NewChannelRecord};
    let dir = temp_dir("new-swarm");
    let path = dir.join("new.db");
    let db = kashkeshet::machine::database::open(&path).expect("a new database");
    testing::write(&db, |s| {
        s.insert_agent(
            &kkt_api_agent::SwarmId::main(),
            &NewAgentRecord {
                name: "amber-falcon".into(),
                token_hash: vec![1],
                recovery_hash: None,
                created_at: "2026-11-08T00:00:00Z".into(),
                last_seen: None,
                run_id: s.insert_server_run(&kkt_rules::testing::NewServerRunRecord {
                    started_at: "2026-11-08T00:00:00Z".into(),
                    version: "test".into(),
                    git_commit: "test".into(),
                    settings: "test".into(),
                    instructions_sha256: "test".into(),
                })?,
            },
        )?;
        s.insert_channel(
            &kkt_api_agent::SwarmId::main(),
            &NewChannelRecord {
                id: "ch_new".into(),
                name: "new".into(),
                purpose: None,
                open: true,
                created_by: "amber-falcon".into(),
                created_at: "2026-11-08T00:00:00Z".into(),
                owner: Some("amber-falcon".into()),
                run_id: 1,
            },
        )
    })
    .await
    .expect("new rows");
    drop(db);
    let conn = Connection::open(&path).expect("reopen");
    assert_eq!(strings(&conn, "SELECT id FROM swarms"), ["sw_main"]);
    assert_eq!(
        strings(
            &conn,
            "SELECT swarm FROM agents UNION ALL SELECT swarm FROM channels"
        ),
        ["sw_main", "sw_main"]
    );
}

/// A 0.10.0 database with something kkt does not declare, a table or a
/// column, or a reference to no row, is refused, unchanged: adopting keeps
/// nothing it cannot account for.
#[tokio::test]
async fn a_v0_10_0_database_with_something_unknown_is_refused_unchanged() {
    for (test, extra, refusal) in [
        (
            "v0_10_0-extra-table",
            "CREATE TABLE notes_of_mine (text TEXT)",
            "a table notes_of_mine that kkt does not declare",
        ),
        (
            "v0_10_0-extra-column",
            "ALTER TABLE channels ADD COLUMN colour TEXT",
            "channels has a column colour that kkt does not declare",
        ),
        (
            "v0_10_0-dangling",
            "PRAGMA foreign_keys = OFF;
             INSERT INTO memberships (channel_id, agent, joined_at)
                 VALUES ('ch_nope', 'amber-falcon', 't');",
            "of memberships refers to a row of channels that is not there; it was not changed",
        ),
    ] {
        let dir = temp_dir(test);
        let path = dir.join("v0_10_0.db");
        make(&path);
        Connection::open(&path)
            .expect("open")
            .execute_batch(extra)
            .expect("something kkt does not declare");
        let before = std::fs::read(&path).expect("read the file");
        let err = match kashkeshet::machine::database::open(&path) {
            Ok(_) => panic!("{test}: adopted"),
            Err(e) => e,
        };
        assert!(err.contains(refusal), "{test}: {err}");
        assert!(
            std::fs::read(&path).expect("read the file") == before,
            "{test}: the refused file changed"
        );
    }
}
