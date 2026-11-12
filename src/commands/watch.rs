//! The terminal as kkt-watch's `Report`, for `kkt watch`, which
//! kkt-agent-commands runs.

use kkt_terminal_os::Terminal;
use kkt_watch as watch;

/// Message lines to standard output, flushed one by one; notes to
/// standard error.
pub(crate) struct WatchOutput;

impl watch::Report for WatchOutput {
    fn message(&mut self, line: &str) -> Result<(), String> {
        Terminal
            .message(line)
            .map_err(|e| format!("could not print a message: {e}"))
    }

    fn note(&mut self, text: &str) {
        Terminal.errors(format_args!("kkt watch: {text}\n"));
    }
}
