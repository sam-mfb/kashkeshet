//! The process terminal: output and errors, output-terminal detection,
//! and the default behavior when a command's output pipe closes.

#![cfg(feature = "real")]

use std::fmt;
use std::io::{self, IsTerminal, Write};

/// The process's standard output and standard error.
pub struct Terminal;

impl Terminal {
    /// Writes formatted output exactly as given, without adding a newline.
    pub fn output(&self, text: fmt::Arguments<'_>) {
        print!("{text}");
    }

    /// Writes formatted errors exactly as given, without adding a newline.
    pub fn errors(&self, text: fmt::Arguments<'_>) {
        eprint!("{text}");
    }

    /// Flushes a prompt before the program waits for an answer.
    pub fn flush_output(&self) -> io::Result<()> {
        io::stdout().flush()
    }

    /// Writes and flushes one message while holding the output lock.
    pub fn message(&self, line: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        writeln!(out, "{line}").and_then(|()| out.flush())
    }

    /// Whether standard output is connected to a terminal.
    pub fn output_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }
}

/// Lets a closed standard output, as with `kkt status | head`, end the
/// command the way it ends other Unix commands (SIGPIPE), where Rust would
/// otherwise panic at the next print. Not for `kkt serve`: a server keeps
/// running when whatever reads its output goes away, and systemd starts it
/// with SIGPIPE ignored on purpose. Network writes are unaffected: Rust
/// asks the system not to signal for those.
pub fn end_on_closed_output() {
    // SAFETY: this sets only what happens on SIGPIPE, to the system's
    // default; no handler of ours runs, and nothing else in kkt sets it.
    #[cfg(unix)]
    if unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) } == libc::SIG_ERR {
        Terminal.errors(format_args!(
            "warning: could not restore the default for a closed output: {}\n",
            std::io::Error::last_os_error()
        ));
    }
}
