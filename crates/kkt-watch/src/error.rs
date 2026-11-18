//! Why a watch ended without its outcome, worded for the person.

use kkt_agent_client::Failure;
use std::error::Error;

/// Why [`run`](crate::run) stopped.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// The timeout given ends past the last time the clock can tell.
    #[error("the timeout of {seconds} seconds is too long")]
    TimeoutTooLong { seconds: u64 },
    /// The clock is so near its end that a wait cannot be reckoned.
    #[error("the clock is out of range")]
    ClockOutOfRange,
    /// The timeout ran out before watching started.
    #[error("no answer from the server before the timeout")]
    NoAnswerBeforeTimeout,
    /// No channel the agent can see has the name given.
    #[error("no visible channel named {name:?}")]
    NoChannelNamed { name: String },
    /// Several channels the agent can see have the name given.
    #[error("several visible channels named {name:?}: {}; use --channel with an ID", ids.join(", "))]
    SeveralChannelsNamed { name: String, ids: Vec<String> },
    /// The server failed in a way that waiting will not mend.
    #[error("{0}")]
    Server(Failure),
    /// The place the last message ID is kept failed.
    #[error(transparent)]
    Place(PlaceError),
    /// The report could not show a message.
    #[error(transparent)]
    Report(ReportError),
}

/// Why a [`Place`](crate::Place) could not load or save: the
/// implementation's own error, which kkt-watch passes on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct PlaceError(pub Box<dyn Error + Send + Sync>);

/// Why a [`Report`](crate::Report) could not show a message: the
/// implementation's own error, which kkt-watch passes on unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ReportError(pub Box<dyn Error + Send + Sync>);
