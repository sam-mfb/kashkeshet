//! The process terminal: input, output, terminal detection, a link shown
//! with a wait for Enter ([`BrowserLink`]) and the default behavior when a
//! command's output pipe closes.

mod browser;
pub use browser::BrowserLink;

use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether a closed standard output or error ends the process as SIGPIPE
/// does ([`end_on_closed_output`]).
static END_ON_CLOSED: AtomicBool = AtomicBool::new(false);

/// The exit code a process ends with where it cannot end by SIGPIPE: 128
/// and the signal's number, as a shell reports one ended by it.
const CLOSED_PIPE_EXIT: i32 = 141;

/// The process's standard output and standard error.
pub struct Terminal;

impl Terminal {
    /// Writes formatted output exactly as given, without adding a newline.
    /// A failed write panics, as `print!` does, unless the output was closed
    /// after [`end_on_closed_output`].
    pub fn output(&self, text: fmt::Arguments<'_>) {
        if let Err(error) = ended_if_closed(io::stdout().lock().write_fmt(text)) {
            panic!("failed printing to stdout: {error}");
        }
    }

    /// Writes formatted errors exactly as given, without adding a newline.
    /// A failed write panics, as `eprint!` does, unless the error output
    /// was closed after [`end_on_closed_output`].
    pub fn errors(&self, text: fmt::Arguments<'_>) {
        if let Err(error) = ended_if_closed(io::stderr().lock().write_fmt(text)) {
            panic!("failed printing to stderr: {error}");
        }
    }

    /// Flushes a prompt before the program waits for an answer.
    pub fn flush_output(&self) -> io::Result<()> {
        ended_if_closed(io::stdout().flush())
    }

    /// Writes and flushes one message while holding the output lock.
    pub fn message(&self, line: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        ended_if_closed(writeln!(out, "{line}").and_then(|()| out.flush()))
    }

    /// Whether standard input is a terminal.
    pub fn interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    /// Shows and flushes a prompt and reads a line, or None at EOF.
    pub fn read_line(&self, prompt: &str) -> Result<Option<String>, ReadLineError> {
        self.output(format_args!("{prompt}"));
        self.flush_output().map_err(ReadLineError::Write)?;
        let mut line = String::new();
        let read = io::stdin()
            .read_line(&mut line)
            .map_err(ReadLineError::Read)?;
        Ok((read != 0).then_some(line))
    }

    /// Whether standard output is connected to a terminal.
    pub fn output_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }

    /// Whether standard error is connected to a terminal.
    pub fn errors_are_terminal(&self) -> bool {
        io::stderr().is_terminal()
    }
}

/// Standard output as a writer, for a library that writes on its own, such
/// as a log: a write that finds it closed after [`end_on_closed_output`]
/// ends the process, as [`Terminal::output`] does; any other failure is
/// returned.
pub struct OutputWriter;

impl Write for OutputWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        ended_if_closed(io::stdout().write(bytes))
    }
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        ended_if_closed(io::stdout().lock().write_all(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        ended_if_closed(io::stdout().flush())
    }
}

/// Standard error as a writer, as [`OutputWriter`] is standard output.
pub struct ErrorWriter;

impl Write for ErrorWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        ended_if_closed(io::stderr().write(bytes))
    }
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        ended_if_closed(io::stderr().lock().write_all(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        ended_if_closed(io::stderr().flush())
    }
}

/// Lets a closed standard output or error, as with `program status | head`,
/// end the command the way it ends other Unix commands (SIGPIPE), where
/// Rust would otherwise panic at the next print. For commands only: a
/// server keeps running when whatever reads its output goes away, and
/// systemd starts it with SIGPIPE ignored on purpose.
///
/// SIGPIPE stays ignored until a write through [`Terminal`],
/// [`OutputWriter`] or [`ErrorWriter`] finds its output closed; only then is the system's default restored and the
/// signal raised. So a network connection the other end closes, whatever
/// write finds it closed, is an error the command reports, not the end of
/// the command.
pub fn end_on_closed_output() {
    END_ON_CLOSED.store(true, Ordering::Relaxed);
}

/// `result`, unless it failed because the output was closed after
/// [`end_on_closed_output`]: then the process ends there.
fn ended_if_closed<T>(result: io::Result<T>) -> io::Result<T> {
    match result {
        Err(error)
            if error.kind() == io::ErrorKind::BrokenPipe
                && END_ON_CLOSED.load(Ordering::Relaxed) =>
        {
            end_by_closed_pipe()
        }
        result => result,
    }
}

/// Ends the process as a closed pipe ends other Unix commands: by SIGPIPE,
/// with the system's default for it, or, where that cannot be, with
/// [`CLOSED_PIPE_EXIT`].
fn end_by_closed_pipe() -> ! {
    // SAFETY: this sets only what happens on SIGPIPE, to the system's
    // default, and raises it in this thread, which ends the process; no
    // handler of ours runs.
    #[cfg(unix)]
    unsafe {
        if libc::signal(libc::SIGPIPE, libc::SIG_DFL) != libc::SIG_ERR {
            libc::raise(libc::SIGPIPE);
        }
    }
    std::process::exit(CLOSED_PIPE_EXIT)
}

/// Why [`Terminal::read_line`] could not read a line.
#[derive(Debug)]
pub enum ReadLineError {
    /// The prompt could not be written.
    Write(io::Error),
    /// The line could not be read.
    Read(io::Error),
}

impl std::fmt::Display for ReadLineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadLineError::Write(e) => write!(f, "could not write to the terminal: {e}"),
            ReadLineError::Read(e) => write!(f, "could not read the answer: {e}"),
        }
    }
}

impl std::error::Error for ReadLineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReadLineError::Write(e) | ReadLineError::Read(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ReadLineError;
    use std::io;

    /// A real terminal cannot be made to fail in a test, so the two
    /// sentences are pinned on the kinds that say them.
    #[test]
    fn a_line_not_read_says_which_step_failed() {
        let closed = || io::Error::new(io::ErrorKind::BrokenPipe, "closed");
        assert_eq!(
            ReadLineError::Write(closed()).to_string(),
            "could not write to the terminal: closed"
        );
        assert_eq!(
            ReadLineError::Read(closed()).to_string(),
            "could not read the answer: closed"
        );
        let source = std::error::Error::source(&ReadLineError::Read(closed()))
            .expect("the io error")
            .to_string();
        assert_eq!(source, "closed");
    }
}
