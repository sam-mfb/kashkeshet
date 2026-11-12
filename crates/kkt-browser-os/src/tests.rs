//! Which opener, how a failure is worded and kept, and, with `real`, the
//! opener itself.

use super::*;
use std::sync::Arc;

#[test]
fn the_system_opener_is_open_on_a_mac_and_xdg_open_elsewhere() {
    let expected = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    assert_eq!(system_opener(), expected);
}

#[test]
fn an_opener_that_ended_is_said_by_its_status_or_why_it_could_not_be_waited_for() {
    assert_eq!(failure_text(Ok((true, "exit status: 0".into()))), None);
    assert_eq!(
        failure_text(Ok((false, "exit status: 1".into()))).as_deref(),
        Some("the opener ended with exit status: 1")
    );
    assert_eq!(
        failure_text(Err(std::io::Error::other("gone"))).as_deref(),
        Some("could not wait for the opener: gone")
    );
}

/// Poisons `slot`, as a thread that panics while holding it would.
fn poison(slot: &Arc<Mutex<Option<String>>>) {
    let held = Arc::clone(slot);
    let ended = std::thread::spawn(move || {
        let _slot = held.lock();
        panic!("a thread panics while holding the failure slot");
    })
    .join();
    assert!(ended.is_err());
    assert!(slot.is_poisoned());
}

#[test]
fn a_failure_kept_in_a_poisoned_slot_is_reported_once_with_the_poisoning_as_its_cause() {
    let slot = Arc::new(Mutex::new(None));
    poison(&slot);
    // No failure yet: nothing to report, and the slot stays poisoned.
    assert_eq!(take_failure(&slot), None);
    assert!(slot.is_poisoned());
    record_failure(&slot, "the opener ended with exit status: 1".into());
    assert_eq!(
        take_failure(&slot).as_deref(),
        Some("the opener ended with exit status: 1 (poisoned lock: another task failed inside)")
    );
    assert_eq!(take_failure(&slot), None);
    assert!(!slot.is_poisoned());
}

/// The opener itself: these run programs that already exist, never a script
/// a test writes just before running it, since running a just-written file
/// can fail with "Text file busy" while other threads start processes.
#[cfg(feature = "real")]
mod real {
    use super::*;
    use kkt_browser::Browser;

    /// A login link's secret part, which no message may repeat.
    const MARKER: &str = "kkt-marker-7f3a";

    /// Waits for `browser` to report a failure, for up to a few seconds.
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "waits in real time for a real opener process to finish"
    )]
    fn failure_of(browser: &SystemBrowser) -> Option<String> {
        for _ in 0..500 {
            if let Some(why) = browser.open_failure() {
                return Some(why);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn a_missing_opener_fails_to_start() {
        let browser = SystemBrowser::with_opener("/no/such/kkt-opener");
        assert!(browser.open("https://example.com/a").is_err());
        assert_eq!(browser.open_failure(), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_opener_that_exists_but_fails_is_reported_without_waiting_for_it() {
        // `false` ignores the link and exits with status 1.
        let browser = SystemBrowser::with_opener("false");
        let url = format!("https://example.com/a?token={MARKER}");
        assert!(browser.open(&url).is_ok());
        let why = failure_of(&browser).expect("the failure is reported");
        assert!(why.contains("exit status: 1"), "{why}");
        assert!(!why.contains(MARKER), "{why}");
        // Reported once.
        assert_eq!(browser.open_failure(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_after_the_slot_is_poisoned_is_reported_with_the_poisoning_as_its_cause() {
        let browser = SystemBrowser::with_opener("false");
        poison(&browser.failure);
        let url = format!("https://example.com/a?token={MARKER}");
        assert!(browser.open(&url).is_ok());
        let why = failure_of(&browser).expect("the failure is reported");
        assert!(
            why.starts_with("the opener ended with exit status: 1 (poisoned lock"),
            "{why}"
        );
        assert!(!why.contains(MARKER), "{why}");
        // Reported once, and the slot works again.
        assert_eq!(browser.open_failure(), None);
        assert!(!browser.failure.is_poisoned());
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "waits in real time for a real opener process to write its file"
    )]
    fn an_opener_that_works_gets_the_link_and_reports_no_failure() {
        let opener = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/opener-writes-its-argument-to-a-file.sh"
        );
        // The fixture treats the link as the path of a file to write to.
        let dir = kkt_scratch::Dir::new("kkt-opened");
        let marker = dir.join("marker");
        let browser = SystemBrowser::with_opener(opener);
        assert!(browser.open(marker.to_str().unwrap()).is_ok());
        // The shell makes the file, empty, a moment before it writes to it:
        // wait for what is written, not for the file.
        let written = || std::fs::read_to_string(&marker).unwrap_or_default();
        for _ in 0..500 {
            if !written().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(written().trim(), "opened");
        assert_eq!(browser.open_failure(), None);
    }
}
