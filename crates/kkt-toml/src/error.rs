//! Why a TOML text could not be read, or a record written.

use std::ops::Range;

/// Why a TOML text could not be read. [`ParseError::reason`] is the
/// parser's reason exactly as it wrote it, which may run over several
/// lines; [`ParseError::place`] is where in the text it found the problem,
/// when it knows. Shown with `{}`, it is the parser's whole report, which
/// quotes the line of the text at fault.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{shown}")]
pub struct ParseError {
    reason: String,
    place: Option<Place>,
    shown: String,
}

/// A place in a text: its line and column, both counted from 1. The column
/// counts characters, not bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub line: usize,
    pub column: usize,
}

impl ParseError {
    pub(crate) fn new(text: &str, reason: &str, span: Option<Range<usize>>, shown: String) -> Self {
        let place = span
            .and_then(|span| text.get(..span.start))
            .map(|before| Place {
                line: before.matches('\n').count() + 1,
                column: before
                    .rsplit('\n')
                    .next()
                    .map_or(0, |start| start.chars().count())
                    + 1,
            });
        ParseError {
            reason: reason.to_string(),
            place,
            shown,
        }
    }

    /// The parser's reason, exactly as it wrote it.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Where in the text the parser found the problem, when it knows.
    pub fn place(&self) -> Option<Place> {
        self.place
    }
}

/// Why a record could not be written as TOML. Shown with `{}`, it is the
/// writer's own reason.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct WriteError(String);

impl WriteError {
    pub(crate) fn new(reason: String) -> Self {
        WriteError(reason)
    }
}
