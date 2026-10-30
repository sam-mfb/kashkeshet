use super::*;
use crate::clock::testing::FixedClock;
use chrono::TimeZone;
use std::collections::VecDeque;
use std::sync::Mutex;

const ME: &str = "me";

fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn message(id: i64, agent: &str) -> Message {
    Message {
        id,
        channel_name: "talk".into(),
        agent: agent.into(),
        body: format!("message {id}"),
        cut: false,
    }
}

fn lost() -> Failure {
    Failure::Lost("connection refused".into())
}

/// A server that answers waits from a script. With the script used up, a
/// wait lasts its full time on the clock and returns nothing.
struct FakeServer<'c> {
    clock: &'c FixedClock,
    name: Result<String, Failure>,
    latest: i64,
    answers: Mutex<VecDeque<Result<Vec<Message>, Failure>>>,
    /// Each wait's `after_id` and how long it asked to wait.
    waits: Mutex<Vec<(i64, Duration)>>,
    /// Whole messages, by ID, for `message`.
    whole: Vec<Message>,
    /// Each ID asked for with `message`.
    fetched: Mutex<Vec<i64>>,
}

impl<'c> FakeServer<'c> {
    fn new(clock: &'c FixedClock, answers: Vec<Result<Vec<Message>, Failure>>) -> Self {
        FakeServer {
            clock,
            name: Ok(ME.into()),
            latest: 0,
            answers: Mutex::new(answers.into()),
            waits: Mutex::new(Vec::new()),
            whole: Vec::new(),
            fetched: Mutex::new(Vec::new()),
        }
    }

    fn waits(&self) -> Vec<(i64, Duration)> {
        self.waits.lock().unwrap().clone()
    }
}

impl Server for FakeServer<'_> {
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn name(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move { self.name.clone() })
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
        Box::pin(async move { Ok(self.latest) })
    }

    fn message(&self, id: i64) -> BoxFuture<'_, Result<Message, Failure>> {
        self.fetched.lock().unwrap().push(id);
        let found = self.whole.iter().find(|m| m.id == id).cloned();
        Box::pin(async move { found.ok_or_else(|| Failure::Lost(format!("no message {id}"))) })
    }

    fn wait<'a>(
        &'a self,
        after_id: i64,
        _channels: Option<&'a [String]>,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Vec<Message>, Failure>> {
        self.waits.lock().unwrap().push((after_id, wait));
        let answer = self.answers.lock().unwrap().pop_front();
        Box::pin(async move {
            match answer {
                Some(answer) => answer,
                None => {
                    let end = self.clock.now() + chrono::Duration::from_std(wait).unwrap();
                    self.clock.sleep_until(end).await;
                    Ok(Vec::new())
                }
            }
        })
    }
}

#[derive(Default)]
struct FakePlace {
    id: Mutex<Option<i64>>,
    saves: Mutex<Vec<i64>>,
}

impl Place for FakePlace {
    fn load(&self) -> Result<Option<i64>, String> {
        Ok(*self.id.lock().unwrap())
    }

    fn save(&self, id: i64) -> Result<(), String> {
        *self.id.lock().unwrap() = Some(id);
        self.saves.lock().unwrap().push(id);
        Ok(())
    }
}

#[derive(Default)]
struct FakeReport {
    lines: Vec<String>,
    notes: Vec<String>,
}

impl Report for FakeReport {
    fn message(&mut self, line: &str) -> Result<(), String> {
        self.lines.push(line.into());
        Ok(())
    }

    fn note(&mut self, text: &str) {
        self.notes.push(text.into());
    }
}

fn once() -> Options {
    Options {
        channels: None,
        channel_names: Vec::new(),
        mentions: false,
        after_id: None,
        once: true,
        timeout: None,
        full: false,
    }
}

/// A server that never answers the calls named, and answers the others
/// at once with nothing new.
struct Stalled {
    name: bool,
    latest_id: bool,
    wait: bool,
}

