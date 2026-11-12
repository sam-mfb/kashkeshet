//! What kkt keeps on this machine: the serve settings and config file, the
//! setup record, key files, how a client reaches the server, agent
//! credential files, and the database file.

pub mod agent_parts;
pub mod client_config;
pub mod connection;
pub(crate) mod credentials;
pub mod database;
pub mod diagnostics;
pub mod keys;
pub mod operate;
mod serve_options;
pub mod server_run;
pub(crate) mod service;
pub mod settings;
