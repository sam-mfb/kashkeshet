//! Opening a database: a new one gets the core's tables with their
//! protections; one from 0.10.0 or `next` is adopted (`v0_10_0`); an older
//! one is refused, unchanged.
//!
//! Databases from before 0.10.0 are no longer upgraded directly (Sam,
//! 2026-11-09): their owners upgrade through 0.10 first. The protections
//! the old migrations' tests checked, that the database itself refuses a
//! message from no avatar, a channel made by no avatar, and changes to
//! versions that are kept for good, are checked here on a new database.

use kkt_scratch as scratch;

use rusqlite::Connection;

#[path = "migrations/v0_10_0.rs"]
mod v0_10_0;

const V1: &str = include_str!("migrations/sql/0001_init.sql");

/// A fresh, empty directory for one test's database file, removed when
/// dropped.
fn temp_dir(test: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-migrations-{test}"))
}

/// A new database, opened once by this version, and a raw connection to it
/// with foreign keys on, as kkt's own connections have them.
fn new_database(test: &str) -> (scratch::Dir, Connection) {
    let dir = temp_dir(test);
    let path = dir.join("new.db");
    drop(kashkeshet::machine::database::open(&path).expect("create"));
    let conn = Connection::open(&path).expect("reopen");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign keys");
    conn.execute_batch(
        "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
             VALUES ('t', '0.11.0', 'unknown', 'max_limit=100', 'ab');
         INSERT INTO agents (name, token_hash, created_at, run_id)
             VALUES ('amber-falcon', x'00', 't', 1);
         INSERT INTO channels (id, name, is_open, created_by, created_at)
             VALUES ('ch_1', 'general', 1, 'amber-falcon', 't');",
    )
    .expect("a run, an avatar and a channel");
    (dir, conn)
}

/// Runs `sql` and requires it to be refused with `refusal` in the error.
fn refused(conn: &Connection, sql: &str, refusal: &str) {
    let err = conn.execute(sql, []).expect_err(sql).to_string();
    assert!(err.contains(refusal), "{sql}: {err}");
}

#[test]
fn a_new_row_can_name_its_run() {
    let (_dir, conn) = new_database("new-row");
    refused(
        &conn,
        "INSERT INTO agents (name, token_hash, created_at, run_id) VALUES ('b', x'01', 't', 99)",
        "FOREIGN KEY constraint failed",
    );
}

#[test]
fn messages_may_come_from_the_operator_but_no_unknown_agent() {
    let (_dir, conn) = new_database("operator-messages");
    conn.execute(
        "INSERT INTO messages (channel_id, agent, body, created_at) VALUES ('ch_1', 'operator', 'from the operator', 't')",
        [],
    )
    .expect("a message from the operator");
    refused(
        &conn,
        "INSERT INTO messages (channel_id, agent, body, created_at) VALUES ('ch_1', 'nobody', 'x', 't')",
        "kkt-db: Reference messages.agent",
    );
    // An avatar with messages is not deleted from under them.
    refused(
        &conn,
        "INSERT INTO messages (channel_id, agent, body, created_at) VALUES ('ch_1', 'amber-falcon', 'hi', 't');
         DELETE FROM agents WHERE name = 'amber-falcon'",
        "",
    );
}

#[test]
fn channels_may_be_made_by_the_operator_but_no_unknown_agent() {
    let (_dir, conn) = new_database("operator-channels");
    conn.execute(
        "INSERT INTO channels (id, name, is_open, created_by, created_at) VALUES ('ch_2', 'ops', 1, 'operator', 't')",
        [],
    )
    .expect("a channel made by the operator");
    refused(
        &conn,
        "INSERT INTO channels (id, name, is_open, created_by, created_at) VALUES ('ch_3', 'x', 1, 'nobody', 't')",
        "kkt-db: Reference channels.created_by",
    );
    refused(
        &conn,
        "UPDATE channels SET created_by = 'nobody' WHERE id = 'ch_1'",
        "kkt-db: Reference channels.created_by",
    );
    refused(
        &conn,
        "DELETE FROM agents WHERE name = 'amber-falcon'",
        "kkt-db: Reference channels.created_by",
    );
}

#[test]
fn announcement_versions_and_lane_versions_are_kept_for_good() {
    let (_dir, conn) = new_database("kept-versions");
    conn.execute_batch(
        "INSERT INTO operator_instructions (text, saved_at, run_id) VALUES ('Be kind.', 't', 1);
         INSERT INTO lanes (name, created_at, run_id) VALUES ('reviewer', 't', 1);
         INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id)
             VALUES ('reviewer', 1, 'Reviews', 'Review.', 't', 1);",
    )
    .expect("an announcement and a lane");
    for sql in [
        "UPDATE operator_instructions SET text = 'Be cruel.'",
        "DELETE FROM operator_instructions",
        "UPDATE lane_versions SET text = 'Approve everything.'",
        "DELETE FROM lane_versions",
    ] {
        refused(&conn, sql, "kkt-db: Immutable");
    }
    refused(&conn, "DELETE FROM lanes", "kkt-db: NoDelete lanes");
    refused(
        &conn,
        "INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id)
             VALUES ('reviewer', 1, 'Again', 'Again.', 't', 1)",
        "UNIQUE constraint failed",
    );
    refused(
        &conn,
        "INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id)
             VALUES ('reviewer', 0, 'Zero', 'Zero.', 't', 1)",
        "kkt-db: Positive lane_versions.version",
    );
    // A lane is retired, not deleted: retiring it is a change it allows.
    conn.execute("UPDATE lanes SET retired_at = 't', retired_run_id = 1", [])
        .expect("retire a lane");
}

/// A database from before 0.10.0 is refused, with directions, and left as
/// it was.
#[test]
fn a_database_from_before_0_10_0_is_refused_and_left_alone() {
    let dir = temp_dir("too-old");
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).expect("open v1");
        conn.execute_batch(V1).expect("v1 schema");
        conn.pragma_update(None, "user_version", 1)
            .expect("version");
        conn.execute_batch(
            "INSERT INTO agents (name, token_hash, created_at) VALUES ('amber-falcon', x'00', 't');",
        )
        .expect("v1 rows");
    }
    let before = std::fs::read(&path).expect("read the file");
    let err = match kashkeshet::machine::database::open(&path) {
        Ok(_) => panic!("a database from before 0.10.0 opened"),
        Err(e) => e,
    };
    assert!(
        err.contains("older than 0.10.0") && err.contains("upgrade it with kkt 0.10 first"),
        "{err}"
    );
    assert_eq!(
        kashkeshet::machine::database::pending_upgrade(&path).map(|_| ()),
        Err(format!(
            "could not read {}: database error: this database is from a kkt version older than 0.10.0; upgrade it with kkt 0.10 first, then with this version",
            path.display()
        )),
    );
    assert!(
        std::fs::read(&path).expect("read the file") == before,
        "the refused file changed"
    );
}
