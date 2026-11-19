//! The rules' scenarios on the real backend. kkt-rules' tests run on kkt-db's
//! fast stand-in, whose SQL is its own; these scenarios are the ones whose
//! answers rest on how a database answers (refusals from its rules, paging
//! and order, search, time windows, waits, failed writes) and that no
//! end-to-end or contract test asserts on kkt-db-sqlite. kkt-rules chose
//! them (11.3); this runs them on the translation the program ships. It is
//! purpose-built integration coverage, not a repeat of the rules' tests:
//! keep it even though they pass.

#[tokio::test(flavor = "multi_thread")]
async fn the_rules_scenarios_hold_on_sqlite() {
    kkt_rules::testing::scenarios::run_all(|| {
        kkt_db_sqlite::MemoryPlace::new()
            .expect("an in-memory SQLite database")
            .open(&[kkt_rules::schema()])
            .expect("the core's tables")
    })
    .await;
}
