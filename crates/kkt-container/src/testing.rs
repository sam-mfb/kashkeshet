//! Compose projects in memory ([`MemoryProjects`]), for tests. No commands
//! run and no files are read; what a command in a service answers is
//! supplied by the test.

mod projects;
pub use projects::{MemoryProjects, Request};
