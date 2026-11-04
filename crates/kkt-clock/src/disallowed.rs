//! Checks that `clippy.toml` keeps forbidding every call that reads or waits
//! on real time. Each function below makes one forbidden call and expects
//! `clippy::disallowed_methods` for it. If an entry leaves `clippy.toml`, or
//! its path stops naming the function, the expectation goes unmet and
//! `cargo clippy -- -D warnings` fails. The test at the end checks that
//! every entry in `clippy.toml` has a function here.
//!
//! None of these functions runs: they only need to compile under clippy.
#![expect(
    dead_code,
    reason = "these calls are only checked by clippy, never run"
)]

use std::sync::mpsc::Receiver;
use std::sync::{Condvar, MutexGuard};
use std::time::Duration;

/// Calls `std::time::SystemTime::now`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn system_time_now() {
    let _now = std::time::SystemTime::now();
}

/// Calls `std::time::SystemTime::elapsed`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn system_time_elapsed() {
    let _elapsed = std::time::UNIX_EPOCH.elapsed();
}

/// Calls `std::time::Instant::now`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn instant_now() {
    let _now = std::time::Instant::now();
}

/// Calls `std::time::Instant::elapsed`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn instant_elapsed(at: std::time::Instant) {
    let _elapsed = at.elapsed();
}

/// Calls `std::thread::sleep`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn thread_sleep() {
    std::thread::sleep(Duration::ZERO);
}

/// Calls `std::thread::sleep_ms`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
#[expect(deprecated, reason = "std deprecates this call, and still has it")]
fn thread_sleep_ms() {
    std::thread::sleep_ms(0);
}

/// Calls `std::thread::park_timeout`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn thread_park_timeout() {
    std::thread::park_timeout(Duration::ZERO);
}

/// Calls `std::thread::park_timeout_ms`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
#[expect(deprecated, reason = "std deprecates this call, and still has it")]
fn thread_park_timeout_ms() {
    std::thread::park_timeout_ms(0);
}

/// Calls `std::sync::mpsc::Receiver::recv_timeout`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn receiver_recv_timeout(receiver: &Receiver<()>) {
    let _got = receiver.recv_timeout(Duration::ZERO);
}

/// Calls `std::sync::Condvar::wait_timeout`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn condvar_wait_timeout(condvar: &Condvar, guard: MutexGuard<'_, ()>) {
    let _woken = condvar.wait_timeout(guard, Duration::ZERO);
}

/// Calls `std::sync::Condvar::wait_timeout_ms`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
#[expect(deprecated, reason = "std deprecates this call, and still has it")]
fn condvar_wait_timeout_ms(condvar: &Condvar, guard: MutexGuard<'_, ()>) {
    let _woken = condvar.wait_timeout_ms(guard, 0);
}

/// Calls `std::sync::Condvar::wait_timeout_while`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn condvar_wait_timeout_while(condvar: &Condvar, guard: MutexGuard<'_, ()>) {
    let _woken = condvar.wait_timeout_while(guard, Duration::ZERO, |()| true);
}

/// Calls `chrono::Utc::now`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn utc_now() {
    let _now = chrono::Utc::now();
}

/// Calls `chrono::Utc::today`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
#[expect(deprecated, reason = "chrono deprecates this call, and still has it")]
fn utc_today() {
    let _today = chrono::Utc::today();
}

/// Calls `chrono::Local::now`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn local_now() {
    let _now = chrono::Local::now();
}

/// Calls `chrono::Local::today`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
#[expect(deprecated, reason = "chrono deprecates this call, and still has it")]
fn local_today() {
    let _today = chrono::Local::today();
}

/// Calls `tokio::time::Instant::now`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_instant_now() {
    let _now = tokio::time::Instant::now();
}

/// Calls `tokio::time::Instant::elapsed`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_instant_elapsed(at: tokio::time::Instant) {
    let _elapsed = at.elapsed();
}

/// Calls `tokio::time::sleep`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_sleep() {
    let _sleep = tokio::time::sleep(Duration::ZERO);
}

/// Calls `tokio::time::sleep_until`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_sleep_until(deadline: tokio::time::Instant) {
    let _sleep = tokio::time::sleep_until(deadline);
}

/// Calls `tokio::time::interval`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_interval() {
    let _ticks = tokio::time::interval(Duration::from_secs(1));
}

/// Calls `tokio::time::interval_at`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_interval_at(start: tokio::time::Instant) {
    let _ticks = tokio::time::interval_at(start, Duration::from_secs(1));
}

/// Calls `tokio::time::timeout`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_timeout() {
    let _bounded = tokio::time::timeout(Duration::ZERO, std::future::ready(()));
}

/// Calls `tokio::time::timeout_at`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks that clippy.toml forbids this call"
)]
fn tokio_timeout_at(deadline: tokio::time::Instant) {
    let _bounded = tokio::time::timeout_at(deadline, std::future::ready(()));
}

/// Each piece of `text` that follows `before` and ends at `after`.
fn between<'a>(text: &'a str, before: &str, after: &str) -> Vec<&'a str> {
    text.split(before)
        .skip(1)
        .filter_map(|rest| rest.split_once(after).map(|(found, _)| found))
        .collect()
}

#[test]
fn every_entry_in_clippy_toml_has_a_call_here() {
    let config = include_str!("../../../clippy.toml");
    let here = include_str!("disallowed.rs");
    let mut listed = between(config, "path = \"", "\"");
    // The newline keeps this line itself from matching.
    let mut checked = between(here, "\n/// Calls `", "`.");
    listed.sort_unstable();
    checked.sort_unstable();
    assert!(!listed.is_empty(), "clippy.toml lists no paths");
    assert_eq!(listed, checked);
}
