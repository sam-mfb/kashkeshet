//! The process terminal: input, output, terminal detection, the terminal's
//! half of the login link (showing it, and the Enter key) and the default
//! behavior when a command's output pipe closes.

#![cfg(feature = "real")]

mod browser;
pub use browser::BrowserLink;

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

    /// Whether standard input is a terminal, as setup has always checked.
    pub fn interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    /// Shows and flushes a prompt and reads a line, or None at EOF.
    pub fn read_line(&self, prompt: &str) -> Result<Option<String>, String> {
        self.output(format_args!("{prompt}"));
        self.flush_output()
            .map_err(|e| format!("could not write to the terminal: {e}"))?;
        let mut line = String::new();
        let read = io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("could not read the answer: {e}"))?;
        Ok((read != 0).then_some(line))
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
