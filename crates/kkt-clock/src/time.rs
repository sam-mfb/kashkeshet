//! kkt's own time types: `Time`, a moment in UTC, and `Span`, a signed
//! length of time between two moments. They wrap chrono, which no other kkt
//! crate depends on, and offer only the operations and text forms kkt uses.

use chrono::{DateTime, NaiveDateTime, SecondsFormat, SubsecRound, TimeDelta, Utc};
use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

/// The time in a backup's name, such as `20261028T131500.123Z`.
const BACKUP_FORMAT: &str = "%Y%m%dT%H%M%S%.3fZ";

/// A moment in UTC, to the nanosecond.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(DateTime<Utc>);

/// A signed length of time, to the nanosecond.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span(TimeDelta);

/// A length of time that a `Span` or a `std::time::Duration` cannot hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfRange(chrono::OutOfRangeError);

impl Time {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Time = Time(DateTime::UNIX_EPOCH);

    /// The text form used for every stored time, e.g. `2026-10-23T14:02:11Z`:
    /// whole seconds, UTC written as `Z`. Times in this form sort correctly as
    /// strings.
    pub fn to_text(self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    /// Reads an RFC 3339 time, such as a stored time, in any offset. `None`
    /// if the text is not one.
    pub fn from_text(text: &str) -> Option<Time> {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|t| Time(t.with_timezone(&Utc)))
    }

    /// The time in a backup's name, such as `20261028T131500.123Z`: to the
    /// millisecond, without separators. It sorts as text in time order.
    pub fn to_backup_text(self) -> String {
        self.0.format(BACKUP_FORMAT).to_string()
    }

    /// Whether `text` is a time in the form `to_backup_text` writes.
    pub fn is_backup_text(text: &str) -> bool {
        NaiveDateTime::parse_from_str(text, BACKUP_FORMAT).is_ok()
    }

    /// `HH:MM`, such as `14:02`.
    pub fn to_hour_minute_text(self) -> String {
        self.0.format("%H:%M").to_string()
    }

    /// `YYYY-MM-DD HH:MM`, such as `2026-10-23 14:02`.
    pub fn to_date_minute_text(self) -> String {
        self.0.format("%Y-%m-%d %H:%M").to_string()
    }

    /// Whether `self` and `other` fall on the same UTC calendar day.
    pub fn same_day(self, other: Time) -> bool {
        self.0.date_naive() == other.0.date_naive()
    }

    /// This time with any fraction of a second dropped.
    pub fn whole_seconds(self) -> Time {
        Time(self.0.trunc_subsecs(0))
    }

    /// Seconds since the Unix epoch, with any fraction dropped toward the past.
    pub fn unix_seconds(self) -> i64 {
        self.0.timestamp()
    }

    /// The time `seconds` after the Unix epoch. `None` if out of range.
    pub fn from_unix_seconds(seconds: i64) -> Option<Time> {
        DateTime::from_timestamp(seconds, 0).map(Time)
    }

    /// `self + span`, or `None` if the result is out of range.
    pub fn checked_add(self, span: Span) -> Option<Time> {
        self.0.checked_add_signed(span.0).map(Time)
    }

    /// `self - span`, or `None` if the result is out of range.
    pub fn checked_sub(self, span: Span) -> Option<Time> {
        self.0.checked_sub_signed(span.0).map(Time)
    }

    /// The UTC time from its parts, for tests. `None` if the parts are not a
    /// real time.
    #[cfg(any(test, feature = "testing"))]
    pub fn from_parts(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Option<Time> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .map(Time)
    }
}

impl Span {
    pub const ZERO: Span = Span(TimeDelta::zero());
    pub const MAX: Span = Span(TimeDelta::MAX);

    /// Panics if out of range, as `i64::MAX` milliseconds is.
    pub fn milliseconds(n: i64) -> Span {
        Span(TimeDelta::milliseconds(n))
    }

    /// Panics if out of range.
    pub fn seconds(n: i64) -> Span {
        Span(TimeDelta::seconds(n))
    }

