//! The terminal's part of the Tailscale login: showing the link, offering to
//! open it in a browser when Enter is pressed, and opening it with the
//! system opener. `kkt_container::provision` only knows [`LoginLink`].

use kkt_container::LoginLink;
use std::io::{self, IsTerminal};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// At a terminal, shows the link and offers to open it when Enter is pressed.
/// Away from a terminal, such as in a script or for an agent, it only prints
/// the link. It never opens a browser unless asked.
pub struct TerminalLink {
    opener: String,
    pressed: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
}

impl TerminalLink {
    #[allow(clippy::new_without_default)]
    pub fn new() -> TerminalLink {
        TerminalLink::with_opener(system_opener())
    }

    /// A link that opens with `opener`, a program that takes the link as its
    /// only argument.
    fn with_opener(opener: impl Into<String>) -> TerminalLink {
        TerminalLink {
            opener: opener.into(),
            pressed: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(Mutex::new(None)),
        }
    }
}

impl LoginLink for TerminalLink {
    fn show(&self, url: &str) {
        if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
            println!("Log in to Tailscale by opening: {url}");
            return;
        }
        println!("Log in to Tailscale by opening this link:\n  {url}");
        println!("Press Enter to open it in your browser (or open it yourself).");
        // A key press is waited for on its own thread, so that setup goes on
        // watching for the login meanwhile. If the person logs in another
        // way, this thread is left waiting; it reads nothing else.
        let pressed = Arc::clone(&self.pressed);
        std::thread::spawn(move || {
            if let Some(message) = wait_for_enter(&mut io::stdin().lock(), &pressed) {
                println!("{message}");
            }
        });
    }

    fn open_requested(&self) -> bool {
        self.pressed.swap(false, Ordering::SeqCst)
    }

    fn open(&self, url: &str) -> io::Result<()> {
        let mut child = std::process::Command::new(&self.opener)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // The opener is waited for on its own thread, which also reaps it:
        // setup does not wait, and learns later if it failed.
        let failure = Arc::clone(&self.failure);
        std::thread::spawn(move || {
            let why = match child.wait() {
                Ok(status) if status.success() => return,
                Ok(status) => format!("the opener ended with {status}"),
                Err(e) => format!("could not wait for the opener: {e}"),
            };
            record_failure(&failure, why);
        });
        Ok(())
    }

    fn open_failure(&self) -> Option<String> {
        match self.failure.lock() {
            Ok(mut slot) => slot.take(),
            // The slot holds only a finished message, so a thread that
            // panicked while holding it cannot have left it half written:
            // what it holds is still reported, once, with the poisoning as
            // its cause. Until there is a failure the slot stays poisoned,
            // so the cause goes with the failure it came before.
            Err(poisoned) => {
                let cause = poisoned.to_string();
                let why = poisoned.into_inner().take()?;
                self.failure.clear_poison();
                Some(format!("{why} ({cause})"))
            }
        }
    }
}

/// Waits for a line on `input` and records it in `pressed`. Returns what to
/// tell the person if the line could not be read. The message never
/// includes what was read.
fn wait_for_enter(input: &mut impl io::BufRead, pressed: &AtomicBool) -> Option<String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => {
            pressed.store(true, Ordering::SeqCst);
            None
        }
        Err(e) => Some(format!(
            "Could not read the Enter key ({e}); open the link above yourself."
        )),
    }
}

/// Keeps `why` for `open_failure`. A poisoned slot still takes it and stays
/// poisoned, so that `open_failure` reports the poisoning with it.
fn record_failure(failure: &Mutex<Option<String>>, why: String) {
    *failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
}

