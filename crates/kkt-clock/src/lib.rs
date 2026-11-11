//! The current time, injected so tests can control it, and kkt's own time
//! types.

mod time;

#[cfg(test)]
mod disallowed;

pub use time::{OutOfRange, Span, Time};

use futures::future::BoxFuture;

pub trait Clock: Send + Sync {
    fn now(&self) -> Time;

    /// Finishes once this clock reaches `deadline`. A deadline already passed
    /// finishes at once.
    fn sleep_until(&self, deadline: Time) -> BoxFuture<'_, ()>;
}

/// The text form used for every stored time, e.g. `2026-10-23T14:02:11Z`.
/// Times in this form sort correctly as strings.
pub fn to_text(time: Time) -> String {
    time.to_text()
}

/// Parses a stored time. Returns `None` if the text is not a stored time.
pub fn from_text(text: &str) -> Option<Time> {
    Time::from_text(text)
}

/// The start of the stretch of `every` that a stored time falls in, as a
/// stored time: with ten seconds, `14:02:17` becomes `14:02:10`. Stretches
/// are counted from the Unix epoch. `None` if the text is not a stored time.
pub fn floor_text(text: &str, every: std::time::Duration) -> Option<String> {
    let seconds = i64::try_from(every.as_secs()).ok()?.max(1);
    let at = Time::from_text(text)?.unix_seconds();
    Time::from_unix_seconds(at - at.rem_euclid(seconds)).map(Time::to_text)
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::{Clock, Span, Time};
    use futures::future::BoxFuture;
    use tokio::sync::watch;

    /// A clock that only moves when told to. Sleeps finish when `advance`
    /// moves it past their deadline.
    pub struct FixedClock(watch::Sender<Time>);

    impl FixedClock {
        pub fn new(start: Time) -> Self {
            Self(watch::Sender::new(start))
        }

        pub fn advance(&self, by: Span) {
            self.0.send_modify(|now| *now += by);
        }

        /// How many sleeps are waiting on this clock. Tests use it to know a
        /// sleep has started before they move the clock.
        pub fn sleepers(&self) -> usize {
            self.0.receiver_count()
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> Time {
            *self.0.borrow()
        }

        fn sleep_until(&self, deadline: Time) -> BoxFuture<'_, ()> {
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

    fn start() -> Time {
        Time::from_parts(2026, 1, 1, 0, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn fixed_clock_sleep_finishes_only_once_advanced_to_the_deadline() {
        let clock = FixedClock::new(start());
        let mut sleep = clock.sleep_until(start() + Span::seconds(10));
        assert_eq!(clock.sleepers(), 1);
        assert!((&mut sleep).now_or_never().is_none());
        clock.advance(Span::seconds(9));
        assert!((&mut sleep).now_or_never().is_none());
        clock.advance(Span::seconds(1));
        assert!(sleep.now_or_never().is_some());
        assert_eq!(clock.sleepers(), 0);
    }

    #[tokio::test]
    async fn fixed_clock_sleep_to_a_past_deadline_finishes_at_once() {
        let clock = FixedClock::new(start());
        let sleep = clock.sleep_until(start() - Span::seconds(1));
        assert!(sleep.now_or_never().is_some());
    }
}
