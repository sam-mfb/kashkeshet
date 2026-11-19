//! This program's path, as this machine gives it, and how kkt words a
//! failure to find it.

use kkt_machine::Machine;
use kkt_machine_os::OsMachine;
use std::path::PathBuf;

/// This program's absolute path, with links followed.
pub fn program_path() -> std::io::Result<PathBuf> {
    OsMachine.program_path()
}

/// Why this program's path could not be found.
#[derive(Debug, thiserror::Error)]
#[error("could not find kkt's path: {0}")]
pub struct KktPathError(#[source] pub std::io::Error);

/// This program's absolute path, for client configurations that run it.
pub fn kkt_path() -> Result<PathBuf, KktPathError> {
    program_path().map_err(KktPathError)
}
