//! Opening a database made by an older version brings it up to date and
//! keeps its rows.

use kkt_scratch as scratch;

use kkt_store::Db;
use rusqlite::{Connection, OptionalExtension};

const V1: &str = include_str!("../crates/kkt-store/migrations/0001_init.sql");

/// A fresh, empty directory for one test's database file, removed when
/// dropped.
fn temp_dir(test: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-migrations-{test}"))
}

/// A database at version 1 with one agent, channel, and message.
fn make_v1(path: &std::path::Path) {
    let conn = Connection::open(path).expect("open v1");
    conn.execute_batch(V1).expect("v1 schema");
    conn.pragma_update(None, "user_version", 1)
        .expect("version");
    conn.execute_batch(
        "INSERT INTO agents (name, token_hash, created_at) VALUES ('amber-falcon', x'00', 't');
         INSERT INTO channels (id, name, is_open, created_by, created_at)
             VALUES ('ch_1', 'general', 1, 'amber-falcon', 't');
         INSERT INTO messages (channel_id, agent, body, created_at)
             VALUES ('ch_1', 'amber-falcon', 'hello from v1', 't');",
    )
    .expect("v1 rows");
}

#[tokio::test]
async fn v050_agents_gain_unknown_clients_and_keep_identity_and_membership() {
    let dir = temp_dir("v050-clients");
    let path = dir.join("old.db");
    make_v1(&path);
    {
        let conn = Connection::open(&path).unwrap();
        // Match the upgrader while rebuilding the released schema's tables.
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        for migration in [
            include_str!("../crates/kkt-store/migrations/0002_server_runs.sql"),
            include_str!("../crates/kkt-store/migrations/0003_revoked_agents.sql"),
            include_str!("../crates/kkt-store/migrations/0004_identity_recovery.sql"),
            include_str!("../crates/kkt-store/migrations/0005_messages_by_time.sql"),
            include_str!("../crates/kkt-store/migrations/0006_agent_descriptions.sql"),
            include_str!("../crates/kkt-store/migrations/0007_channel_notes.sql"),
            include_str!("../crates/kkt-store/migrations/0008_operator_messages.sql"),
            include_str!("../crates/kkt-store/migrations/0009_handoff_notes.sql"),
            include_str!("../crates/kkt-store/migrations/0010_hidden_entries.sql"),
            include_str!("../crates/kkt-store/migrations/0011_operator_instructions.sql"),
            include_str!("../crates/kkt-store/migrations/0012_operator_channels.sql"),
        ] {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 12).unwrap();
        conn.execute_batch("UPDATE agents SET recovery_hash=x'1234', description='worker: #152'; INSERT INTO memberships VALUES ('ch_1', 'amber-falcon', 't')").unwrap();
    }
    let db = Db::open(&path).unwrap();
    let old = db
        .read(|s| s.agent_by_token_hash(&[0]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.name, "amber-falcon");
    assert_eq!(old.client, None);
    assert_eq!(old.description.as_deref(), Some("worker: #152"));
    assert!(
        db.read(|s| s.agent_by_recovery_hash("amber-falcon", &[0x12, 0x34]))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.read(|s| s.is_member("ch_1", "amber-falcon"))
            .await
            .unwrap()
    );
    assert_eq!(
        db.read(|s| s.message(1)).await.unwrap().unwrap().body,
        "hello from v1"
    );
    db.write(|s| {
        let run_id = s.insert_server_run(&kkt_store::NewServerRunRecord {
            started_at: "now".into(),
            version: "test".into(),
            git_commit: "test".into(),
            settings: "test".into(),
            instructions_sha256: "test".into(),
        })?;
        s.set_agent_client(
            "amber-falcon",
            Some(&kkt_store::AgentClient {
                name: "Codex".into(),
                version: "1.2.3".into(),
                recorded_at: "now".into(),
                run_id,
            }),
        )
    })
    .await
    .unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    let client = db
        .read(|s| s.agent("amber-falcon"))
        .await
        .unwrap()
        .unwrap()
        .client
        .unwrap();
    assert_eq!(
        (
            client.name.as_str(),
            client.version.as_str(),
            client.recorded_at.as_str()
        ),
        ("Codex", "1.2.3", "now")
    );
    assert!(client.run_id > 0);
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.query_row(
        &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?"),
        [column],
        |_| Ok(()),
    )
    .optional()
    .expect("table info")
    .is_some()
}

#[test]
fn a_v1_database_gains_server_runs_and_keeps_its_rows() {
    let dir = temp_dir("v1");
    let path = dir.join("old.db");
    make_v1(&path);

    drop(Db::open(&path).expect("open and migrate"));

    let conn = Connection::open(&path).expect("reopen");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("version");
    assert_eq!(version, 19);
    let runs: i64 = conn
        .query_row("SELECT count(*) FROM server_runs", [], |r| r.get(0))
        .expect("server_runs exists");
    assert_eq!(runs, 0);
    assert!(column_exists(&conn, "agents", "run_id"));
    assert!(column_exists(&conn, "messages", "run_id"));
    assert!(column_exists(&conn, "agents", "revoked_at"));
    assert!(column_exists(&conn, "agents", "recovery_hash"));
    let recovery_hash: Option<Vec<u8>> = conn
        .query_row(
            "SELECT recovery_hash FROM agents WHERE name = 'amber-falcon'",
            [],
            |r| r.get(0),
        )
        .expect("old agent kept without recovery credentials");
    assert_eq!(recovery_hash, None);

    let agent_run: Option<i64> = conn
        .query_row(
            "SELECT run_id FROM agents WHERE name = 'amber-falcon'",
            [],
            |r| r.get(0),
        )
        .expect("old agent kept");
    assert_eq!(agent_run, None);
    let revoked: Option<String> = conn
        .query_row(
            "SELECT revoked_at FROM agents WHERE name = 'amber-falcon'",
            [],
            |r| r.get(0),
        )
        .expect("old agent kept");
    assert_eq!(revoked, None, "existing agents stay in use");
    let description: Option<String> = conn
        .query_row(
            "SELECT description FROM agents WHERE name = 'amber-falcon'",
            [],
            |r| r.get(0),
        )
        .expect("old agent kept with no description");
    assert_eq!(description, None);
    let (note, note_set_at): (String, Option<String>) = conn
        .query_row(
            "SELECT handoff_note, handoff_note_set_at FROM agents WHERE name = 'amber-falcon'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("old agent kept with an empty handoff note");
    assert_eq!((note.as_str(), note_set_at), ("", None));
    let (body, message_run): (String, Option<i64>) = conn
        .query_row("SELECT body, run_id FROM messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("old message kept");
    assert_eq!(body, "hello from v1");
    assert_eq!(message_run, None);

    // Search still finds the old message: the full-text index survived.
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'hello'",
            [],
            |r| r.get(0),
        )
        .expect("search");
    assert_eq!(hits, 1);
}

#[test]
fn a_new_row_can_name_its_run() {
    let dir = temp_dir("new-row");
    let path = dir.join("new.db");
    drop(Db::open(&path).expect("create"));

    let conn = Connection::open(&path).expect("reopen");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign keys");
    conn.execute_batch(
        "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
             VALUES ('t', '0.1.0', 'unknown', 'max_limit=100', 'ab');
         INSERT INTO agents (name, token_hash, created_at, run_id)
             VALUES ('amber-falcon', x'00', 't', 1);",
    )
    .expect("insert with run");
    let bad = conn.execute(
        "INSERT INTO agents (name, token_hash, created_at, run_id) VALUES ('b', x'01', 't', 99)",
        [],
    );
    assert!(bad.is_err(), "a run_id must name an existing run");
}

#[test]
fn messages_may_come_from_the_operator_but_no_unknown_agent() {
    let dir = temp_dir("operator");
    let path = dir.join("old.db");
    make_v1(&path);

    drop(Db::open(&path).expect("open and migrate"));

    let conn = Connection::open(&path).expect("reopen");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign keys");
    // The old message kept its ID, and search still finds it.
    let (id, agent): (i64, String) = conn
        .query_row(
            "SELECT m.id, m.agent FROM messages_fts f JOIN messages m ON m.id = f.rowid
             WHERE messages_fts MATCH 'hello'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("old message found");
    assert_eq!((id, agent.as_str()), (1, "amber-falcon"));
    conn.execute(
        "INSERT INTO messages (channel_id, agent, body, created_at)
         VALUES ('ch_1', 'operator', 'The operator changed the purpose', 't')",
        [],
    )
    .expect("an operator message");
    let unknown = conn.execute(
        "INSERT INTO messages (channel_id, agent, body, created_at)
         VALUES ('ch_1', 'nobody-here', 'hi', 't')",
        [],
    );
    assert!(unknown.is_err(), "a message's agent must exist");
    let bad_channel = conn.execute(
        "INSERT INTO messages (channel_id, agent, body, created_at)
         VALUES ('ch_none', 'operator', 'hi', 't')",
        [],
    );
    assert!(bad_channel.is_err(), "a message's channel must exist");
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'purpose'",
            [],
            |r| r.get(0),
        )
        .expect("search");
    assert_eq!(hits, 1, "new messages are indexed");
}

#[test]
fn an_older_database_gains_the_message_time_index() {
    let dir = temp_dir("time-index");
    let path = dir.join("old.db");
    make_v1(&path);

    drop(Db::open(&path).expect("open and migrate"));

    let conn = Connection::open(&path).expect("reopen");
    let index: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'messages_by_time'",
            [],
            |r| r.get(0),
        )
        .optional()
        .expect("schema");
    assert_eq!(index.as_deref(), Some("messages_by_time"));
    // The query the dashboard runs uses it.
    let plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT channel_id, created_at FROM messages WHERE created_at >= 'a'",
            [],
            |r| r.get(3),
        )
        .expect("plan");
    assert!(plan.contains("messages_by_time"), "{plan}");
    let kept: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .expect("count");
    assert_eq!(kept, 1);
}

