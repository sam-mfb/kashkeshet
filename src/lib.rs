//! The `kkt` program's library: what the binary in `src/bin/kkt/` runs.
//! `machine` is what kkt keeps on this machine; `terminal` asks setup's
//! questions and shows the Tailscale login link; `logging` starts the log;
//! `process` is this process's environment, program path and file writes;
//! `commands` has one module per command.

/// Where unit tests put their scratch files: shared with the integration tests.
#[cfg(test)]
use kkt_scratch as scratch;

pub mod commands;
pub mod logging;
pub mod machine;
pub mod process;
pub mod terminal;
