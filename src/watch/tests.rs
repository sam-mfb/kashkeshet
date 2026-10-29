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
}

impl<'c> FakeServer<'c> {
    fn new(clock: &'c FixedClock, answers: Vec<Result<Vec<Message>, Failure>>) -> Self {
        FakeServer {
            clock,
            name: Ok(ME.into()),
            latest: 0,
            answers: Mutex::new(answers.into()),
            waits: Mutex::new(Vec::new()),
        }
    }

    fn waits(&self) -> Vec<(i64, Duration)> {
        self.waits.lock().unwrap().clone()
    }
}

impl Server for FakeServer<'_> {
    fn name(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move { self.name.clone() })
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
        Box::pin(async move { Ok(self.latest) })
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
        after_id: None,
        once: true,
        timeout: None,
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
async fn a_given_id_wins_over_the_place() {
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
    assert_eq!(server.waits()[0].0, 10);
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
    // A wait that never ends: the script holds one answer that never comes.
    struct Silent;
    impl Server for Silent {
        fn name(&self) -> BoxFuture<'_, Result<String, Failure>> {
            Box::pin(async { Ok(ME.into()) })
        }
        fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
            Box::pin(async { Ok(0) })
        }
        fn wait<'a>(
            &'a self,
            _: i64,
            _: Option<&'a [String]>,
            _: Duration,
        ) -> BoxFuture<'a, Result<Vec<Message>, Failure>> {
            Box::pin(futures::future::pending())
        }
    }
    let mut report = FakeReport::default();
    let options = Options {
        timeout: Some(Duration::from_secs(100)),
        ..once()
    };
    let limit = (WATCH_WAIT + WATCH_CALL_GRACE).as_secs();
    let (outcome, ()) = tokio::join!(
        run(&Silent, &clock, None, &mut report, &options),
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
    assert_eq!(line(&m), "[9] #talk other: first / second / third [31m");
    m.body = "x".repeat(WATCH_BODY_CHARS + 1);
    assert_eq!(
        line(&m),
        format!("[9] #talk other: {}…", "x".repeat(WATCH_BODY_CHARS))
    );
    m.body = "x".repeat(WATCH_BODY_CHARS);
    assert!(!line(&m).ends_with('…'));
}
