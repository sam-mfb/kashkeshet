//! Structured log events, independent of their destination. A caller gives
//! its target, level, message and named values; the provider applies filters
//! and records them. No logging library's types cross this contract.

use std::fmt;

/// An event's severity, from errors through tracing detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// A borrowed field's formatting, retained until the provider records it.
#[derive(Clone, Copy)]
pub enum Value<'a> {
    Display(&'a dyn fmt::Display),
    Debug(&'a dyn fmt::Debug),
}

/// One named value accompanying an event's message.
#[derive(Clone, Copy)]
pub struct Field<'a> {
    pub name: &'static str,
    pub value: Value<'a>,
}

impl<'a> Field<'a> {
    /// A field using its Display representation.
    pub fn display(name: &'static str, value: &'a dyn fmt::Display) -> Self {
        Self {
            name,
            value: Value::Display(value),
        }
    }

    /// A field using its Debug representation.
    pub fn debug(name: &'static str, value: &'a dyn fmt::Debug) -> Self {
        Self {
            name,
            value: Value::Debug(value),
        }
    }
}

/// A destination for the caller's structured events.
pub trait Log: Send + Sync {
    /// Records one event, subject to this provider's filters. `target` is
    /// normally the caller's module path; field and message borrows last
    /// only for this call.
    fn record(
        &self,
        level: Level,
        target: &'static str,
        message: fmt::Arguments<'_>,
        fields: &[Field<'_>],
    );
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use testing::{CapturedLog, Entry};

    #[test]
    fn records_keep_the_target_level_message_and_separate_formatted_fields() {
        let log = CapturedLog::default();
        let host = Some("bad.example");
        log.record(
            Level::Warn,
            "example::page",
            format_args!("refused request {}", 7),
            &[
                Field::debug("host", &host),
                Field::display("error", &"not allowed"),
            ],
        );
        assert_eq!(
            log.entries(),
            vec![Entry {
                level: Level::Warn,
                target: "example::page",
                message: "refused request 7".into(),
                fields: vec![
                    ("host", "Some(\"bad.example\")".into()),
                    ("error", "not allowed".into()),
                ],
            }]
        );
    }
}
