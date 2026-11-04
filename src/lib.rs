//! Kashkeshet: a local server that lets AI agents talk to each other.
//!
//! The public modules are what the binary and the integration tests use to
//! assemble and run a server. Everything else is internal.

pub use kkt_admission as admission;
pub mod app;
pub use kkt_backup as backup;
pub mod clients;
pub use kkt_clock as clock;
pub use kkt_config as config;
pub mod diagnostics;
pub mod docker;
pub use kkt_hosts as hosts;
pub use kkt_lane_files as lane_files;
pub mod logs;
pub mod mcp_client;
pub use kkt_names as names;
pub use kkt_provenance as provenance;
pub mod service;
pub use kkt_settings as settings;
pub mod setup;
pub use kkt_remote_access as tailscale;
pub use kkt_shutdown as shutdown;
pub use kkt_store as store;
pub mod watch;
pub mod web;
pub mod web_real;

use kkt_events as events;
mod format;
mod mcp;
mod mcp_admin;
mod mcp_tools;
mod mcp_transport;
mod rules;
mod tool_text;

/// Where unit tests put their scratch files: shared with the integration tests.
#[cfg(test)]
use kkt_scratch as scratch;

pub use mcp::DEFAULT_INSTRUCTIONS;
/// The operator's commands against the database, for `kkt agent` and `kkt
/// lanes`.
pub use rules::{
    AgentStatus, ImportedLane, Lane, LaneImport, LaneOutcome, LaneSave, LaneSource, Operator,
    Revoked, RuleError, check_lane_import, check_retire_missing,
};
