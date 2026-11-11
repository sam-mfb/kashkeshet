//! Starting the log: to stdout, stderr or a log file, with kkt's own lines
//! and its libraries' at their own levels.

use kkt_clock::SystemClock;
use kkt_logs::{LogLevel, LogPolicy, RotatingFile};
use kkt_settings::DEFAULT_LOG_LEVEL;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The targets of kkt's own code: the library and the `kkt` binary.
const LOG_OWN_TARGETS: &[&str] = &["kashkeshet", "kkt"];

/// Sends the log to stdout, or to a log file kept within the policy given
/// with it. kkt's own code logs at `own`, the libraries it uses at
/// `dependencies`.
pub fn init_logging(
    log_file: Option<(&Path, LogPolicy)>,
    own: LogLevel,
    dependencies: LogLevel,
) -> Result<(), String> {
    let filter = log_filter(own, dependencies);
    let format = tracing_subscriber::fmt::layer().with_target(false);
    let started = match log_file {
        // Color codes only for a terminal: the journal would show them as
        // raw escape characters.
        None => tracing_subscriber::registry()
            .with(format.with_ansi(std::io::stdout().is_terminal()))
            .with(filter)
            .try_init(),
        Some((path, policy)) => {
            policy
                .check()
                .map_err(|e| format!("invalid setting: {e}"))?;
            let file = RotatingFile::open(path, policy, Arc::new(SystemClock)).map_err(|e| {
                format!(
                    "invalid setting: log_file could not be opened at {}: {e}",
                    path.display()
                )
            })?;
            tracing_subscriber::registry()
                .with(format.with_ansi(false).with_writer(Mutex::new(file)))
                .with(filter)
                .try_init()
        }
    };
    started.map_err(|e| format!("could not start logging: {e}"))
}

/// Sends the log to stderr: kkt's own lines only (see [`own_log_filter`]).
pub fn init_stderr_logging() -> Result<(), String> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(std::io::stderr().is_terminal())
                .with_writer(std::io::stderr),
        )
        .with(own_log_filter(DEFAULT_LOG_LEVEL))
        .try_init()
        .map_err(|e| format!("could not start logging: {e}"))
}

/// kkt's own lines at `own`, and none from the libraries it uses. For a
/// client command such as `kkt watch`, whose output is for a person or a
/// script: the MCP library logs errors of its own when a call is dropped
/// on purpose, as at the timeout, and they are not the command's to print.
fn own_log_filter(own: LogLevel) -> Targets {
    LOG_OWN_TARGETS.iter().fold(
        Targets::new().with_default(LevelFilter::OFF),
        |filter, target| filter.with_target(*target, level_filter(own)),
    )
}

fn log_filter(own: LogLevel, dependencies: LogLevel) -> Targets {
    LOG_OWN_TARGETS.iter().fold(
        Targets::new().with_default(level_filter(dependencies)),
        |filter, target| filter.with_target(*target, level_filter(own)),
    )
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    #[test]
    fn watch_logs_only_kkts_own_lines() {
        let filter = own_log_filter(DEFAULT_LOG_LEVEL);
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
        let filter = log_filter(LogLevel::Debug, LogLevel::Warn);
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
