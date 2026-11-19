use super::*;
use std::io::{self, Write};
use std::sync::{Arc, MutexGuard};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

struct BufferWriter<'a>(MutexGuard<'a, Vec<u8>>);

impl Write for BufferWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = BufferWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        BufferWriter(self.0.lock().expect("buffer"))
    }
}

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("buffer").clone()).expect("UTF-8")
    }
}

#[test]
fn structured_events_render_as_the_original_tracing_macros_do() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(true)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(buffer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let error = "broken template";
        let name = "channels";
        tracing::error!(target: "kkt_web::page", error = %error, "could not render {name}");
        TracingLog.record(
            Level::Error,
            "kkt_web::page",
            format_args!("could not render {name}"),
            &[Field::display("error", &error)],
        );
        let host = Some("bad.example");
        tracing::warn!(target: "kkt_web::page", host = ?host, "refused a web request");
        TracingLog.record(
            Level::Warn,
            "kkt_web::page",
            format_args!("refused a web request"),
            &[Field::debug("host", &host)],
        );
        tracing::error!(target: "kkt_mcp", "missing principal");
        TracingLog.record(
            Level::Error,
            "kkt_mcp",
            format_args!("missing principal"),
            &[],
        );
    });
    let text = buffer.text();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 6, "{text}");
    for pair in lines.as_chunks::<2>().0 {
        assert_eq!(pair[0], pair[1]);
    }
}

#[test]
fn the_active_subscribers_target_and_level_filters_apply() {
    let buffer = Buffer::default();
    let filter = tracing_subscriber::filter::Targets::new()
        .with_target("kept", LevelFilter::WARN)
        .with_default(LevelFilter::OFF);
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_writer(buffer.clone()),
        )
        .with(filter);
    tracing::subscriber::with_default(subscriber, || {
        TracingLog.record(Level::Info, "kept::page", format_args!("too quiet"), &[]);
        TracingLog.record(Level::Error, "other", format_args!("wrong target"), &[]);
        TracingLog.record(Level::Warn, "kept::page", format_args!("kept event"), &[]);
    });
    let text = buffer.text();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("kept event"), "{text}");
}

#[test]
fn simultaneous_first_events_keep_all_their_fields() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(buffer.clone())
        .finish();
    let dispatch = tracing::Dispatch::new(subscriber);
    std::thread::scope(|scope| {
        for id in 0..16 {
            let dispatch = dispatch.clone();
            scope.spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    TracingLog.record(
                        Level::Info,
                        "kkt_log_concurrent_first",
                        format_args!("first event"),
                        &[Field::display("id", &id)],
                    );
                });
            });
        }
    });
    let text = buffer.text();
    assert_eq!(text.lines().count(), 16, "{text}");
    for id in 0..16 {
        assert!(
            text.lines()
                .any(|line| line.ends_with(&format!("first event id={id}"))),
            "{id}: {text}"
        );
    }
}
