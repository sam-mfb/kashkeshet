//! Logging starts once in a process: a second start is refused, saying
//! so. Its own test binary, so no other test's logging is in the way.

#[test]
fn logging_started_twice_says_why_the_second_failed() {
    kkt_log_tracing::init_stderr_logging(&["kkt"], kkt_log::Level::Info).expect("the first start");
    assert!(kkt_log_tracing::logging_started());
    let again = kkt_log_tracing::init_stderr_logging(&["kkt"], kkt_log::Level::Info)
        .expect_err("a second start is refused");
    assert_eq!(
        again.to_string(),
        "could not start logging: a global default trace dispatcher has already been set"
    );
    let again =
        kkt_log_tracing::init_logging(None, &["kkt"], kkt_log::Level::Info, kkt_log::Level::Warn)
            .expect_err("a second start is refused");
    assert_eq!(
        again.to_string(),
        "could not start logging: a global default trace dispatcher has already been set"
    );
}