impl Stalled {
    const WAIT: Stalled = Stalled {
        name: false,
        latest_id: false,
        wait: true,
    };
}

fn answer_unless<T: Send + 'static>(stalled: bool, answer: T) -> BoxFuture<'static, T> {
    if stalled {
        Box::pin(futures::future::pending())
    } else {
        Box::pin(async move { answer })
    }
}

impl Server for Stalled {
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        answer_unless(false, Ok(Vec::new()))
    }
    fn name(&self) -> BoxFuture<'_, Result<String, Failure>> {
        answer_unless(self.name, Ok(ME.into()))
    }
    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
        answer_unless(self.latest_id, Ok(0))
    }
    fn message(&self, id: i64) -> BoxFuture<'_, Result<Message, Failure>> {
        answer_unless(false, Ok(message(id, "other")))
    }
    fn wait<'a>(
        &'a self,
        _: i64,
        _: Option<&'a [String]>,
        _: Duration,
    ) -> BoxFuture<'a, Result<Vec<Message>, Failure>> {
        answer_unless(self.wait, Ok(Vec::new()))
    }
}

/// Moves `clock` forward by each step in turn, each once something sleeps
/// on it beyond `busy` sleeps that are always there.
async fn advance(clock: &FixedClock, busy: usize, steps: Vec<u64>) {
    for step in steps {
        while clock.sleepers() <= busy {
            tokio::task::yield_now().await;
        }
        clock.advance(chrono::Duration::seconds(step as i64));
        // Let the watch run, so the sleep that just finished is dropped
        // before the sleepers are counted again.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
}

#[tokio::test]
async fn a_lost_connection_is_retried_with_a_doubling_pause() {
    let clock = FixedClock::new(start());
    let mut answers: Vec<_> = (0..7).map(|_| Err(lost())).collect();
    answers.push(Ok(vec![message(5, "other")]));
    let server = FakeServer::new(&clock, answers);
    let mut report = FakeReport::default();
    let pauses = [2, 4, 8, 16, 32, 60, 60];
    let options = once();
    let (outcome, ()) = tokio::join!(
        run(&server, &clock, None, &mut report, &options),
        advance(&clock, 0, pauses.to_vec()),
    );
    assert_eq!(outcome, Ok(Outcome::Received));
    assert_eq!(report.lines, vec!["[5] #talk other: message 5"]);
    let said: Vec<_> = report.notes.iter().skip(1).cloned().collect();
    let expected: Vec<_> = pauses
        .iter()
        .map(|s| format!("could not reach the server: connection refused; trying again in {s}s"))
        .collect();
    assert_eq!(said, expected);
}

#[tokio::test]
async fn a_busy_server_is_retried_but_a_refusal_ends_the_watch() {
    let clock = FixedClock::new(start());
    let refused = |code: &str| Failure::Refused {
        code: code.into(),
        message: "no".into(),
    };
    let server = FakeServer::new(
        &clock,
        vec![
            Err(refused("rate_limited")),
            Err(refused("unauthenticated")),
        ],
    );
    let mut report = FakeReport::default();
    let options = once();
    let (outcome, ()) = tokio::join!(
        run(&server, &clock, None, &mut report, &options),
        advance(&clock, 0, vec![2]),
    );
    assert_eq!(outcome, Err("unauthenticated: no".into()));
    assert_eq!(server.waits().len(), 2);
}

#[tokio::test]
async fn failing_before_watching_starts_ends_the_watch() {
    let clock = FixedClock::new(start());
    let mut server = FakeServer::new(&clock, vec![]);
    server.name = Err(lost());
    let outcome = run(&server, &clock, None, &mut FakeReport::default(), &once()).await;
    assert_eq!(
        outcome,
        Err("could not reach the server: connection refused".into())
    );
    assert!(server.waits().is_empty());
}

#[tokio::test]
async fn own_messages_are_skipped_but_still_move_the_place() {
    let clock = FixedClock::new(start());
    let server = FakeServer::new(
        &clock,
        vec![
            Ok(vec![message(5, ME)]),
            Ok(vec![message(6, ME), message(7, "other")]),
        ],
    );
    let place = FakePlace::default();
    *place.id.lock().unwrap() = Some(4);
    let mut report = FakeReport::default();
    let outcome = run(&server, &clock, Some(&place), &mut report, &once()).await;
    assert_eq!(outcome, Ok(Outcome::Received));
    assert_eq!(report.lines, vec!["[7] #talk other: message 7"]);
    assert_eq!(*place.saves.lock().unwrap(), vec![5, 7]);
    let afters: Vec<i64> = server.waits().iter().map(|(after, _)| *after).collect();
    assert_eq!(afters, vec![4, 5]);
}

#[tokio::test]
async fn without_a_place_or_id_it_starts_after_the_latest_and_saves_it() {
    let clock = FixedClock::new(start());
    let mut server = FakeServer::new(&clock, vec![Ok(vec![message(43, "other")])]);
    server.latest = 42;
    let place = FakePlace::default();
    let mut report = FakeReport::default();
    run(&server, &clock, Some(&place), &mut report, &once())
        .await
        .unwrap();
    assert_eq!(server.waits()[0].0, 42);
    assert_eq!(*place.saves.lock().unwrap(), vec![42, 43]);
}

#[tokio::test]
async fn a_given_id_seeds_an_empty_place() {
    let clock = FixedClock::new(start());
    let mut server = FakeServer::new(&clock, vec![Ok(vec![message(11, "other")])]);
    server.latest = 42;
    let place = FakePlace::default();
    let options = Options {
        after_id: Some(10),
        ..once()
    };
    run(
        &server,
        &clock,
        Some(&place),
        &mut FakeReport::default(),
        &options,
    )
    .await
    .unwrap();
    assert_eq!(server.waits()[0].0, 10);
    assert_eq!(*place.saves.lock().unwrap(), vec![10, 11]);
}

#[tokio::test]
async fn an_id_in_the_place_wins_over_a_given_id() {
    let clock = FixedClock::new(start());
    let server = FakeServer::new(&clock, vec![Ok(vec![message(11, "other")])]);
    let place = FakePlace::default();
    *place.id.lock().unwrap() = Some(3);
    let options = Options {
        after_id: Some(10),
        ..once()
    };
    run(
        &server,
        &clock,
        Some(&place),
        &mut FakeReport::default(),
        &options,
    )
    .await
    .unwrap();
    assert_eq!(server.waits()[0].0, 3);
    assert_eq!(*place.saves.lock().unwrap(), vec![11]);
}

#[tokio::test]
async fn the_timeout_shortens_the_wait_and_ends_with_nothing() {
    let clock = FixedClock::new(start());
    let server = FakeServer::new(&clock, vec![]);
    let options = Options {
        timeout: Some(Duration::from_secs(10)),
        ..once()
    };
    let mut report = FakeReport::default();
    // The fake's wait and the call's own limit both sleep.
    let (outcome, ()) = tokio::join!(
        run(&server, &clock, None, &mut report, &options),
        advance(&clock, 1, vec![10]),
    );
    assert_eq!(outcome, Ok(Outcome::TimedOut));
    assert_eq!(server.waits(), vec![(0, Duration::from_secs(10))]);
    assert!(report.lines.is_empty());
}

#[tokio::test]
async fn a_retry_pause_stops_at_the_timeout() {
    let clock = FixedClock::new(start());
    let server = FakeServer::new(&clock, vec![Err(lost())]);
    let options = Options {
        timeout: Some(Duration::from_secs(1)),
        ..once()
    };
    let mut report = FakeReport::default();
    let (outcome, ()) = tokio::join!(
        run(&server, &clock, None, &mut report, &options),
        advance(&clock, 0, vec![1]),
    );
    assert_eq!(outcome, Ok(Outcome::TimedOut));
}

#[tokio::test]
async fn a_wait_with_no_answer_counts_as_lost() {
    let clock = FixedClock::new(start());
    let mut report = FakeReport::default();
    let options = Options {
        timeout: Some(Duration::from_secs(100)),
        ..once()
    };
    let limit = (WATCH_WAIT + WATCH_CALL_GRACE).as_secs();
    let stalled = Stalled::WAIT;
    let (outcome, ()) = tokio::join!(
        run(&stalled, &clock, None, &mut report, &options),
        advance(&clock, 0, vec![limit, 2, 100]),
    );
    assert_eq!(outcome, Ok(Outcome::TimedOut));
    assert!(
        report.notes[1].contains("no answer in time"),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_line_is_one_line_with_a_cut_body() {
    let mut m = message(9, "other");
    m.body = "first\r\nsecond\nthird\x1b[31m".into();
    assert_eq!(
        line(&m, false),
        "[9] #talk other: first / second / third [31m"
    );
    m.body = "x".repeat(WATCH_BODY_CHARS + 1);
    assert_eq!(
        line(&m, false),
        format!("[9] #talk other: {}…", "x".repeat(WATCH_BODY_CHARS))
    );
    m.body = "x".repeat(WATCH_BODY_CHARS);
    assert!(!line(&m, false).ends_with('…'));
    m.body = "x".repeat(WATCH_BODY_CHARS + 1);
    assert_eq!(
        line(&m, true),
        format!("[9] #talk other: {}", "x".repeat(WATCH_BODY_CHARS + 1))
    );
}

#[tokio::test]
async fn the_timeout_ends_a_start_with_no_answer() {
    for stalled in [
        Stalled {
            name: true,
            latest_id: false,
            wait: false,
        },
        Stalled {
            name: false,
            latest_id: true,
            wait: false,
        },
    ] {
        let clock = FixedClock::new(start());
        let options = Options {
            timeout: Some(Duration::from_secs(1)),
            ..once()
        };
        let mut report = FakeReport::default();
        let (outcome, ()) = tokio::join!(
            run(&stalled, &clock, None, &mut report, &options),
            advance(&clock, 0, vec![1]),
        );
        assert_eq!(
            outcome,
            Err("no answer from the server before the timeout".into())
        );
        assert_eq!(clock.now(), start() + chrono::Duration::seconds(1));
    }
}

#[tokio::test]
async fn without_a_timeout_a_start_with_no_answer_ends_after_the_grace() {
    let clock = FixedClock::new(start());
    let stalled = Stalled {
        name: true,
        latest_id: false,
        wait: false,
    };
    let mut report = FakeReport::default();
    let options = once();
    let (outcome, ()) = tokio::join!(
        run(&stalled, &clock, None, &mut report, &options),
        advance(&clock, 0, vec![WATCH_CALL_GRACE.as_secs()]),
    );
    assert_eq!(
        outcome,
        Err("could not reach the server: no answer in time".into())
    );
}

#[tokio::test]
async fn the_timeout_ends_a_wait_with_no_answer_at_the_deadline() {
    let clock = FixedClock::new(start());
    let options = Options {
        timeout: Some(Duration::from_secs(1)),
        ..once()
    };
    let mut report = FakeReport::default();
    let stalled = Stalled::WAIT;
    let (outcome, ()) = tokio::join!(
        run(&stalled, &clock, None, &mut report, &options),
        advance(&clock, 0, vec![1]),
    );
    assert_eq!(outcome, Ok(Outcome::TimedOut));
    assert_eq!(clock.now(), start() + chrono::Duration::seconds(1));
}

#[tokio::test]
async fn a_timeout_past_the_last_date_is_an_error() {
    let clock = FixedClock::new(start());
    let server = FakeServer::new(&clock, vec![]);
    let options = Options {
        timeout: Some(Duration::from_secs(1_000_000_000_000_000)),
        ..once()
    };
    let outcome = run(&server, &clock, None, &mut FakeReport::default(), &options).await;
    assert_eq!(
        outcome,
        Err("the timeout of 1000000000000000 seconds is too long".into())
    );
}

#[tokio::test]
async fn full_fetches_bodies_the_server_cut_but_not_its_own() {
    let clock = FixedClock::new(start());
    let long = "y".repeat(WATCH_BODY_CHARS * 2);
    let cut = |id, agent: &str| Message {
        body: "y".repeat(10),
        cut: true,
        ..message(id, agent)
    };
    let mut server = FakeServer::new(
        &clock,
        vec![Ok(vec![cut(5, ME), cut(6, "other"), message(7, "other")])],
    );
    server.whole = vec![Message {
        body: long.clone(),
        ..message(6, "other")
    }];
    let options = Options {
        full: true,
        ..once()
    };
    let mut report = FakeReport::default();
    let outcome = run(&server, &clock, None, &mut report, &options).await;
    assert_eq!(outcome, Ok(Outcome::Received));
    assert_eq!(*server.fetched.lock().unwrap(), vec![6]);
    assert_eq!(
        report.lines,
        vec![
            format!("[6] #talk other: {long}"),
            "[7] #talk other: message 7".into()
        ]
    );
}

#[tokio::test]
async fn without_full_nothing_is_fetched() {
    let clock = FixedClock::new(start());
    let server = FakeServer::new(
        &clock,
        vec![Ok(vec![Message {
            cut: true,
            ..message(6, "other")
        }])],
    );
    let mut report = FakeReport::default();
    run(&server, &clock, None, &mut report, &once())
        .await
        .unwrap();
    assert!(server.fetched.lock().unwrap().is_empty());
    assert_eq!(report.lines, vec!["[6] #talk other: message 6"]);
}

#[test]
fn mentions_require_the_whole_agent_name() {
    let name = "steady-robin";
    for body in [
        "steady-robin",
        "hello @steady-robin!",
        "(steady-robin), please reply",
        "first\nsteady-robin\nlast",
        "unsteady-robin then steady-robin",
    ] {
        assert!(filters::mentions(body, name), "{body}");
    }
    for body in [
        "no mention",
        "unsteady-robin",
        "steady-robins",
        "steady-robin-helper",
        "steady-robin_2",
        "ésteady-robin",
        "steady-robin字",
        "STEADY-ROBIN",
    ] {
        assert!(!filters::mentions(body, name), "{body}");
    }
}

#[tokio::test]
async fn mentions_skip_messages_advance_the_place_and_fetch_cut_bodies() {
    let clock = FixedClock::new(start());
    let mut server = FakeServer::new(
        &clock,
        vec![
            Ok(vec![message(5, "other")]),
            Ok(vec![Message {
                cut: true,
                ..message(6, "other")
            }]),
            Ok(vec![
                Message {
                    body: "@me: hello".into(),
                    ..message(7, "other")
                },
                message(8, "other"),
            ]),
        ],
    );
    server.whole = vec![message(6, "other")];
    let place = FakePlace::default();
    let mut report = FakeReport::default();
    let options = Options {
        mentions: true,
        ..once()
    };
    let outcome = run(&server, &clock, Some(&place), &mut report, &options).await;
    assert_eq!(outcome, Ok(Outcome::Received));
    assert_eq!(report.lines, vec!["[7] #talk other: @me: hello"]);
    assert_eq!(*place.saves.lock().unwrap(), vec![0, 5, 6, 8]);
    assert_eq!(*server.fetched.lock().unwrap(), vec![6]);
    assert_eq!(
        server.waits().iter().map(|w| w.0).collect::<Vec<_>>(),
        vec![0, 5, 6]
    );
}