/// The program that opens a link in the person's browser.
fn system_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn there_is_no_request_until_a_key_is_pressed_and_a_request_is_taken_once() {
        let link = TerminalLink::new();
        assert!(!link.open_requested());
        link.pressed.store(true, Ordering::SeqCst);
        assert!(link.open_requested());
        assert!(!link.open_requested());
    }

    #[test]
    fn a_missing_opener_fails_to_start() {
        let link = TerminalLink::with_opener("/no/such/kkt-opener");
        assert!(link.open("https://example.com/a").is_err());
        assert_eq!(link.open_failure(), None);
    }

    /// Waits for `link` to report a failure, for up to a few seconds.
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "waits in real time for a real opener process to finish"
    )]
    fn failure_of(link: &TerminalLink) -> Option<String> {
        for _ in 0..500 {
            if let Some(why) = link.open_failure() {
                return Some(why);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    // These tests run programs that already exist, never a script a test
    // writes just before running it: running a just-written file can fail
    // with "Text file busy" while other threads of the test program start
    // processes, which made them fail now and then.

    /// A login link's secret part, which no message may repeat.
    const MARKER: &str = "kkt-marker-7f3a";

    #[cfg(unix)]
    #[test]
    fn an_opener_that_exists_but_fails_is_reported_without_waiting_for_it() {
        // `false` ignores the link and exits with status 1.
        let link = TerminalLink::with_opener("false");
        let url = format!("https://example.com/a?token={MARKER}");
        assert!(link.open(&url).is_ok());
        let why = failure_of(&link).expect("the failure is reported");
        assert!(why.contains("exit status: 1"), "{why}");
        assert!(!why.contains(MARKER), "{why}");
        // Reported once.
        assert_eq!(link.open_failure(), None);
    }

    /// Poisons `link`'s failure slot, as a thread that panics while holding
    /// it would.
    fn poison(link: &TerminalLink) {
        let failure = Arc::clone(&link.failure);
        let ended = std::thread::spawn(move || {
            let _slot = failure.lock();
            panic!("a thread panics while holding the failure slot");
        })
        .join();
        assert!(ended.is_err());
        assert!(link.failure.is_poisoned());
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_after_the_slot_is_poisoned_is_reported_with_the_poisoning_as_its_cause() {
        let link = TerminalLink::with_opener("false");
        poison(&link);
        let url = format!("https://example.com/a?token={MARKER}");
        assert!(link.open(&url).is_ok());
        let why = failure_of(&link).expect("the failure is reported");
        assert!(
            why.starts_with("the opener ended with exit status: 1 (poisoned lock"),
            "{why}"
        );
        assert!(!why.contains(MARKER), "{why}");
        // Reported once, and the slot works again.
        assert_eq!(link.open_failure(), None);
        assert!(!link.failure.is_poisoned());
    }

    #[test]
    fn a_failure_kept_in_a_poisoned_slot_is_reported_once_with_the_poisoning_as_its_cause() {
        let link = TerminalLink::with_opener("false");
        poison(&link);
        // No failure yet: nothing to report, and the slot stays poisoned.
        assert_eq!(link.open_failure(), None);
        assert!(link.failure.is_poisoned());
        record_failure(&link.failure, "the opener ended with exit status: 1".into());
        assert_eq!(
            link.open_failure().as_deref(),
            Some(
                "the opener ended with exit status: 1 (poisoned lock: another task failed inside)"
            )
        );
        assert_eq!(link.open_failure(), None);
    }

    #[test]
    fn a_line_is_a_request_to_open_and_the_end_of_input_is_not() {
        let pressed = AtomicBool::new(false);
        assert_eq!(wait_for_enter(&mut io::Cursor::new(""), &pressed), None);
        assert!(!pressed.load(Ordering::SeqCst));
        assert_eq!(wait_for_enter(&mut io::Cursor::new("\n"), &pressed), None);
        assert!(pressed.load(Ordering::SeqCst));
    }

    #[test]
    fn an_unreadable_key_press_is_reported_with_its_cause_and_never_what_was_read() {
        let pressed = AtomicBool::new(false);
        // Not UTF-8, so the line cannot be read.
        let mut input = io::Cursor::new([MARKER.as_bytes(), b"\xff\n"].concat());
        let message = wait_for_enter(&mut input, &pressed).expect("the failure is reported");
        assert!(
            message
                .starts_with("Could not read the Enter key (stream did not contain valid UTF-8)"),
            "{message}"
        );
        assert!(
            message.ends_with("; open the link above yourself."),
            "{message}"
        );
        assert!(!message.contains(MARKER), "{message}");
        assert!(!pressed.load(Ordering::SeqCst));
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
        let dir = crate::scratch::Dir::new("kkt-opened");
        let marker = dir.join("marker");
        let link = TerminalLink::with_opener(opener);
        assert!(link.open(marker.to_str().unwrap()).is_ok());
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
        assert_eq!(link.open_failure(), None);
    }
}
