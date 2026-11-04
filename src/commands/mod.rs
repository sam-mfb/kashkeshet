//! The `kkt` commands, one module each. Each exposes a `run` that takes
//! plain inputs; the binary's `cli` turns the command line into them.
//! `lifecycle` so far holds only the backups that `serve` also makes.

pub mod lifecycle;
pub mod serve;
pub mod service;