    /// Panics if out of range.
    pub fn minutes(n: i64) -> Span {
        Span(TimeDelta::minutes(n))
    }

    /// Panics if out of range.
    pub fn hours(n: i64) -> Span {
        Span(TimeDelta::hours(n))
    }

    /// Panics if out of range.
    pub fn days(n: i64) -> Span {
        Span(TimeDelta::days(n))
    }

    /// The span as long as `duration`. Fails if it is longer than `Span::MAX`.
    pub fn from_std(duration: std::time::Duration) -> Result<Span, OutOfRange> {
        TimeDelta::from_std(duration).map(Span).map_err(OutOfRange)
    }

    /// The span as a `std::time::Duration`. Fails if it is negative.
    pub fn to_std(self) -> Result<std::time::Duration, OutOfRange> {
        self.0.to_std().map_err(OutOfRange)
    }

    /// The whole milliseconds in the span, rounded toward zero.
    pub fn num_milliseconds(self) -> i64 {
        self.0.num_milliseconds()
    }
}

impl fmt::Display for OutOfRange {
    /// The same text as chrono's error, which kkt's messages have always shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for OutOfRange {}

/// The same text as a chrono time's debug form, such as
/// `2026-10-23T14:02:11Z`, so test failures read as before.
impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// The same text as a chrono duration's debug form.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// A file's time, such as when it was last written.
impl From<std::time::SystemTime> for Time {
    fn from(time: std::time::SystemTime) -> Time {
        Time(DateTime::from(time))
    }
}

/// Panics if the result is out of range.
impl Add<Span> for Time {
    type Output = Time;
    fn add(self, span: Span) -> Time {
        Time(self.0 + span.0)
    }
}

/// Panics if the result is out of range.
impl Add<std::time::Duration> for Time {
    type Output = Time;
    fn add(self, duration: std::time::Duration) -> Time {
        Time(self.0 + duration)
    }
}

/// Panics if the result is out of range.
impl Sub<Span> for Time {
    type Output = Time;
    fn sub(self, span: Span) -> Time {
        Time(self.0 - span.0)
    }
}

/// Panics if the result is out of range.
impl AddAssign<Span> for Time {
    fn add_assign(&mut self, span: Span) {
        self.0 += span.0;
    }
}

/// Panics if the result is out of range.
impl SubAssign<Span> for Time {
    fn sub_assign(&mut self, span: Span) {
        self.0 -= span.0;
    }
}

/// How long after `earlier` this time is; negative if it is before.
impl Sub<Time> for Time {
    type Output = Span;
    fn sub(self, earlier: Time) -> Span {
        Span(self.0 - earlier.0)
    }
}

/// Panics if the result is out of range.
impl Add<Span> for Span {
    type Output = Span;
    fn add(self, other: Span) -> Span {
        Span(self.0 + other.0)
    }
}

/// Panics if the result is out of range.
impl Sub<Span> for Span {
    type Output = Span;
    fn sub(self, other: Span) -> Span {
        Span(self.0 - other.0)
    }
}

// The conversions to and from chrono, while the other crates move to `Time`
// and `Span`. They go once no crate needs them.

impl From<DateTime<Utc>> for Time {
    fn from(time: DateTime<Utc>) -> Time {
        Time(time)
    }
}

impl From<Time> for DateTime<Utc> {
    fn from(time: Time) -> DateTime<Utc> {
        time.0
    }
}

impl From<TimeDelta> for Span {
    fn from(span: TimeDelta) -> Span {
        Span(span)
    }
}

impl From<Span> for TimeDelta {
    fn from(span: Span) -> TimeDelta {
        span.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Time {
        Time::from_text(text).unwrap()
    }

    #[test]
    fn stored_text_is_whole_seconds_in_utc_with_z() {
        let t = at("2026-10-23T14:02:11.987654321Z");
        assert_eq!(t.to_text(), "2026-10-23T14:02:11Z");
        assert_eq!(Time::UNIX_EPOCH.to_text(), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn stored_text_reads_back_to_the_same_time() {
        let text = "2026-10-23T14:02:11Z";
        assert_eq!(at(text).to_text(), text);
        assert_eq!(Time::from_parts(2026, 10, 23, 14, 2, 11), Some(at(text)));
    }

    #[test]
    fn from_text_reads_other_offsets_as_utc_and_refuses_non_times() {
        assert_eq!(
            at("2026-10-23T16:02:11+02:00").to_text(),
            "2026-10-23T14:02:11Z"
        );
        assert_eq!(Time::from_text("not a time"), None);
        assert_eq!(Time::from_text("2026-10-23 14:02:11"), None);
        assert_eq!(Time::from_text(""), None);
    }

    #[test]
    fn backup_text_is_to_the_millisecond_and_reads_back() {
        let t = at("2026-10-28T13:15:00.123456Z");
        assert_eq!(t.to_backup_text(), "20261028T131500.123Z");
        assert_eq!(
            at("2026-10-28T13:15:00Z").to_backup_text(),
            "20261028T131500.000Z"
        );
        assert!(Time::is_backup_text("20261028T131500.123Z"));
        // As chrono has always read it, the fraction may be left out.
        assert!(Time::is_backup_text("20261028T131500Z"));
        assert!(!Time::is_backup_text("2026-10-28T13:15:00Z"));
        assert!(!Time::is_backup_text("x"));
    }

    #[test]
    fn short_forms_and_same_day() {
        let t = at("2026-10-23T14:02:59Z");
        assert_eq!(t.to_hour_minute_text(), "14:02");
        assert_eq!(t.to_date_minute_text(), "2026-10-23 14:02");
        assert!(t.same_day(at("2026-10-23T00:00:00Z")));
        assert!(t.same_day(at("2026-10-23T23:59:59Z")));
        assert!(!t.same_day(at("2026-10-24T00:00:00Z")));
        // The day is the UTC day, whatever offset the text was written in.
        assert!(t.same_day(at("2026-10-24T01:00:00+02:00")));
        assert!(!t.same_day(at("2026-10-23T23:00:00-02:00")));
    }

    #[test]
    fn whole_seconds_drops_the_fraction() {
        let t = at("2026-10-23T14:02:11.5Z");
        assert_eq!(t.whole_seconds(), at("2026-10-23T14:02:11Z"));
        assert!(t.whole_seconds() < t);
        let whole = at("2026-10-23T14:02:11Z");
        assert_eq!(whole.whole_seconds(), whole);
    }

    #[test]
    fn unix_seconds_round_trip() {
        let t = at("2026-10-23T14:02:11.9Z");
        assert_eq!(t.unix_seconds(), 1_792_764_131);
        assert_eq!(
            Time::from_unix_seconds(1_792_764_131),
            Some(at("2026-10-23T14:02:11Z"))
        );
        assert_eq!(Time::from_unix_seconds(0), Some(Time::UNIX_EPOCH));
        assert_eq!(Time::from_unix_seconds(i64::MAX), None);
    }

    #[test]
    fn times_compare_in_time_order() {
        let a = at("2026-10-23T14:02:11Z");
        let b = at("2026-10-23T14:02:12Z");
        assert!(a < b);
        assert_eq!(a.max(b), b);
        assert_eq!(a, at("2026-10-23T16:02:11+02:00"));
    }

    #[test]
    fn time_arithmetic() {
        let t = at("2026-10-23T14:02:11Z");
        assert_eq!(t + Span::seconds(49), at("2026-10-23T14:03:00Z"));
        assert_eq!(t - Span::minutes(2), at("2026-10-23T14:00:11Z"));
        assert_eq!(t + Span::hours(10), at("2026-10-24T00:02:11Z"));
        assert_eq!(t - Span::days(23), at("2026-09-30T14:02:11Z"));
        assert_eq!(t + Span::milliseconds(1500), at("2026-10-23T14:02:12.5Z"));
        assert_eq!(
            t + std::time::Duration::from_secs(60),
            at("2026-10-23T14:03:11Z")
        );
        let mut moving = t;
        moving += Span::seconds(1);
        assert_eq!(moving, at("2026-10-23T14:02:12Z"));
        moving -= Span::seconds(2);
        assert_eq!(moving, at("2026-10-23T14:02:10Z"));
        assert_eq!(at("2026-10-23T14:03:11Z") - t, Span::minutes(1));
        assert_eq!(t - at("2026-10-23T14:03:11Z"), Span::seconds(-60));
    }

    #[test]
    fn checked_arithmetic_refuses_out_of_range() {
        let t = at("2026-10-23T14:02:11Z");
        assert_eq!(
            t.checked_add(Span::seconds(1)),
            Some(at("2026-10-23T14:02:12Z"))
        );
        assert_eq!(
            t.checked_sub(Span::seconds(1)),
            Some(at("2026-10-23T14:02:10Z"))
        );
        assert_eq!(t.checked_add(Span::MAX), None);
        assert_eq!(t.checked_sub(Span::MAX), None);
    }

    #[test]
    fn span_arithmetic_and_comparison() {
        assert_eq!(Span::seconds(60), Span::minutes(1));
        assert_eq!(Span::minutes(60), Span::hours(1));
        assert_eq!(Span::hours(24), Span::days(1));
        assert_eq!(Span::milliseconds(1000), Span::seconds(1));
        assert_eq!(Span::seconds(1) + Span::seconds(2), Span::seconds(3));
        assert_eq!(Span::seconds(1) - Span::seconds(2), Span::seconds(-1));
        assert!(Span::seconds(-1) < Span::ZERO);
        assert!(Span::ZERO < Span::milliseconds(1));
        assert!(Span::days(36_500) < Span::MAX);
        assert_eq!(Span::milliseconds(1999).num_milliseconds(), 1999);
        assert_eq!(Span::seconds(-2).num_milliseconds(), -2000);
    }

    #[test]
    fn spans_convert_to_and_from_std() {
        let std = std::time::Duration::from_millis(1500);
        assert_eq!(Span::from_std(std), Ok(Span::milliseconds(1500)));
        assert_eq!(Span::milliseconds(1500).to_std(), Ok(std));
        assert_eq!(Span::ZERO.to_std(), Ok(std::time::Duration::ZERO));
        assert!(Span::seconds(-1).to_std().is_err());
        assert!(Span::from_std(std::time::Duration::MAX).is_err());
    }

    #[test]
    fn out_of_range_says_what_chrono_said() {
        let ours = Span::seconds(-1).to_std().unwrap_err();
        let chrono = TimeDelta::seconds(-1).to_std().unwrap_err();
        assert_eq!(ours.to_string(), chrono.to_string());
        assert_eq!(
            ours.to_string(),
            "Source duration value is out of range for the target type"
        );
    }

    #[test]
    fn system_times_become_times() {
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_792_764_131);
        assert_eq!(Time::from(t), at("2026-10-23T14:02:11Z"));
    }

    #[test]
    fn debug_forms_are_chronos() {
        let chrono = DateTime::<Utc>::UNIX_EPOCH + TimeDelta::seconds(1);
        assert_eq!(format!("{:?}", Time::from(chrono)), format!("{chrono:?}"));
        assert_eq!(
            format!("{:?}", Span::seconds(90)),
            format!("{:?}", TimeDelta::seconds(90))
        );
    }

    #[test]
    fn chrono_conversions_keep_the_value() {
        let chrono = DateTime::<Utc>::UNIX_EPOCH + TimeDelta::milliseconds(1234);
        assert_eq!(DateTime::<Utc>::from(Time::from(chrono)), chrono);
        let delta = TimeDelta::milliseconds(-1234);
        assert_eq!(TimeDelta::from(Span::from(delta)), delta);
    }
}
