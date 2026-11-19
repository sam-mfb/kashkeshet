//! The terminal as kkt-watch's `Report`, for `kkt watch`, which
//! kkt-agent-commands runs.

use kkt_terminal::Terminal;
use kkt_watch as watch;

/// Message lines to standard output, flushed one by one; notes to
/// standard error.
pub(crate) struct WatchOutput;

impl watch::Report for WatchOutput {
    fn message(&mut self, line: &str) -> Result<(), watch::ReportError> {
        Terminal
            .message(line)
            .map_err(|e| watch::ReportError(format!("could not print a message: {e}").into()))
    }

    fn note(&mut self, text: &str) {
        Terminal.errors(format_args!("kkt watch: {text}\n"));
    }
}
