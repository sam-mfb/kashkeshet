//! Kashkeshet: a local server that lets AI agents talk to each other.
//!
//! The public modules are what the binary and the integration tests use to
//! assemble and run a server. Everything else is internal.

pub use kkt_admission as admission;
pub mod app;
pub use kkt_agent_client as mcp_client;
pub use kkt_backup as backup;
pub use kkt_client_config as clients;
pub use kkt_clock as clock;
pub use kkt_config as config;
pub use kkt_container as docker;
pub use kkt_diagnostics as diagnostics;
pub use kkt_hosts as hosts;
pub use kkt_lane_files as lane_files;
pub use kkt_logs as logs;
pub use kkt_names as names;
pub use kkt_provenance as provenance;
pub use kkt_remote_access as tailscale;
pub use kkt_service as service;
pub use kkt_settings as settings;
pub use kkt_setup as setup;
pub use kkt_shutdown as shutdown;
pub use kkt_store as store;
pub use kkt_watch as watch;
pub use kkt_web as web;
pub mod web_real;

use kkt_events as events;
use kkt_format as format;
use kkt_mcp as mcp;
use kkt_mcp::mcp_admin;
use kkt_mcp::mcp_transport;
use kkt_rules as rules;

pub use mcp::DEFAULT_INSTRUCTIONS;
/// The operator's commands against the database, for `kkt agent` and `kkt
/// lanes`.
pub use rules::{
    AgentStatus, ImportedLane, Lane, LaneImport, LaneOutcome, LaneSave, LaneSource, Operator,
    Revoked, RuleError, check_lane_import, check_retire_missing,
};
