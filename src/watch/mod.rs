//! `kkt watch`: wait for one agent's new messages from the shell, with no
//! model running. It prints one line per new message and keeps going, or,
//! with `once`, stops after the first new messages.
//!
//! What it depends on is injected: the server (`Server`, over MCP in
//! `mcp_client`), the time (`Clock`), where it keeps its place (`Place`) and
//! where its output goes (`Report`).

mod filters;
#[cfg(test)]
mod tests;

pub use crate::mcp_client::{Failure, Remote};

use crate::clock::Clock;
use crate::config::{
    WATCH_BODY_CHARS, WATCH_CALL_GRACE, WATCH_RETRY_FIRST, WATCH_RETRY_MAX, WATCH_WAIT,
};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use std::time::Duration;

/// A new message, as `kkt watch` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
    /// Whether the server cut the body to fit its answer.
    pub cut: bool,
}

/// A visible channel, for resolving names before watching starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
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

/// The server, acting as one agent.
pub trait Server: Send + Sync {
    /// The agent's name.
    fn name(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// All visible channels, including every page of the server's list.
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>>;
    /// The highest message ID in the agent's channels, or 0 if none.
    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>>;
    /// One message, with its whole body.
    fn message(&self, id: i64) -> BoxFuture<'_, Result<Message, Failure>>;
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
    /// Exact channel names, resolved once before watching starts.
    pub channel_names: Vec<String>,
    /// Print only messages naming this agent as a whole word.
    pub mentions: bool,
    /// Start after this ID instead of the place's or the latest.
    pub after_id: Option<i64>,
    /// Stop after the first new messages.
    pub once: bool,
    /// With `once`, stop after this long even if nothing came. It bounds
    /// everything, including finding where to start.
    pub timeout: Option<Duration>,
    /// Print whole bodies: fetch any the server cut, and cut none here.
    pub full: bool,
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
///
/// The timeout covers every call. If it runs out before watching starts,
/// that is an error, since nothing has been checked; after, it is
/// `TimedOut`. Each call made with no sooner deadline may take its wait
/// plus `WATCH_CALL_GRACE` before it counts as lost.
pub async fn run(
    server: &dyn Server,
    clock: &dyn Clock,
    place: Option<&dyn Place>,
    report: &mut dyn Report,
    options: &Options,
) -> Result<Outcome, String> {
    let deadline =
        match options.timeout {
            Some(timeout) => Some(later(clock.now(), timeout).ok_or_else(|| {
                format!("the timeout of {} seconds is too long", timeout.as_secs())
            })?),
            None => None,
        };
    let me = starting(clock, deadline, server.name()).await?;
    let visible = if options.channel_names.is_empty() {
        Vec::new()
    } else {
        starting(clock, deadline, server.channels()).await?
    };
    let channels = filters::channels(
        options.channels.as_deref(),
        &options.channel_names,
        &visible,
    )?;
    let saved = match (options.after_id, place) {
        (Some(_), _) | (None, None) => None,
        (None, Some(place)) => place.load()?,
    };
    let mut after = match options.after_id.or(saved) {
        Some(id) => id,
        None => {
            let id = starting(clock, deadline, server.latest_id()).await?;
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
        let grace = later(clock.now(), wait + WATCH_CALL_GRACE).ok_or(OUT_OF_RANGE)?;
        let limit = deadline.map_or(grace, |d| d.min(grace));
        let answer = tokio::select! {
            answer = fetch(server, &me, after, options, channels.as_deref(), wait) => answer,
            () = clock.sleep_until(limit) => Err(Failure::Lost("no answer in time".into())),
        };
        match answer {
            Ok(messages) => {
                pause = WATCH_RETRY_FIRST;
                let before = after;
                let mut shown = false;
                for message in messages.iter().filter(|m| m.id > before) {
                    if message.agent != me
                        && (!options.mentions || filters::mentions(&message.body, &me))
                    {
                        report.message(&line(message, options.full))?;
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
            Err(failure) if failure.passing() && deadline.is_some_and(|d| clock.now() >= d) => {
                return Ok(Outcome::TimedOut);
            }
            Err(failure) if failure.passing() => {
                report.note(&format!("{failure}; trying again in {}s", pause.as_secs()));
                let resume = later(clock.now(), pause).ok_or(OUT_OF_RANGE)?;
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

const OUT_OF_RANGE: &str = "the clock is out of range";

/// A call made before watching starts. It may take `WATCH_CALL_GRACE`, or
/// until the deadline if that is sooner.
async fn starting<T>(
    clock: &dyn Clock,
    deadline: Option<DateTime<Utc>>,
    call: BoxFuture<'_, Result<T, Failure>>,
) -> Result<T, String> {
    let grace = later(clock.now(), WATCH_CALL_GRACE).ok_or(OUT_OF_RANGE)?;
    let limit = deadline.map_or(grace, |d| d.min(grace));
    tokio::select! {
        answer = call => answer.map_err(|e| e.to_string()),
        () = clock.sleep_until(limit) => Err(if limit == grace {
            Failure::Lost("no answer in time".into()).to_string()
        } else {
            "no answer from the server before the timeout".into()
        }),
    }
}

/// Messages after `after`, waiting up to `wait`. Whole bodies are needed
/// for `full` and for finding mentions beyond the server's preview. The
/// agent's own messages are not shown, so they are not fetched.
async fn fetch(
    server: &dyn Server,
    me: &str,
    after: i64,
    options: &Options,
    channels: Option<&[String]>,
    wait: Duration,
) -> Result<Vec<Message>, Failure> {
    let mut messages = server.wait(after, channels, wait).await?;
    if options.full || options.mentions {
        for message in messages.iter_mut().filter(|m| m.cut && m.agent != me) {
            *message = server.message(message.id).await?;
        }
    }
    Ok(messages)
}

/// `[id] #channel sender: body`, all on one line: the body's line breaks
/// become ` / ` and other control characters spaces. Unless `full`, a long
/// body is cut.
pub fn line(message: &Message, full: bool) -> String {
    let body = message.body.lines().collect::<Vec<_>>().join(" / ");
    let mut body = one_line(&body);
    if !full && body.chars().count() > WATCH_BODY_CHARS {
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

/// `duration` after `now`, or `None` if that is past the dates we can hold.
fn later(now: DateTime<Utc>, duration: Duration) -> Option<DateTime<Utc>> {
    now.checked_add_signed(chrono::Duration::from_std(duration).ok()?)
}
