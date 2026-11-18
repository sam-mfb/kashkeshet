//! Showing a link at the terminal, and waiting for Enter as a request to
//! open it in a browser. A caller asks [`BrowserLink::open_requested`] and
//! opens the link its own way.

use crate::Terminal;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A link shown at the terminal, with a wait for Enter: at a terminal, it
/// shows the link and offers to open it when Enter is pressed; away from
/// one, such as in a script, it only prints the link. It opens no browser
/// itself.
pub struct BrowserLink {
    lead: String,
    pressed: Arc<AtomicBool>,
}

impl BrowserLink {
    /// `lead` says what to do with the link, in the caller's words, such as
    /// "Sign in by opening". At a terminal it is followed by " this link:"
    /// and the link on a line of its own; away from one, by ": " and the
    /// link.
    pub fn new(lead: String) -> BrowserLink {
        BrowserLink {
            lead,
            pressed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl BrowserLink {
    /// Shows the link, offering browser opening only at a terminal.
    pub fn show(&self, url: &str) {
        let at_terminal = Terminal.interactive() && Terminal.output_is_terminal();
        Terminal.output(format_args!("{}", self.said(url, at_terminal)));
        if !at_terminal {
            return;
        }
        // A key press is waited for on its own thread, so that the caller
        // goes on with its work meanwhile. If the person goes on another
        // way, this thread is left waiting; it reads nothing else.
        let pressed = Arc::clone(&self.pressed);
        std::thread::spawn(move || {
            if let Some(message) = wait_for_enter(&mut io::stdin().lock(), &pressed) {
                Terminal.output(format_args!("{message}\n"));
            }
        });
    }

    /// What [`show`](Self::show) prints, at a terminal or away from one.
    fn said(&self, url: &str, at_terminal: bool) -> String {
        let lead = &self.lead;
        if at_terminal {
            format!(
                "{lead} this link:\n  {url}\n\
                 Press Enter to open it in your browser (or open it yourself).\n"
            )
        } else {
            format!("{lead}: {url}\n")
        }
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
        let link = BrowserLink::new("Open".to_string());
        assert!(!link.open_requested());
        link.pressed.store(true, Ordering::SeqCst);
        assert!(link.open_requested());
        assert!(!link.open_requested());
    }

    #[test]
    fn the_link_is_shown_after_the_callers_words() {
        let link = BrowserLink::new("Sign in by opening".to_string());
        assert_eq!(
            link.said("https://example.com/a", false),
            "Sign in by opening: https://example.com/a\n"
        );
        assert_eq!(
            link.said("https://example.com/a", true),
            "Sign in by opening this link:\n  https://example.com/a\n\
             Press Enter to open it in your browser (or open it yourself).\n"
        );
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
