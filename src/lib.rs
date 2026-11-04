//! Kashkeshet: a local server that lets AI agents talk to each other.
//!
//! The public modules are what the binary and the integration tests use to
//! assemble and run a server. Everything else is internal.

pub use kkt_admission as admission;
pub mod app;
pub use kkt_backup as backup;
pub use kkt_client_config as clients;
pub use kkt_clock as clock;
pub use kkt_config as config;
pub use kkt_container as docker;
pub use kkt_diagnostics as diagnostics;
pub use kkt_hosts as hosts;
pub use kkt_lane_files as lane_files;
pub use kkt_logs as logs;
pub mod mcp_client;
pub use kkt_names as names;
pub use kkt_provenance as provenance;
pub use kkt_service as service;
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

pub use mcp::DEFAULT_INSTRUCTIONS;
/// The operator's commands against the database, for `kkt agent` and `kkt
/// lanes`.
pub use rules::{
    AgentStatus, ImportedLane, Lane, LaneImport, LaneOutcome, LaneSave, LaneSource, Operator,
    Revoked, RuleError, check_lane_import, check_retire_missing,
};
