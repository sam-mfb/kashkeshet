//! The [`Log`](kkt_log::Log) contract over `tracing`: [`TracingLog`]
//! sends structured events to the tracing subscriber, and [`init_logging`]
//! and [`init_stderr_logging`] start that subscriber, to stdout, stderr or a
//! writer the program gives, such as a rolling log file, with the program's
//! own lines (by the targets it names) and its libraries' at their own
//! levels. It also owns the log level setting: [`parse_level`] and its
//! defaults.

mod level;
mod record;
use kkt_log::Level;
pub use level::{DEFAULT_DEPENDENCY_LOG_LEVEL, DEFAULT_LOG_LEVEL, SettingError, parse_level};
pub use record::TracingLog;

use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Sends the log to stdout, or to `log_file`, such as a rolling log file
/// the program opens. The program's own code, whose module paths start with
/// one of `own_targets`, logs at `own`; the libraries it uses at
/// `dependencies`.
pub fn init_logging(
    log_file: Option<Box<dyn Write + Send>>,
    own_targets: &[&str],
    own: Level,
    dependencies: Level,
) -> Result<(), LoggingError> {
    let filter = log_filter(own_targets, own, dependencies);
    let format = tracing_subscriber::fmt::layer().with_target(false);
    let started = match log_file {
        // Color codes only for a terminal: the journal would show them as
        // raw escape characters.
        None => tracing_subscriber::registry()
            .with(format.with_ansi(std::io::stdout().is_terminal()))
            .with(filter)
            .try_init(),
        Some(file) => tracing_subscriber::registry()
            .with(format.with_ansi(false).with_writer(Mutex::new(file)))
            .with(filter)
            .try_init(),
    };
    started.map_err(LoggingError)
}
/// Why logging could not start, as when it has started already in this
/// process. The cause is the logging library's, given only as an error.
#[derive(Debug, thiserror::Error)]
#[error("could not start logging: {0}")]
pub struct LoggingError(#[source] tracing_subscriber::util::TryInitError);

/// Whether logging has started in this process, by either function above.
pub fn logging_started() -> bool {
    tracing::dispatcher::has_been_set()
}

/// Sends the log to stderr: the program's own lines, from `own_targets`, at
/// `own` (see [`own_log_filter`]).
pub fn init_stderr_logging(own_targets: &[&str], own: Level) -> Result<(), LoggingError> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(std::io::stderr().is_terminal())
                .with_writer(std::io::stderr),
        )
        .with(own_log_filter(own_targets, own))
        .try_init()
        .map_err(LoggingError)
}

/// The program's own lines at `own`, and none from the libraries it uses.
/// For a client command whose output is for a person or a script: a
/// library may log errors of its own when a call is dropped on purpose, as
/// at a timeout, and they are not the command's to print.
fn own_log_filter(own_targets: &[&str], own: Level) -> Targets {
    own_targets.iter().fold(
        Targets::new().with_default(LevelFilter::OFF),
        |filter, target| filter.with_target(*target, level_filter(own)),
    )
}

fn log_filter(own_targets: &[&str], own: Level, dependencies: Level) -> Targets {
    own_targets.iter().fold(
        Targets::new().with_default(level_filter(dependencies)),
        |filter, target| filter.with_target(*target, level_filter(own)),
    )
}

fn level_filter(level: Level) -> LevelFilter {
    match level {
        Level::Error => LevelFilter::ERROR,
        Level::Warn => LevelFilter::WARN,
        Level::Info => LevelFilter::INFO,
        Level::Debug => LevelFilter::DEBUG,
        Level::Trace => LevelFilter::TRACE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    /// The targets kkt's programs name: the library and the `kkt` binary,
    /// and every `kkt-*` crate, whose paths start with "kkt".
    const LOG_OWN_TARGETS: &[&str] = &["kashkeshet", "kkt"];

    #[test]
    fn watch_logs_only_kkts_own_lines() {
        let filter = own_log_filter(LOG_OWN_TARGETS, kkt_log::Level::Info);
        for target in ["rmcp", "rmcp::transport::worker", "reqwest", "hyper_util"] {
            assert!(!filter.would_enable(target, &Level::ERROR), "{target}");
        }
        for target in LOG_OWN_TARGETS {
            assert!(filter.would_enable(target, &Level::ERROR), "{target}");
            assert!(filter.would_enable(target, &Level::INFO), "{target}");
        }
    }

    /// Code in a `kkt-*` crate logs under `kkt_<crate>::…`. The filter keeps
    /// those lines at kkt's own level because "kkt" matches the start of the
    /// module path; check-crates.sh makes every crate name start with `kkt-`.
    #[test]
    fn log_filter_keeps_kkt_crate_paths_at_the_own_level() {
        let filter = log_filter(LOG_OWN_TARGETS, kkt_log::Level::Debug, kkt_log::Level::Warn);
        for target in [
            "kashkeshet",
            "kashkeshet::x",
            "kkt",
            "kkt::x",
            "kkt_x",
            "kkt_x::y",
            "kkt_web::routes",
            // A plain prefix of the text, not of a path segment.
            "kktools::x",
        ] {
            assert!(filter.would_enable(target, &Level::DEBUG), "{target}");
        }
        for target in ["other", "other::kkt", "other::kkt_x::y", "rmcp", "x_kkt"] {
            assert!(!filter.would_enable(target, &Level::INFO), "{target}");
            assert!(filter.would_enable(target, &Level::WARN), "{target}");
        }
    }
}
