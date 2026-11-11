//! What kkt keeps on this machine: the serve settings and config file, the
//! setup record, key files, how a client reaches the server, agent
//! credential files, and the database file.

pub mod connection;
pub(crate) mod container;
pub(crate) mod credentials;
pub mod database;
pub mod keys;
pub mod record;
pub(crate) mod service;
pub mod settings;
