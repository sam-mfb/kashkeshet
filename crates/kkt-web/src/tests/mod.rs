//! The page's tests. `testing` holds what every source must do, written
//! once over a source and a gate, and the helpers for driving the page;
//! `preview` runs them against the preview behind its simulated gate, and
//! tests the preview itself. The server's runs, over the rules behind real
//! admission, are in `web_real`'s tests. `viewing` tests that the gate is
//! asked about every request and each viewer reads its own source. Below
//! are tests of how the page words times, which need no source.

mod preview;
mod viewing;

use super::{period_label, uptime};

#[test]
fn uptime_reads_as_the_two_largest_units() {
    let at = |s: &str| format!("2026-10-25T{s}Z");
    let up = |from: &str, to: &str| uptime(from, to).expect("readable times");
    assert_eq!(up(&at("12:00:00"), &at("12:00:59")), "less than a minute");
    assert_eq!(up(&at("12:00:00"), &at("12:01:00")), "1 minute");
    assert_eq!(up(&at("12:00:00"), &at("13:00:00")), "1 hour");
    assert_eq!(up(&at("12:00:00"), &at("14:30:00")), "2 hours 30 minutes");
    assert_eq!(
        up("2026-10-24T11:00:00Z", "2026-10-25T12:00:00Z"),
        "1 day 1 hour"
    );
    assert_eq!(up("2026-10-23T12:00:00Z", "2026-10-25T12:05:00Z"), "2 days");
    // A start after now, from a clock set back, counts as just started.
    assert_eq!(up(&at("13:00:00"), &at("12:00:00")), "less than a minute");
    assert_eq!(uptime("not a time", &at("12:00:00")), None);
}

#[test]
fn stale_periods_read_as_hours_or_days() {
    assert_eq!(period_label(1), "1 hour");
    assert_eq!(period_label(6), "6 hours");
    assert_eq!(period_label(24), "1 day");
    assert_eq!(period_label(168), "7 days");
    assert_eq!(period_label(36), "36 hours");
}