#[tokio::test]
async fn existing_channels_gain_an_empty_note_and_edits_survive_reopening() {
    let dir = temp_dir("channel-notes");
    let path = dir.join("old.db");
    make_v1(&path);
    let db = Db::open(&path).unwrap();
    let note = db
        .read(|s| s.channel_summary("ch_1", None, None))
        .await
        .unwrap()
        .unwrap()
        .note;
    assert_eq!(note.text, "");
    assert_eq!(note.edited_by, None);
    assert_eq!(note.edited_at, None);
    db.write(|s| s.set_note("ch_1", "a note\n😀", "amber-falcon", "2026-10-26T12:00:00Z"))
        .await
        .unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    let note = db
        .read(|s| s.channel_summary("ch_1", None, None))
        .await
        .unwrap()
        .unwrap()
        .note;
    assert_eq!(note.text, "a note\n😀");
    assert_eq!(note.edited_by.as_deref(), Some("amber-falcon"));
    assert_eq!(note.edited_at.as_deref(), Some("2026-10-26T12:00:00Z"));
    drop(db);
}

#[tokio::test]
async fn hidden_entries_migrate_and_stay_hidden_after_reopening_without_losing_messages() {
    let dir = temp_dir("hidden-entries");
    let path = dir.join("old.db");
    make_v1(&path);
    let db = Db::open(&path).unwrap();
    assert!(
        db.read(|s| s.agent("amber-falcon"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(db.read(|s| s.channel("ch_1")).await.unwrap().is_some());
    db.write(|s| {
        s.add_member("ch_1", "amber-falcon", "t")?;
        s.hide_agent("amber-falcon", "later")?;
        s.hide_channel("ch_1")
    })
    .await
    .unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    assert!(
        db.read(|s| s.agent("amber-falcon"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.read(|s| s.agent_name_taken("amber-falcon"))
            .await
            .unwrap()
    );
    assert!(db.read(|s| s.channel("ch_1")).await.unwrap().is_none());
    assert!(
        db.read(|s| s.all_channel_ids(None))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db.read(|s| s.agents_by_last_seen())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.read(|s| s.member_count("ch_1")).await.unwrap(), 0);
    assert_eq!(
        db.read(|s| s.message(1)).await.unwrap().unwrap().body,
        "hello from v1"
    );
    assert_eq!(db.read(|s| s.active_agent_count()).await.unwrap(), 0);
    let conn = Connection::open(&path).unwrap();
    let revoked: Option<String> = conn
        .query_row(
            "SELECT revoked_at FROM agents WHERE name = 'amber-falcon'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revoked.as_deref(), Some("later"));
    drop(conn);
    drop(db);
}

#[tokio::test]
async fn instruction_versions_survive_reopen_and_reject_mutation() {
    let dir = temp_dir("instructions");
    let path = dir.join("old.db");
    make_v1(&path);
    let db = Db::open(&path).unwrap();
    assert!(
        db.read(|s| s.current_server_announcement())
            .await
            .unwrap()
            .is_none()
    );
    let saved = db
        .write(|s| {
            let run_id = s.insert_server_run(&kkt_store::NewServerRunRecord {
                started_at: "2026-10-27T00:00:00Z".into(),
                version: "test".into(),
                git_commit: "test".into(),
                settings: "test".into(),
                instructions_sha256: "test".into(),
            })?;
            s.append_server_announcement("first\n😀", "2026-10-27T00:00:01Z", run_id, None)
        })
        .await
        .unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.read(|s| s.server_announcement_history()).await.unwrap(),
        vec![saved]
    );
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert!(
        conn.execute("UPDATE operator_instructions SET text = 'changed'", [])
            .is_err()
    );
    assert!(conn.execute("INSERT OR REPLACE INTO operator_instructions (version, text, saved_at, run_id) SELECT version, 'replaced', saved_at, run_id FROM operator_instructions", []).is_err());
    assert!(
        conn.execute("DELETE FROM operator_instructions", [])
            .is_err()
    );
    assert_eq!(
        conn.query_row("SELECT body FROM messages WHERE id = 1", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "hello from v1"
    );
    drop(conn);
}

/// The migrations v0.5.0 shipped with.
const V0_5_0: [&str; 12] = [
    V1,
    include_str!("../crates/kkt-store/migrations/0002_server_runs.sql"),
    include_str!("../crates/kkt-store/migrations/0003_revoked_agents.sql"),
    include_str!("../crates/kkt-store/migrations/0004_identity_recovery.sql"),
    include_str!("../crates/kkt-store/migrations/0005_messages_by_time.sql"),
    include_str!("../crates/kkt-store/migrations/0006_agent_descriptions.sql"),
    include_str!("../crates/kkt-store/migrations/0007_channel_notes.sql"),
    include_str!("../crates/kkt-store/migrations/0008_operator_messages.sql"),
    include_str!("../crates/kkt-store/migrations/0009_handoff_notes.sql"),
    include_str!("../crates/kkt-store/migrations/0010_hidden_entries.sql"),
    include_str!("../crates/kkt-store/migrations/0011_operator_instructions.sql"),
    include_str!("../crates/kkt-store/migrations/0012_operator_channels.sql"),
];

#[tokio::test]
async fn a_v0_5_0_database_gives_each_channel_its_creator_as_owner_unless_it_is_gone() {
    let dir = temp_dir("channel-owners");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        for migration in V0_5_0 {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 12).unwrap();
        conn.execute_batch(
            "INSERT INTO agents (name, token_hash, created_at) VALUES ('amber-falcon', x'00', 't');
             INSERT INTO agents (name, token_hash, created_at, hidden, revoked_at)
                 VALUES ('gone-heron', x'01', 't', 1, 't');
             INSERT INTO channels (id, name, is_open, created_by, created_at)
                 VALUES ('ch_kept', 'kept', 0, 'amber-falcon', 't'),
                        ('ch_gone', 'gone', 1, 'gone-heron', 't'),
                        ('ch_op', 'help', 0, 'operator', 't');
             INSERT INTO memberships VALUES ('ch_kept', 'amber-falcon', 't');
             INSERT INTO messages (channel_id, agent, body, created_at)
                 VALUES ('ch_kept', 'amber-falcon', 'hello from v0.5.0', 't');",
        )
        .unwrap();
    }
    let db = Db::open(&path).unwrap();
    let owner = |id: &'static str| {
        let db = db.clone();
        async move {
            db.read(move |s| s.channel(id))
                .await
                .unwrap()
                .unwrap()
                .owner
        }
    };
    assert_eq!(owner("ch_kept").await.as_deref(), Some("amber-falcon"));
    assert_eq!(owner("ch_gone").await, None);
    assert_eq!(owner("ch_op").await, None);
    let summary = db
        .read(|s| s.channel_summary("ch_kept", None, None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.owner.as_deref(), Some("amber-falcon"));
    assert_eq!(summary.created_by, "amber-falcon");
    assert_eq!(summary.member_count, 1);
    assert_eq!(
        db.read(|s| s.message(1)).await.unwrap().unwrap().body,
        "hello from v0.5.0"
    );
    drop(db);
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert!(
        !conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    assert!(
        conn.execute_batch("UPDATE channels SET owner = 'absent' WHERE id = 'ch_op'")
            .is_err(),
        "an owner must be an agent"
    );
}

#[tokio::test]
async fn a_v0_5_0_databases_instructions_are_the_humans_and_new_ones_name_their_admin() {
    let dir = temp_dir("instruction-authors");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        for migration in V0_5_0 {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 12).unwrap();
        conn.execute_batch(
            "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
                 VALUES ('t', '0.5.0', 'c', 's', 'h');
             INSERT INTO operator_instructions (text, saved_at, run_id)
                 VALUES ('Be kind.', '2026-10-01T00:00:00Z', 1), ('', '2026-10-02T00:00:00Z', 1);",
        )
        .unwrap();
    }
    let db = Db::open(&path).unwrap();
    let history = db.read(|s| s.server_announcement_history()).await.unwrap();
    let kept: Vec<_> = history
        .iter()
        .map(|v| {
            (
                v.version,
                v.text.as_str(),
                v.saved_at.as_str(),
                v.run_id,
                v.saved_by_admin.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        kept,
        [
            (2, "", "2026-10-02T00:00:00Z", 1, None),
            (1, "Be kind.", "2026-10-01T00:00:00Z", 1, None),
        ]
    );
    let saved = db
        .write(|s| {
            s.append_server_announcement(
                "Be brief.",
                "2026-10-30T00:00:00Z",
                1,
                Some("admin-sentinel"),
            )
        })
        .await
        .unwrap();
    assert_eq!(saved.version, 3);
    drop(db);
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.read(|s| s.current_server_announcement()).await.unwrap(),
        Some(saved)
    );
    drop(db);
    // Versions stay immutable, the new column included.
    let conn = Connection::open(&path).unwrap();
    assert!(
        conn.execute("UPDATE operator_instructions SET saved_by_admin = NULL", [])
            .is_err()
    );
}

/// The migrations before lanes, as the release before them shipped.
const BEFORE_LANES: [&str; 15] = [
    V1,
    include_str!("../crates/kkt-store/migrations/0002_server_runs.sql"),
    include_str!("../crates/kkt-store/migrations/0003_revoked_agents.sql"),
    include_str!("../crates/kkt-store/migrations/0004_identity_recovery.sql"),
    include_str!("../crates/kkt-store/migrations/0005_messages_by_time.sql"),
    include_str!("../crates/kkt-store/migrations/0006_agent_descriptions.sql"),
    include_str!("../crates/kkt-store/migrations/0007_channel_notes.sql"),
    include_str!("../crates/kkt-store/migrations/0008_operator_messages.sql"),
    include_str!("../crates/kkt-store/migrations/0009_handoff_notes.sql"),
    include_str!("../crates/kkt-store/migrations/0010_hidden_entries.sql"),
    include_str!("../crates/kkt-store/migrations/0011_operator_instructions.sql"),
    include_str!("../crates/kkt-store/migrations/0012_operator_channels.sql"),
    include_str!("../crates/kkt-store/migrations/0013_channel_owners.sql"),
    include_str!("../crates/kkt-store/migrations/0014_agent_clients.sql"),
    include_str!("../crates/kkt-store/migrations/0015_instruction_authors.sql"),
];

#[tokio::test]
async fn a_database_from_before_lanes_keeps_its_rows_and_gains_lanes_that_stay_versioned() {
    use kkt_store::{LaneRetiredRecord, NewLaneVersionRecord};
    let dir = temp_dir("lanes");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        for migration in BEFORE_LANES {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 15).unwrap();
        conn.execute_batch(
            "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
                 VALUES ('t', '0.7.0', 'c', 's', 'h');
             INSERT INTO agents (name, token_hash, created_at) VALUES ('amber-falcon', x'00', 't');
             INSERT INTO channels (id, name, is_open, created_by, created_at, owner)
                 VALUES ('ch_1', 'general', 1, 'amber-falcon', 't', 'amber-falcon');
             INSERT INTO memberships VALUES ('ch_1', 'amber-falcon', 't');
             INSERT INTO messages (channel_id, agent, body, created_at)
                 VALUES ('ch_1', 'amber-falcon', 'hello from 0.7.0', 't');
             INSERT INTO operator_instructions (text, saved_at, run_id, saved_by_admin)
                 VALUES ('Be kind.', 't', 1, 'admin-sentinel');",
        )
        .unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert!(db.read(|s| s.lanes()).await.unwrap().is_empty());
    assert_eq!(
        db.read(|s| s.message(1)).await.unwrap().unwrap().body,
        "hello from 0.7.0"
    );
    assert!(
        db.read(|s| s.is_member("ch_1", "amber-falcon"))
            .await
            .unwrap()
    );
    let announcement = db
        .read(|s| s.current_server_announcement())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            announcement.text.as_str(),
            announcement.saved_by_admin.as_deref()
        ),
        ("Be kind.", Some("admin-sentinel"))
    );

    let version = |text: &str, by: Option<&str>| NewLaneVersionRecord {
        lane: "reviewer".into(),
        description: "Reviews pull requests.".into(),
        text: text.into(),
        saved_at: "2026-11-01T00:00:00Z".into(),
        run_id: 1,
        saved_by_admin: by.map(str::to_string),
        source: None,
    };
    let (first, second) = (
        version("Read the diff.", None),
        version("Read it twice.", Some("admin-sentinel")),
    );
    db.write(move |s| {
        s.insert_lane("reviewer", "2026-11-01T00:00:00Z", 1)?;
        s.append_lane_version(&first)?;
        s.append_lane_version(&second)?;
        s.retire_lane(
            "reviewer",
            &LaneRetiredRecord {
                at: "2026-11-02T00:00:00Z".into(),
                run_id: 1,
                by_admin: None,
            },
        )
    })
    .await
    .unwrap();
    drop(db);

    let db = Db::open(&path).unwrap();
    let lane = db.read(|s| s.lane("reviewer")).await.unwrap().unwrap();
    assert_eq!(lane.current.version, 2);
    assert_eq!(lane.current.text, "Read it twice.");
    assert_eq!(
        lane.current.saved_by_admin.as_deref(),
        Some("admin-sentinel")
    );
    assert_eq!(
        lane.retired.as_ref().map(|r| r.at.as_str()),
        Some("2026-11-02T00:00:00Z")
    );
    let history = db.read(|s| s.lane_history("reviewer")).await.unwrap();
    assert_eq!(
        history
            .iter()
            .map(|v| (v.version, v.text.as_str(), v.saved_by_admin.as_deref()))
            .collect::<Vec<_>>(),
        [
            (2, "Read it twice.", Some("admin-sentinel")),
            (1, "Read the diff.", None)
        ]
    );
    drop(db);

    // Versions are immutable and lanes are never deleted.
    let conn = Connection::open(&path).unwrap();
    assert!(
        conn.execute("UPDATE lane_versions SET text = 'changed'", [])
            .is_err()
    );
    assert!(conn.execute("DELETE FROM lane_versions", []).is_err());
    assert!(
        conn.execute(
            "INSERT OR REPLACE INTO lane_versions (lane, version, description, text, saved_at, run_id)
             VALUES ('reviewer', 1, 'd', 'replaced', 't', 1)",
            []
        )
        .is_err()
    );
    assert!(conn.execute("DELETE FROM lanes", []).is_err());
}

/// A database from the release with stored lanes but none taken: its agents
/// hold no lane, keep everything else, and can then take one that survives
/// reopening.
#[tokio::test]
async fn agents_from_before_lanes_could_be_taken_hold_none_and_can_take_one() {
    use kkt_store::TakenLaneRecord;
    let dir = temp_dir("agent-lanes");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        for migration in BEFORE_LANES {
            conn.execute_batch(migration).unwrap();
        }
        conn.execute_batch(include_str!(
            "../crates/kkt-store/migrations/0016_lanes.sql"
        ))
        .unwrap();
        conn.pragma_update(None, "user_version", 16).unwrap();
        conn.execute_batch(
            "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
                 VALUES ('t', '0.7.0', 'c', 's', 'h');
             INSERT INTO agents (name, token_hash, created_at, description)
                 VALUES ('amber-falcon', x'00', 't', 'reviews');
             INSERT INTO lanes (name, created_at, run_id) VALUES ('reviewer', 't', 1);
             INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id)
                 VALUES ('reviewer', 1, 'Reviews.', 'Read.', 't', 1);
             INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id)
                 VALUES ('reviewer', 2, 'Reviews.', 'Read twice.', 't', 1);",
        )
        .unwrap();
    }
    let db = Db::open(&path).unwrap();
    let agent = db.read(|s| s.agent("amber-falcon")).await.unwrap().unwrap();
    assert_eq!(agent.lane, None);
    assert_eq!(agent.description.as_deref(), Some("reviews"));
    assert_eq!(
        db.read(|s| s.lane("reviewer"))
            .await
            .unwrap()
            .unwrap()
            .current
            .version,
        2
    );
    db.write(|s| {
        s.set_agent_lane(
            "amber-falcon",
            Some(&TakenLaneRecord {
                lane: "reviewer".into(),
                version: 1,
                taken_at: "2026-11-01T00:00:00Z".into(),
                run_id: 1,
            }),
        )
    })
    .await
    .unwrap();
    drop(db);

    let db = Db::open(&path).unwrap();
    let lane = db
        .read(|s| s.agent("amber-falcon"))
        .await
        .unwrap()
        .unwrap()
        .lane
        .expect("lane");
    assert_eq!(
        (
            lane.taken.lane.as_str(),
            lane.taken.version,
            lane.current_version
        ),
        ("reviewer", 1, 2)
    );
    let holding = db
        .read(|s| s.agents(None, Some("reviewer"), None, 10))
        .await
        .unwrap();
    assert_eq!(holding.len(), 1);
    assert!(
        db.read(|s| s.agents(None, Some("writer"), None, 10))
            .await
            .unwrap()
            .is_empty()
    );
}

/// A database from the release with lanes that agents take, before imports:
/// its lane versions keep their rows with no source, an imported version
/// keeps its folder and commit, and versions stay immutable.
#[tokio::test]
async fn lane_versions_from_before_imports_have_no_source_and_an_import_keeps_its_own() {
    use kkt_store::{LaneSourceRecord, NewLaneVersionRecord};
    let dir = temp_dir("lane-sources");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        for migration in BEFORE_LANES {
            conn.execute_batch(migration).unwrap();
        }
        conn.execute_batch(include_str!(
            "../crates/kkt-store/migrations/0016_lanes.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!(
            "../crates/kkt-store/migrations/0017_agent_lanes.sql"
        ))
        .unwrap();
        conn.pragma_update(None, "user_version", 17).unwrap();
        conn.execute_batch(
            "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
                 VALUES ('t', '0.8.0', 'c', 's', 'h');
             INSERT INTO lanes (name, created_at, run_id) VALUES ('reviewer', 't', 1);
             INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id,
                                        saved_by_admin)
                 VALUES ('reviewer', 1, 'Reviews.', 'Read.', 't', 1, 'admin-sentinel');
             INSERT INTO agents (name, token_hash, created_at, lane, lane_version,
                                 lane_taken_at, lane_run_id)
                 VALUES ('amber-falcon', x'00', 't', 'reviewer', 1, 't', 1);",
        )
        .unwrap();
    }
    let db = Db::open(&path).unwrap();
    let lane = db.read(|s| s.lane("reviewer")).await.unwrap().unwrap();
    assert_eq!(
        (
            lane.current.text.as_str(),
            lane.current.saved_by_admin.as_deref(),
            &lane.current.source
        ),
        ("Read.", Some("admin-sentinel"), &None)
    );
    let agent = db.read(|s| s.agent("amber-falcon")).await.unwrap().unwrap();
    assert_eq!(agent.lane.map(|l| l.taken.version), Some(1));

    let source = LaneSourceRecord {
        folder: "/home/sam/lanes".into(),
        commit: Some("abc123".into()),
    };
    let imported = NewLaneVersionRecord {
        lane: "reviewer".into(),
        description: "Reviews.".into(),
        text: "Read twice.".into(),
        saved_at: "2026-11-01T00:00:00Z".into(),
        run_id: 1,
        saved_by_admin: None,
        source: Some(source.clone()),
    };
    db.write(move |s| s.append_lane_version(&imported))
        .await
        .unwrap();
    drop(db);

    let db = Db::open(&path).unwrap();
    let history = db.read(|s| s.lane_history("reviewer")).await.unwrap();
    assert_eq!(
        history
            .iter()
            .map(|v| (v.version, v.source.clone()))
            .collect::<Vec<_>>(),
        [(2, Some(source)), (1, None)]
    );
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert!(
        conn.execute("UPDATE lane_versions SET source_folder = '/elsewhere'", [])
            .is_err()
    );
}

