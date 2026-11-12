//! The terminal's part of the Tailscale login: showing the link, and
//! offering to open it in a browser when Enter is pressed. Opening it is
//! kkt-browser's; the program adapts this to the container's login-link
//! contract.

use crate::Terminal;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// At a terminal, shows the link and offers to open it when Enter is pressed.
/// Away from a terminal, such as in a script or for an agent, it only prints
/// the link. It never opens a browser itself.
pub struct BrowserLink {
    pressed: Arc<AtomicBool>,
}

impl BrowserLink {
    #[allow(clippy::new_without_default)]
    pub fn new() -> BrowserLink {
        BrowserLink {
            pressed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl BrowserLink {
    /// Shows the link, offering browser opening only at a terminal.
    pub fn show(&self, url: &str) {
        if !(Terminal.interactive() && Terminal.output_is_terminal()) {
            Terminal.output(format_args!("Log in to Tailscale by opening: {url}\n"));
            return;
        }
        Terminal.output(format_args!(
            "Log in to Tailscale by opening this link:\n  {url}\n"
        ));
        Terminal.output(format_args!(
            "Press Enter to open it in your browser (or open it yourself).\n"
        ));
        // A key press is waited for on its own thread, so that setup goes on
        // watching for the login meanwhile. If the person logs in another
        // way, this thread is left waiting; it reads nothing else.
        let pressed = Arc::clone(&self.pressed);
        std::thread::spawn(move || {
            if let Some(message) = wait_for_enter(&mut io::stdin().lock(), &pressed) {
                Terminal.output(format_args!("{message}\n"));
            }
        });
    }

    /// Takes one pending Enter-key request to open the link.
    pub fn open_requested(&self) -> bool {
        self.pressed.swap(false, Ordering::SeqCst)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A secret, which no message may repeat.
    const MARKER: &str = "kkt-marker-7f3a";

    #[test]
    fn there_is_no_request_until_a_key_is_pressed_and_a_request_is_taken_once() {
        let link = BrowserLink::new();
        assert!(!link.open_requested());
        link.pressed.store(true, Ordering::SeqCst);
        assert!(link.open_requested());
        assert!(!link.open_requested());
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
}
