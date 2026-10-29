//! `kkt watch`: wait for one agent's new messages from the shell, with no
//! model running. It prints one line per new message and keeps going, or,
//! with `once`, stops after the first new messages.
//!
//! What it depends on is injected: the server (`Server`, over MCP in
//! `remote.rs`), the time (`Clock`), where it keeps its place (`Place`) and
//! where its output goes (`Report`).

mod remote;
#[cfg(test)]
mod tests;

pub use remote::Remote;

use crate::clock::Clock;
use crate::config::{
    WATCH_BODY_CHARS, WATCH_CALL_GRACE, WATCH_RETRY_FIRST, WATCH_RETRY_MAX, WATCH_WAIT,
};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use std::fmt;
use std::time::Duration;

/// A new message, as `kkt watch` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
}

/// Why a call to the server failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The server could not be reached, the connection broke, or its answer
    /// made no sense.
    Lost(String),
    /// The server answered with one of its error codes, such as
    /// `unauthenticated`.
    Refused { code: String, message: String },
}

impl Failure {
    /// Whether trying again later may work. A lost connection or a busy
    /// server may recover; a bad token or a missing channel will not.
    fn passing(&self) -> bool {
        match self {
            Failure::Lost(_) => true,
            Failure::Refused { code, .. } => matches!(code.as_str(), "rate_limited" | "internal"),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Lost(reason) => write!(f, "could not reach the server: {reason}"),
            Failure::Refused { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

/// The server, acting as one agent.
pub trait Server: Send + Sync {
    /// The agent's name.
    fn name(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// The highest message ID in the agent's channels, or 0 if none.
    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>>;
    /// Messages newer than `after_id`, oldest first, waiting up to `wait`
    /// for one. `channels` limits them to those channels.
    fn wait<'a>(
        &'a self,
        after_id: i64,
        channels: Option<&'a [String]>,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Vec<Message>, Failure>>;
}

/// Where the highest message ID seen is kept between runs.
pub trait Place {
    fn load(&self) -> Result<Option<i64>, String>;
    fn save(&self, id: i64) -> Result<(), String>;
}

/// Where the output goes.
pub trait Report {
    /// One line for one new message. Nothing else is reported this way.
    fn message(&mut self, line: &str) -> Result<(), String>;
    /// Anything else: where watching starts, lost connections.
    fn note(&mut self, text: &str);
}

pub struct Options {
    /// Only these channels; `None` for every channel the agent is in.
    pub channels: Option<Vec<String>>,
    /// Start after this ID instead of the place's or the latest.
    pub after_id: Option<i64>,
    /// Stop after the first new messages.
    pub once: bool,
    /// With `once`, stop after this long even if nothing came.
    pub timeout: Option<Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `once`: new messages were printed.
    Received,
    /// `once` with `timeout`: the time ran out with nothing new.
    TimedOut,
}

/// Watches until `options` say to stop. Without `once` it returns only on
/// an error.
///
/// Failures before watching starts (reaching the server, finding the
/// agent's name and start) end the watch, so a wrong address, key or token
/// is reported at once. After that, a lost connection is retried with a
/// growing pause and never ends the watch; only an error that cannot pass,
/// such as a revoked token, does. The agent's own messages are skipped.
pub async fn run(
    server: &dyn Server,
    clock: &dyn Clock,
    place: Option<&dyn Place>,
    report: &mut dyn Report,
    options: &Options,
) -> Result<Outcome, String> {
    let deadline = match options.timeout {
        Some(timeout) => Some(clock.now() + chrono_duration(timeout)?),
        None => None,
    };
    let me = server.name().await.map_err(|e| e.to_string())?;
    let saved = match (options.after_id, place) {
        (Some(_), _) | (None, None) => None,
        (None, Some(place)) => place.load()?,
    };
    let mut after = match options.after_id.or(saved) {
        Some(id) => id,
        None => {
            let id = server.latest_id().await.map_err(|e| e.to_string())?;
            if let Some(place) = place {
                place.save(id)?;
            }
            id
        }
    };
    report.note(&format!("watching as {me}, after message {after}"));
    let mut pause = WATCH_RETRY_FIRST;
    loop {
        let wait = match deadline {
            Some(deadline) => left(clock.now(), deadline).min(WATCH_WAIT),
            None => WATCH_WAIT,
        };
        let limit = clock.now() + chrono_duration(wait + WATCH_CALL_GRACE)?;
        let answer = tokio::select! {
            answer = server.wait(after, options.channels.as_deref(), wait) => answer,
            () = clock.sleep_until(limit) => Err(Failure::Lost("no answer in time".into())),
        };
        match answer {
            Ok(messages) => {
                pause = WATCH_RETRY_FIRST;
                let before = after;
                let mut shown = false;
                for message in messages.iter().filter(|m| m.id > before) {
                    if message.agent != me {
                        report.message(&line(message))?;
                        shown = true;
                    }
                    after = after.max(message.id);
                }
                if after != before
                    && let Some(place) = place
                {
                    place.save(after)?;
                }
                if options.once && shown {
                    return Ok(Outcome::Received);
                }
            }
            Err(failure) if failure.passing() => {
                report.note(&format!("{failure}; trying again in {}s", pause.as_secs()));
                let resume = clock.now() + chrono_duration(pause)?;
                clock
                    .sleep_until(deadline.map_or(resume, |d| d.min(resume)))
                    .await;
                pause = (pause * 2).min(WATCH_RETRY_MAX);
            }
            Err(failure) => return Err(failure.to_string()),
        }
        if let Some(deadline) = deadline
            && clock.now() >= deadline
        {
            return Ok(Outcome::TimedOut);
        }
    }
}

/// `[id] #channel sender: body`, all on one line: the body's line breaks
/// become ` / `, other control characters spaces, and a long body is cut.
pub fn line(message: &Message) -> String {
    let body = message.body.lines().collect::<Vec<_>>().join(" / ");
    let mut body = one_line(&body);
    if body.chars().count() > WATCH_BODY_CHARS {
        body = body.chars().take(WATCH_BODY_CHARS).collect::<String>() + "…";
    }
    format!(
        "[{}] #{} {}: {}",
        message.id,
        one_line(&message.channel_name),
        one_line(&message.agent),
        body
    )
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn left(now: DateTime<Utc>, deadline: DateTime<Utc>) -> Duration {
    // A deadline already passed leaves nothing, not an error.
    (deadline - now).to_std().unwrap_or(Duration::ZERO)
}

fn chrono_duration(duration: Duration) -> Result<chrono::Duration, String> {
    chrono::Duration::from_std(duration).map_err(|_| "the timeout is too long".into())
}
