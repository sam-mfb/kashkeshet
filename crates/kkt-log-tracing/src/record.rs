//! The shared log contract over the current tracing subscriber. Metadata
//! is registered once for each target, level and ordered field set.

use kkt_log::{Field, Level, Log, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tracing::callsite::{Callsite, Identifier};
use tracing::field::{DebugValue, DisplayValue, FieldSet, Value as TraceValue};
use tracing::level_filters::{LevelFilter, STATIC_MAX_LEVEL};
use tracing::metadata::Kind;
use tracing::subscriber::{Interest, NoSubscriber};
use tracing::{Event, Metadata};

/// Sends structured events to the active tracing subscriber, retaining
/// the caller's target, level, message and named fields.
pub struct TracingLog;

#[derive(PartialEq, Eq, Hash)]
struct SiteKey {
    level: Level,
    target: &'static str,
    fields: Vec<&'static str>,
}

struct Site {
    metadata: OnceLock<&'static Metadata<'static>>,
    registered: AtomicBool,
}

impl Callsite for Site {
    fn set_interest(&self, _interest: Interest) {
        // Interest is checked on the active dispatcher for every event.
    }

    fn metadata(&self) -> &Metadata<'_> {
        self.metadata.wait()
    }
}

fn trace_level(level: Level) -> tracing::Level {
    match level {
        Level::Error => tracing::Level::ERROR,
        Level::Warn => tracing::Level::WARN,
        Level::Info => tracing::Level::INFO,
        Level::Debug => tracing::Level::DEBUG,
        Level::Trace => tracing::Level::TRACE,
    }
}

fn new_site(key: &SiteKey) -> &'static Site {
    let site: &'static Site = Box::leak(Box::new(Site {
        metadata: OnceLock::new(),
        registered: AtomicBool::new(false),
    }));
    let names = Box::leak(key.fields.clone().into_boxed_slice());
    let metadata = Box::leak(Box::new(Metadata::new(
        "kkt log event",
        key.target,
        trace_level(key.level),
        None,
        None,
        Some(key.target),
        FieldSet::new(names, Identifier(site)),
        Kind::EVENT,
    )));
    site.metadata.get_or_init(|| metadata);
    site
}

fn site(level: Level, target: &'static str, fields: &[Field<'_>]) -> &'static Site {
    static SITES: OnceLock<Mutex<HashMap<SiteKey, &'static Site>>> = OnceLock::new();
    let key = SiteKey {
        level,
        target,
        fields: std::iter::once("message")
            .chain(fields.iter().map(|field| field.name))
            .collect(),
    };
    let site = {
        let mut sites = SITES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *sites.entry(key).or_insert_with_key(new_site)
    };
    // Subscriber callbacks run only after the cache mutex is released.
    // A simultaneous first event can check its dispatcher while the first
    // thread registers, as tracing's own callsites do.
    if !site.registered.swap(true, Ordering::AcqRel) {
        tracing::callsite::register(site);
    }
    site
}

enum Encoded<'a> {
    Display(DisplayValue<&'a dyn fmt::Display>),
    Debug(DebugValue<&'a dyn fmt::Debug>),
}

impl Encoded<'_> {
    fn value(&self) -> &(dyn TraceValue + '_) {
        match self {
            Self::Display(value) => value,
            Self::Debug(value) => value,
        }
    }
}

impl Log for TracingLog {
    fn record(
        &self,
        level: Level,
        target: &'static str,
        message: fmt::Arguments<'_>,
        fields: &[Field<'_>],
    ) {
        let native_level = trace_level(level);
        if native_level > STATIC_MAX_LEVEL || native_level > LevelFilter::current() {
            return;
        }
        tracing::dispatcher::get_default(|dispatch| {
            if dispatch.is::<NoSubscriber>() {
                return;
            }
            let site = site(level, target, fields);
            let metadata = site.metadata();
            if !dispatch.enabled(metadata) {
                return;
            }
            let encoded: Vec<_> = fields
                .iter()
                .map(|field| match field.value {
                    Value::Display(value) => Encoded::Display(tracing::field::display(value)),
                    Value::Debug(value) => Encoded::Debug(tracing::field::debug(value)),
                })
                .collect();
            let message = tracing::field::display(message);
            let values: Vec<Option<&dyn TraceValue>> =
                std::iter::once(Some(&message as &dyn TraceValue))
                    .chain(encoded.iter().map(|value| Some(value.value())))
                    .collect();
            let values = metadata.fields().value_set_all(&values);
            let event = Event::new(metadata, &values);
            // Dispatch applies the subscriber's event-level filter too.
            dispatch.event(&event);
        });
    }
}

#[cfg(test)]
mod tests;
