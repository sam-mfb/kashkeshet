//! The rules' store on the real backend. kkt-rules' own tests run on
//! kkt-db's fast stand-in, whose SQL is its own; this runs the store's
//! questions and changes on kkt-db-sqlite's translation, which the program
//! ships. It is purpose-built integration coverage, not a repeat of the
//! rules' tests: keep it even though they pass.
//!
//! `swarms_stay_apart` asks every store read that takes a swarm, many ways,
//! and makes every change, in two swarms, and checks that neither sees the
//! other: the store's whole surface, on SQLite.

#[tokio::test]
async fn the_store_keeps_swarms_apart_on_sqlite() {
    let place = kkt_db_sqlite::MemoryPlace::new().expect("an in-memory SQLite database");
    let db = kkt_rules::Db::new(
        place
            .open(&[kkt_rules::schema()])
            .expect("the core's tables"),
    );
    if let Err(leak) = kkt_rules::testing::swarms_stay_apart_in(&db).await {
        panic!("{leak}");
    }
}
