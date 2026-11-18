//! `kkt watch`: wait for one agent's new messages from the shell, with no
//! model running. It prints one line per new message and keeps going, or,
//! with `once`, stops after the first new messages. When the agent's lane
//! has a newer version, it says so once, apart from the message lines.
//!
//! What it depends on is injected: the server, as kkt-agent-client's
//! `Watched` contract, the time (`Clock`), where it keeps its place
//! (`Place`) and where its output goes (`Report`).

mod error;
mod filters;
#[cfg(test)]
mod tests;

use futures::future::BoxFuture;
use kkt_agent_client::{Channel, Failure, Message, NewerLane, Waited, Watched};
use kkt_clock::{Clock, Span, Time};
use std::time::Duration;

pub use error::{PlaceError, ReportError, WatchError};

/// The most characters of a message body on its line.
const WATCH_BODY_CHARS: usize = 300;

/// How long the watch's calls wait, and how it retries. The program passes
/// [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How long each `wait_for_messages` call asks to wait. The server caps
    /// it at its own `max_wait`.
    pub watch_wait: Duration,
    /// How long past its wait a call may take before it counts as lost.
    pub watch_call_grace: Duration,
    /// The first pause after a lost connection; it doubles up to
    /// `watch_retry_max`.
    pub watch_retry_first: Duration,
    /// The longest pause after a lost connection.
    pub watch_retry_max: Duration,
    /// How often a server whose waits answer at once ([`Watched::polls`],
    /// the admin endpoint's) is asked again while nothing new comes.
    pub watch_poll: Duration,
}

impl Settings {
    /// Waits of 50 seconds with 30 seconds' grace; pauses from 2 seconds,
    /// doubling up to a minute; a server that answers at once asked every
    /// 2 seconds.
    pub const DEFAULT: Settings = Settings {
        watch_wait: Duration::from_secs(50),
        watch_call_grace: Duration::from_secs(30),
        watch_retry_first: Duration::from_secs(2),
        watch_retry_max: Duration::from_secs(60),
        watch_poll: Duration::from_secs(2),
    };
}

/// Whether trying again later may work. A lost connection or a busy
/// server may recover; a bad token, a refused key or a missing channel will
/// not.
fn passing(failure: &Failure) -> bool {
    match failure {
        Failure::Lost(_) => true,
        Failure::Refused { code, .. } => matches!(code.as_str(), "rate_limited" | "internal"),
        Failure::KeyRefused { .. } => false,
    }
}

/// Where the highest message ID seen is kept between runs.
pub trait Place {
    /// The kept ID, or `None` if none is kept yet.
    fn load(&self) -> Result<Option<i64>, PlaceError>;
    fn save(&self, id: i64) -> Result<(), PlaceError>;
}

/// Where the output goes.
pub trait Report {
    /// One line for one new message. Nothing else is reported this way.
    fn message(&mut self, line: &str) -> Result<(), ReportError>;
    /// Anything else: where watching starts, lost connections, a newer
    /// version of the agent's lane.
    fn note(&mut self, text: &str);
}

