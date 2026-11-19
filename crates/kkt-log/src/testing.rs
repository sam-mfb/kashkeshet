//! An in-memory log, for assertions about the events a caller produces.

use crate::{Field, Level, Log, Value};
use std::fmt;
use std::sync::Mutex;

/// One recorded event, retaining field names and their formatted values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub level: Level,
    pub target: &'static str,
    pub message: String,
    pub fields: Vec<(&'static str, String)>,
}

/// A log that retains events for assertions without performing I/O.
#[derive(Default)]
pub struct CapturedLog {
    entries: Mutex<Vec<Entry>>,
}

impl CapturedLog {
    /// The recorded events in order, as an independent snapshot.
    pub fn entries(&self) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Log for CapturedLog {
    fn record(
        &self,
        level: Level,
        target: &'static str,
        message: fmt::Arguments<'_>,
        fields: &[Field<'_>],
    ) {
        let entry = Entry {
            level,
            target,
            message: message.to_string(),
            fields: fields
                .iter()
                .map(|field| {
                    let value = match field.value {
                        Value::Display(value) => value.to_string(),
                        Value::Debug(value) => format!("{value:?}"),
                    };
                    (field.name, value)
                })
                .collect(),
        };
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(entry);
    }
}
