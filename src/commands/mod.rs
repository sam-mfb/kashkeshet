//! The `kkt` commands, one module each. Each exposes a `run` that takes
//! plain inputs; the binary's `cli` turns the command line into them.
//! A module for a command with several subcommands exposes one entry for
//! each instead.

pub mod agent;
pub mod doctor;
pub mod key;
pub mod lanes;
pub mod lifecycle;
pub mod serve;
pub mod service;
pub mod setup;
pub mod watch;