/// A v0.7.0 database with handoff notes: each note with text becomes a
/// memory entry named `handoff` with its text and time, an empty or cleared
/// note becomes nothing, and the avatars can then keep more entries.
#[tokio::test]
async fn v070_handoff_notes_become_handoff_memory_entries() {
    use kkt_store::MemoryEntryRecord;
    let dir = temp_dir("memory");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        for migration in BEFORE_LANES {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 15).unwrap();
        conn.execute_batch(
            "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
                 VALUES ('t', '0.7.0', 'c', 's', 'h');
             INSERT INTO agents (name, token_hash, created_at, handoff_note, handoff_note_set_at)
                 VALUES ('amber-falcon', x'00', 't', 'Next: the parser.\n«ünïcode»', '2026-10-30T12:00:00Z');
             INSERT INTO agents (name, token_hash, created_at, handoff_note, handoff_note_set_at)
                 VALUES ('brave-otter', x'01', 't', '', '2026-10-30T13:00:00Z');
             INSERT INTO agents (name, token_hash, created_at)
                 VALUES ('calm-heron', x'02', 't');
             INSERT INTO agents (name, token_hash, created_at, handoff_note, hidden)
                 VALUES ('dim-lynx', x'03', '2026-10-01T00:00:00Z', 'gone but kept', 1);",
        )
        .unwrap();
    }
    let db = Db::open(&path).unwrap();
    let entries = db.read(|s| s.memory_entries("amber-falcon")).await.unwrap();
    assert_eq!(
        entries,
        [MemoryEntryRecord {
            name: "handoff".into(),
            text: "Next: the parser.\n«ünïcode»".into(),
            saved_at: "2026-10-30T12:00:00Z".into(),
            run_id: None,
        }]
    );
    for empty in ["brave-otter", "calm-heron"] {
        assert!(
            db.read(move |s| s.memory_entries(empty))
                .await
                .unwrap()
                .is_empty(),
            "{empty}"
        );
    }
    // A note with no time set keeps its text, timed from the avatar's start.
    let hidden = db
        .read(|s| s.memory_entry("dim-lynx", "handoff"))
        .await
        .unwrap()
        .expect("entry");
    assert_eq!(
        (hidden.text.as_str(), hidden.saved_at.as_str()),
        ("gone but kept", "2026-10-01T00:00:00Z")
    );
    // The rest of the avatar is as it was.
    let agent = db.read(|s| s.agent("amber-falcon")).await.unwrap().unwrap();
    assert_eq!(agent.created_at, "t");

    let entry = MemoryEntryRecord {
        name: "rules".into(),
        text: "Ask first.".into(),
        saved_at: "2026-11-01T00:00:00Z".into(),
        run_id: Some(1),
    };
    let saved = entry.clone();
    db.write(move |s| s.save_memory_entry("amber-falcon", &saved))
        .await
        .unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    let names: Vec<_> = db
        .read(|s| s.memory_entries("amber-falcon"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["handoff", "rules"]);
    assert_eq!(
        db.read(|s| s.memory_entry("amber-falcon", "rules"))
            .await
            .unwrap(),
        Some(entry)
    );
}

