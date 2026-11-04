//! The `kkt` commands, one module each. Each exposes a `run` that takes
//! plain inputs; the binary's `cli` turns the command line into them.
//! `lifecycle` so far holds only the backups that `serve` also makes, and
//! `lanes` only the lane helpers that `setup` also uses.

pub mod doctor;
pub mod lanes;
pub mod lifecycle;
pub mod serve;
pub mod service;
pub mod setup;