pub struct Options {
    /// Only these channels; `None` for every channel the agent is in.
    pub channels: Option<Vec<String>>,
    /// Exact channel names, resolved once before watching starts.
    pub channel_names: Vec<String>,
    /// Print only messages naming this agent as a whole word.
    pub mentions: bool,
    /// Start after this ID instead of the latest. With a place, it only
    /// seeds a place that holds no ID; an ID already kept there wins.
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
/// agent's handle and start) end the watch, so a wrong address, key or token
/// is reported at once. After that, a lost connection is retried with a
/// growing pause and never ends the watch; only an error that cannot pass,
/// such as a revoked token or a refused key, does. The agent's own messages
/// are skipped.
///
/// A newer version of the agent's lane is noted once for each version: it
/// is not a message, so it neither moves the place nor ends a `once` watch.
///
/// The timeout covers every call. If it runs out before watching starts,
/// that is an error, since nothing has been checked; after, it is
/// `TimedOut`. Each call made with no sooner deadline may take its wait
/// plus `settings.watch_call_grace` before it counts as lost.
pub async fn run(
    server: &dyn Watched,
    clock: &dyn Clock,
    settings: Settings,
    place: Option<&dyn Place>,
    report: &mut dyn Report,
    options: &Options,
) -> Result<Outcome, WatchError> {
    let deadline = match options.timeout {
        Some(timeout) => Some(
            later(clock.now(), timeout).ok_or(WatchError::TimeoutTooLong {
                seconds: timeout.as_secs(),
            })?,
        ),
        None => None,
    };
    let me = starting(clock, settings, deadline, server.handle()).await?;
    let visible = if options.channel_names.is_empty() {
        Vec::new()
    } else {
        starting(clock, settings, deadline, server.channels()).await?
    };
    let channels = filters::channels(
        options.channels.as_deref(),
        &options.channel_names,
        &visible,
    )?;
    let saved = match place {
        Some(place) => place.load().map_err(WatchError::Place)?,
        None => None,
    };
    let mut after = match saved {
        Some(id) => id,
        None => {
            let id = match options.after_id {
                Some(id) => id,
                None => starting(clock, settings, deadline, server.latest_id()).await?,
            };
            if let Some(place) = place {
                place.save(id).map_err(WatchError::Place)?;
            }
            id
        }
    };
    report.note(&format!("watching as {me}, after message {after}"));
    let mut pause = settings.watch_retry_first;
    let mut noted_lane: Option<NewerLane> = None;
    loop {
        let wait = match deadline {
            Some(deadline) => left(clock.now(), deadline).min(settings.watch_wait),
            None => settings.watch_wait,
        };
        let grace = later(clock.now(), wait + settings.watch_call_grace).ok_or(OUT_OF_RANGE)?;
        let limit = deadline.map_or(grace, |d| d.min(grace));
        let answer = tokio::select! {
            answer = fetch(server, clock, settings, &me, after, options, channels.as_deref(), wait) => answer,
            () = clock.sleep_until(limit) => Err(Failure::Lost("no answer in time".into())),
        };
        match answer {
            Ok(waited) => {
                pause = settings.watch_retry_first;
                let before = after;
                let mut shown = false;
                for message in waited.messages.iter().filter(|m| m.id > before) {
                    if message.agent != me
                        && (!options.mentions || filters::mentions(&message.body, &me))
                    {
                        report
                            .message(&line(message, options.full))
                            .map_err(WatchError::Report)?;
                        shown = true;
                    }
                    after = after.max(message.id);
                }
                if after != before
                    && let Some(place) = place
                {
                    place.save(after).map_err(WatchError::Place)?;
                }
                if let Some(lane) = waited.newer_lane
                    && noted_lane.as_ref() != Some(&lane)
                {
                    report.note(&lane_note(&lane));
                    noted_lane = Some(lane);
                }
                if options.once && shown {
                    return Ok(Outcome::Received);
                }
            }
            Err(failure) if passing(&failure) && deadline.is_some_and(|d| clock.now() >= d) => {
                return Ok(Outcome::TimedOut);
            }
            Err(failure) if passing(&failure) => {
                report.note(&format!("{failure}; trying again in {}s", pause.as_secs()));
                let resume = later(clock.now(), pause).ok_or(OUT_OF_RANGE)?;
                clock
                    .sleep_until(deadline.map_or(resume, |d| d.min(resume)))
                    .await;
                pause = (pause * 2).min(settings.watch_retry_max);
            }
            Err(failure) => return Err(WatchError::Server(failure)),
        }
        if let Some(deadline) = deadline
            && clock.now() >= deadline
        {
            return Ok(Outcome::TimedOut);
        }
    }
}

const OUT_OF_RANGE: WatchError = WatchError::ClockOutOfRange;

/// A call made before watching starts. It may take
/// `settings.watch_call_grace`, or until the deadline if that is sooner.
async fn starting<T>(
    clock: &dyn Clock,
    settings: Settings,
    deadline: Option<Time>,
    call: BoxFuture<'_, Result<T, Failure>>,
) -> Result<T, WatchError> {
    let grace = later(clock.now(), settings.watch_call_grace).ok_or(OUT_OF_RANGE)?;
    let limit = deadline.map_or(grace, |d| d.min(grace));
    tokio::select! {
        answer = call => answer.map_err(WatchError::Server),
        () = clock.sleep_until(limit) => Err(if limit == grace {
            WatchError::Server(Failure::Lost("no answer in time".into()))
        } else {
            WatchError::NoAnswerBeforeTimeout
        }),
    }
}

/// Messages after `after`, waiting up to `wait`: on the server, or, for a
/// server that answers at once, by asking again every
/// `settings.watch_poll` until something comes or the wait is over. Whole
/// bodies are needed for `full` and for finding mentions beyond the
/// server's preview. The agent's own messages are not shown, so they are
/// not fetched.
#[expect(
    clippy::too_many_arguments,
    reason = "the loop's state, passed as it is; a struct of them would only rename it"
)]
async fn fetch(
    server: &dyn Watched,
    clock: &dyn Clock,
    settings: Settings,
    me: &str,
    after: i64,
    options: &Options,
    channels: Option<&[String]>,
    wait: Duration,
) -> Result<Waited, Failure> {
    let mut waited = if server.polls() {
        let lost = || Failure::Lost(OUT_OF_RANGE.to_string());
        let end = later(clock.now(), wait).ok_or_else(lost)?;
        let poll = Span::from_std(settings.watch_poll).map_err(|_| lost())?;
        // Asked only before the end: once the next ask would fall at or
        // after it, the wait sleeps to its end and gives what it has, so
        // the end of the wait wins over a last ask. The next wait asks at
        // once.
        loop {
            let waited = server.wait(after, channels, wait).await?;
            let next = clock.now() + poll;
            if !waited.messages.is_empty() || next >= end {
                if waited.messages.is_empty() {
                    clock.sleep_until(end).await;
                }
                break waited;
            }
            clock.sleep_until(next).await;
        }
    } else {
        server.wait(after, channels, wait).await?
    };
    if options.full || options.mentions {
        for message in waited
            .messages
            .iter_mut()
            .filter(|m| m.cut && m.agent != me)
        {
            *message = server.message(message.id).await?;
        }
    }
    Ok(waited)
}

/// What the watch says when the agent's lane has a newer version.
fn lane_note(lane: &NewerLane) -> String {
    format!(
        "your lane {} has a newer version, {} (you hold version {}); \
         `take_lane` with lane=`{}` moves to it",
        one_line(&lane.name),
        lane.current_version,
        lane.version,
        one_line(&lane.name)
    )
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

fn left(now: Time, deadline: Time) -> Duration {
    // A deadline already passed leaves nothing, not an error.
    (deadline - now).to_std().unwrap_or(Duration::ZERO)
}

/// `duration` after `now`, or `None` if that is past the dates we can hold.
fn later(now: Time, duration: Duration) -> Option<Time> {
    now.checked_add(Span::from_std(duration).ok()?)
}