#[test]
fn operator_channel_migration_preserves_references_and_checks_creators() {
    let dir = temp_dir("operator-channel");
    let path = dir.join("old.db");
    make_v1(&path);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("INSERT INTO memberships VALUES ('ch_1', 'amber-falcon', 't')")
            .unwrap();
        for migration in [
            include_str!("../crates/kkt-store/migrations/0002_server_runs.sql"),
            include_str!("../crates/kkt-store/migrations/0003_revoked_agents.sql"),
            include_str!("../crates/kkt-store/migrations/0004_identity_recovery.sql"),
            include_str!("../crates/kkt-store/migrations/0005_messages_by_time.sql"),
            include_str!("../crates/kkt-store/migrations/0006_agent_descriptions.sql"),
            include_str!("../crates/kkt-store/migrations/0007_channel_notes.sql"),
            include_str!("../crates/kkt-store/migrations/0008_operator_messages.sql"),
            include_str!("../crates/kkt-store/migrations/0009_handoff_notes.sql"),
            include_str!("../crates/kkt-store/migrations/0010_hidden_entries.sql"),
            include_str!("../crates/kkt-store/migrations/0011_operator_instructions.sql"),
        ] {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 11).unwrap();
        conn.execute_batch("UPDATE channels SET note='keep this note', note_edited_by='amber-falcon', note_edited_at='t', hidden=1 WHERE id='ch_1'").unwrap();
    }
    drop(Db::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT body FROM messages WHERE channel_id='ch_1'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "hello from v1"
    );
    assert_eq!(
        conn.query_row(
            "SELECT agent FROM memberships WHERE channel_id='ch_1'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "amber-falcon"
    );
    assert!(
        !conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    let retained: (String, String, String, i64) = conn
        .query_row(
            "SELECT note, note_edited_by, note_edited_at, hidden FROM channels WHERE id='ch_1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        retained,
        (
            "keep this note".into(),
            "amber-falcon".into(),
            "t".into(),
            1
        )
    );
    conn.execute_batch("INSERT INTO channels(id,name,is_open,created_by,created_at) VALUES ('ch_op','help',0,'operator','t')").unwrap();
    assert!(conn.execute_batch("INSERT INTO channels(id,name,is_open,created_by,created_at) VALUES ('ch_bad','bad',1,'absent','t')").is_err());
    assert!(
        conn.execute_batch("UPDATE channels SET created_by='absent' WHERE id='ch_op'")
            .is_err()
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM agents WHERE name='operator'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    drop(Db::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT created_by FROM channels WHERE id='ch_op'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "operator"
    );
}
