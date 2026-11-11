//! The system clock and its wall-clock sleep, implementing the shared
//! `kkt_clock::Clock` contract. Time values and formatting stay in kkt-clock.

#![cfg(any(test, feature = "real"))]

use futures::future::BoxFuture;
use kkt_clock::{Clock, Time};

/// The real system clock.
pub struct SystemClock;

/// The longest the system clock's sleeps go without looking at the wall
/// clock. Timers stop while the computer sleeps, so a sleep is a series of
/// timers this long, each followed by a look at the time; one long timer
/// could end long after its deadline.
const CLOCK_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

impl Clock for SystemClock {
    fn now(&self) -> Time {
        system_now()
    }

    fn sleep_until(&self, deadline: Time) -> BoxFuture<'_, ()> {
        Box::pin(sleep_by_wall_clock(system_now, deadline, CLOCK_RECHECK))
    }
}

/// Finishes once `now` reaches `deadline`, looking at `now` at least every
/// `step`. tokio's timers count only time the computer is awake, so a single
/// timer for the whole wait would end late by however long the computer
/// slept; checking the wall clock between short timers keeps the deadline.
async fn sleep_by_wall_clock(now: impl Fn() -> Time, deadline: Time, step: std::time::Duration) {
    // A deadline already passed cannot be converted, so it ends the wait.
    while let Ok(remaining) = (deadline - now()).to_std() {
        if remaining.is_zero() {
            return;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "the system clock's sleep is the one place kkt waits in real time"
        )]
        tokio::time::sleep(remaining.min(step)).await;
    }
}

/// The one place kkt reads the machine's wall clock.
#[expect(
    clippy::disallowed_methods,
    reason = "the system clock is the one place kkt reads real time"
)]
fn system_now() -> Time {
    Time::from(std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use kkt_clock::Span;

    fn start() -> Time {
        Time::from_text("2026-01-01T00:00:00Z").unwrap()
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "bounds a real wall-clock sleep, so a failure ends instead of hanging"
    )]
    async fn a_sleep_keeps_its_deadline_when_the_computer_sleeps() {
        // While the computer sleeps the wall clock moves and tokio's timers
        // do not. Here the wall clock jumps past the deadline, as on waking,
        // and the sleep ends within a step, not after the whole wait.
        let wall = std::sync::Mutex::new(start());
        let step = std::time::Duration::from_millis(10);
        let mut sleep = Box::pin(sleep_by_wall_clock(
            || *wall.lock().unwrap(),
            start() + Span::seconds(80),
            step,
        ));
        assert!((&mut sleep).now_or_never().is_none());
        *wall.lock().unwrap() += Span::seconds(600);
        tokio::time::timeout(std::time::Duration::from_secs(5), sleep)
            .await
            .expect("the sleep should see the wall clock pass its deadline");
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "measures a real wall-clock sleep against real elapsed time"
    )]
    async fn a_sleep_in_steps_still_lasts_until_its_deadline() {
        let begun = std::time::Instant::now();
        let wall = || start() + Span::from_std(begun.elapsed()).unwrap();
        let step = std::time::Duration::from_millis(10);
        sleep_by_wall_clock(wall, start() + Span::milliseconds(100), step).await;
        assert!(begun.elapsed() >= std::time::Duration::from_millis(100));
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "bounds the system clock's real sleep, so a failure ends instead of hanging"
    )]
    async fn system_clock_sleep_to_a_past_deadline_finishes_at_once() {
        let clock = SystemClock;
        let sleep = clock.sleep_until(clock.now() - Span::hours(1));
        tokio::time::timeout(std::time::Duration::from_secs(1), sleep)
            .await
            .expect("a past deadline should not wait");
    }
}
