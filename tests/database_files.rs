//! The database file as the program opens it: reopening keeps what was
//! written, and copies taken while a server writes are whole and
//! consistent, and never replace a file.

use kashkeshet::machine::database;
use kkt_rules::testing::{self, Memory};
use kkt_scratch as scratch;
use std::sync::Arc;

/// A fresh, empty directory for one test's files, removed when dropped.
fn scratch_dir(test: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("database-files-{test}"))
}

/// The rules over `db`, on the real clock and randomness, with no limit on
/// how fast an avatar sends that a test could meet.
async fn rules_on(db: kkt_rules::Db) -> kkt_rules::Rules {
    testing::rules_on(
        db,
        Memory {
            settings: kkt_rules::Settings {
                sends_per_minute: u32::MAX,
                ..kkt_rules::Settings::DEFAULT
            },
            ..Memory::new(
                Arc::new(kkt_clock_system::SystemClock),
                Arc::new(kkt_entropy_os::OsEntropy),
            )
        },
    )
    .await
    .expect("the rules start")
}

#[tokio::test]
async fn file_database_reopens_without_rerunning_migrations() {
    let dir = scratch::Dir::new("database-files-reopen");
    let path = dir.join("test.db");

    let db = database::open(&path).expect("open");
    let registered = rules_on(db).await.register().await.expect("an avatar");

    // Reopened, the file still holds the avatar and its recovery code.
    let db = database::open(&path).expect("reopen");
    let rules = rules_on(db).await;
    rules
        .authenticate(Some(&registered.token))
        .await
        .expect("the avatar's token after restart");
    let reclaimed = rules
        .reclaim(registered.handle.clone(), registered.recovery_code.clone())
        .await
        .expect("recovery after restart");
    assert_eq!(reclaimed.handle, registered.handle);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copies_taken_during_writes_are_whole_and_consistent() {
    const MESSAGES: i64 = 400;
    let dir = scratch_dir("copy");
    let path = dir.join("live.db");
    let db = database::open(&path).expect("open");
    let rules = rules_on(db).await;
    let registered = rules.register().await.expect("an avatar");
    let me = rules
        .authenticate(Some(&registered.token))
        .await
        .expect("its token");
    let channel = rules
        .create_channel(
            &me,
            kkt_api_agent::NewChannel {
                name: "general".into(),
                open: true,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("a channel")
        .id;

    let writer = {
        let rules = rules.clone();
        tokio::spawn(async move {
            for i in 0..MESSAGES {
                rules
                    .send(&me, channel.clone(), format!("message {i}"))
                    .await
                    .expect("send");
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
        rules_on(reopened)
            .await
            .authenticate(Some(&registered.token))
            .await
            .expect("the avatar in the copy");
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
