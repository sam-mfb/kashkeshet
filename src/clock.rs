//! The current time, injected so tests can control it.

use crate::config::CLOCK_RECHECK;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::future::BoxFuture;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;

    /// Finishes once this clock reaches `deadline`. A deadline already passed
    /// finishes at once.
    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'_, ()>;
}

/// The real system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'_, ()> {
        Box::pin(sleep_by_wall_clock(Utc::now, deadline, CLOCK_RECHECK))
    }
}

/// Finishes once `now` reaches `deadline`, looking at `now` at least every
/// `step`. tokio's timers count only time the computer is awake, so a single
/// timer for the whole wait would end late by however long the computer
/// slept; checking the wall clock between short timers keeps the deadline.
async fn sleep_by_wall_clock(
    now: impl Fn() -> DateTime<Utc>,
    deadline: DateTime<Utc>,
    step: std::time::Duration,
) {
    // A deadline already passed cannot be converted, so it ends the wait.
    while let Ok(remaining) = (deadline - now()).to_std() {
        if remaining.is_zero() {
            return;
        }
        tokio::time::sleep(remaining.min(step)).await;
    }
}

/// The text form used for every stored time, e.g. `2026-10-23T14:02:11Z`.
/// Times in this form sort correctly as strings.
pub fn to_text(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parses a stored time. Returns `None` if the text is not a stored time.
pub fn from_text(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The start of the stretch of `every` that a stored time falls in, as a
/// stored time: with ten seconds, `14:02:17` becomes `14:02:10`. Stretches
/// are counted from the Unix epoch. `None` if the text is not a stored time.
pub fn floor_text(text: &str, every: std::time::Duration) -> Option<String> {
    let seconds = i64::try_from(every.as_secs()).ok()?.max(1);
    let at = from_text(text)?.timestamp();
    DateTime::from_timestamp(at - at.rem_euclid(seconds), 0).map(to_text)
}

#[cfg(test)]
pub mod testing {
    use super::Clock;
    use chrono::{DateTime, Duration, Utc};
    use futures::future::BoxFuture;
    use tokio::sync::watch;

    /// A clock that only moves when told to. Sleeps finish when `advance`
    /// moves it past their deadline.
    pub struct FixedClock(watch::Sender<DateTime<Utc>>);

    impl FixedClock {
        pub fn new(start: DateTime<Utc>) -> Self {
            Self(watch::Sender::new(start))
        }

        pub fn advance(&self, by: Duration) {
            self.0.send_modify(|now| *now += by);
        }

        /// How many sleeps are waiting on this clock. Tests use it to know a
        /// sleep has started before they move the clock.
        pub fn sleepers(&self) -> usize {
            self.0.receiver_count()
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            *self.0.borrow()
        }

        fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'_, ()> {
            // Subscribe now, not on first poll, so `sleepers` counts this sleep
            // as soon as it is created.
            let mut now = self.0.subscribe();
            Box::pin(async move {
                // `wait_for` fails only if the sender is dropped, and the
                // sender lives as long as `self`, which this future borrows.
                now.wait_for(|t| *t >= deadline)
                    .await
                    .expect("clock outlives its sleeps");
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FixedClock;
    use super::*;
    use chrono::{Duration, TimeZone};
    use futures::FutureExt;

    #[test]
    fn times_floor_to_the_start_of_their_stretch() {
        let ten = std::time::Duration::from_secs(10);
        assert_eq!(
            floor_text("2026-10-23T14:02:17Z", ten).as_deref(),
            Some("2026-10-23T14:02:10Z")
        );
        assert_eq!(
            floor_text("2026-10-23T14:02:10Z", ten).as_deref(),
            Some("2026-10-23T14:02:10Z")
        );
        assert_eq!(
            floor_text("2026-10-23T14:02:17Z", std::time::Duration::from_secs(60)).as_deref(),
            Some("2026-10-23T14:02:00Z")
        );
        assert_eq!(floor_text("not a time", ten), None);
    }

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn fixed_clock_sleep_finishes_only_once_advanced_to_the_deadline() {
        let clock = FixedClock::new(start());
        let mut sleep = clock.sleep_until(start() + Duration::seconds(10));
        assert_eq!(clock.sleepers(), 1);
        assert!((&mut sleep).now_or_never().is_none());
        clock.advance(Duration::seconds(9));
        assert!((&mut sleep).now_or_never().is_none());
        clock.advance(Duration::seconds(1));
        assert!(sleep.now_or_never().is_some());
        assert_eq!(clock.sleepers(), 0);
    }

    #[tokio::test]
    async fn fixed_clock_sleep_to_a_past_deadline_finishes_at_once() {
        let clock = FixedClock::new(start());
        let sleep = clock.sleep_until(start() - Duration::seconds(1));
        assert!(sleep.now_or_never().is_some());
    }

    #[tokio::test]
    async fn a_sleep_keeps_its_deadline_when_the_computer_sleeps() {
        // While the computer sleeps the wall clock moves and tokio's timers
        // do not. Here the wall clock jumps past the deadline, as on waking,
        // and the sleep ends within a step, not after the whole wait.
        let wall = std::sync::Mutex::new(start());
        let step = std::time::Duration::from_millis(10);
        let mut sleep = Box::pin(sleep_by_wall_clock(
            || *wall.lock().unwrap(),
            start() + Duration::seconds(80),
            step,
        ));
        assert!((&mut sleep).now_or_never().is_none());
        *wall.lock().unwrap() += Duration::seconds(600);
        tokio::time::timeout(std::time::Duration::from_secs(5), sleep)
            .await
            .expect("the sleep should see the wall clock pass its deadline");
    }

    #[tokio::test]
    async fn a_sleep_in_steps_still_lasts_until_its_deadline() {
        let begun = std::time::Instant::now();
        let wall = || start() + Duration::from_std(begun.elapsed()).unwrap();
        let step = std::time::Duration::from_millis(10);
        sleep_by_wall_clock(wall, start() + Duration::milliseconds(100), step).await;
        assert!(begun.elapsed() >= std::time::Duration::from_millis(100));
    }

    #[tokio::test]
    async fn system_clock_sleep_to_a_past_deadline_finishes_at_once() {
        let clock = SystemClock;
        let sleep = clock.sleep_until(Utc::now() - Duration::hours(1));
        tokio::time::timeout(std::time::Duration::from_secs(1), sleep)
            .await
            .expect("a past deadline should not wait");
    }
}
