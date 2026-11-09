//! The database file as the program opens it: reopening keeps what was
//! written, and copies taken while a server writes are whole and
//! consistent, and never replace a file.

use kashkeshet::machine::database;
use kkt_scratch as scratch;
use kkt_store::{NewAgentRecord, NewChannelRecord, NewMessageRecord, NewServerRunRecord, SwarmId};

const T: &str = "2026-10-23T14:00:00Z";

/// A fresh, empty directory for one test's files, removed when dropped.
fn scratch_dir(test: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("database-files-{test}"))
}

fn run(version: &str) -> NewServerRunRecord {
    NewServerRunRecord {
        started_at: T.into(),
        version: version.into(),
        git_commit: "abc123".into(),
        settings: "default_limit=20".into(),
        instructions_sha256: "00".into(),
    }
}

#[tokio::test]
async fn file_database_reopens_without_rerunning_migrations() {
    let dir = scratch::Dir::new("database-files-reopen");
    let path = dir.join("test.db");

    let db = database::open(&path).expect("open");
    db.write(|w| {
        let run_id = w.insert_server_run(&run("1.0.0"))?;
        w.insert_agent(
            &SwarmId::main(),
            &NewAgentRecord {
                name: "a".into(),
                token_hash: vec![0],
                recovery_hash: Some(vec![42]),
                created_at: T.into(),
                last_seen: None,
                run_id,
            },
        )
    })
    .await
    .expect("insert");
    drop(db);

    let db = database::open(&path).expect("reopen");
    let found = db
        .read(|r| r.agent(&SwarmId::main(), "a"))
        .await
        .expect("read");
    assert_eq!(found.map(|a| a.name), Some("a".to_string()));
    assert!(
        db.read(|r| r.agent_by_recovery_hash(&SwarmId::main(), "a", &[42]))
            .await
            .expect("recovery lookup after restart")
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copies_taken_during_writes_are_whole_and_consistent() {
    const MESSAGES: i64 = 400;
    let dir = scratch_dir("copy");
    let path = dir.join("live.db");
    let db = database::open(&path).expect("open");
    db.write(|w| {
        let run_id = w.insert_server_run(&run("1.0.0"))?;
        w.insert_agent(
            &SwarmId::main(),
            &NewAgentRecord {
                name: "a".into(),
                token_hash: vec![0],
                recovery_hash: None,
                created_at: T.into(),
                last_seen: None,
                run_id,
            },
        )?;
        w.insert_channel(
            &SwarmId::main(),
            &NewChannelRecord {
                id: "ch_1".into(),
                name: "general".into(),
                purpose: None,
                open: true,
                created_by: "a".into(),
                created_at: T.into(),
                owner: None,
                run_id,
            },
        )
    })
    .await
    .expect("seed");

    let writer = {
        let db = db.clone();
        tokio::spawn(async move {
            for i in 0..MESSAGES {
                db.write(move |w| {
                    w.insert_message(
                        &SwarmId::main(),
                        &NewMessageRecord {
                            channel_id: "ch_1".into(),
                            agent: "a".into(),
                            body: format!("message {i}"),
                            created_at: T.into(),
                            run_id: 1,
                        },
                    )
                })
                .await
                .expect("insert");
            }
        })
    };
    let copies = {
        let (path, dir) = (path.clone(), dir.to_path_buf());
        tokio::task::spawn_blocking(move || {
            (0..8)
                .map(|i| {
                    let copy = dir.join(format!("copy-{i}.db"));
                    database::copy(&path, &copy).expect("copy");
                    copy
                })
                .collect::<Vec<_>>()
        })
    };
    let copies = copies.await.expect("copies");
    writer.await.expect("writer");

    let mut counts = Vec::new();
    for copy in &copies {
        assert!(!std::path::PathBuf::from(format!("{}-wal", copy.display())).exists());
        let conn = rusqlite::Connection::open(copy).expect("open copy");
        let check: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .expect("integrity check");
        assert_eq!(check, "ok");
        // Each message was its own transaction, so a consistent copy holds
        // the first n and nothing else.
        let (count, highest): (i64, Option<i64>) = conn
            .query_row("SELECT count(*), max(id) FROM messages", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("count");
        assert!(count <= MESSAGES);
        assert_eq!(highest.unwrap_or(0), count);
        counts.push(count);
        drop(conn);
        let reopened = database::open(copy).expect("a copy opens as a database");
        let agent = reopened
            .read(|r| r.agent(&SwarmId::main(), "a"))
            .await
            .expect("read copy");
        assert!(agent.is_some());
    }
    assert!(
        counts.iter().any(|&n| n > 0 && n < MESSAGES),
        "no copy was taken while messages were being written: {counts:?}"
    );
}

#[test]
fn a_copy_never_replaces_an_existing_file() {
    let dir = scratch_dir("copy-exists");
    let path = dir.join("live.db");
    drop(database::open(&path).expect("open"));
    let copy = dir.join("copy.db");
    std::fs::write(&copy, "keep me").expect("existing file");
    assert!(database::copy(&path, &copy).is_err());
    assert_eq!(std::fs::read_to_string(&copy).expect("read"), "keep me");
}
