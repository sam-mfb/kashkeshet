//! A stand-in [`Machine`]: the facts it is given.

use crate::{Env, Machine, Platform};
use std::io;
use std::path::PathBuf;

/// A machine whose facts are fixed when it is made.
#[derive(Clone, Debug)]
pub struct FixedMachine {
    pub env: Env,
    pub platform: Platform,
    pub process_id: u32,
    /// The program's path; `None` when it cannot be found.
    pub program_path: Option<PathBuf>,
    pub path_dirs: Vec<PathBuf>,
}

impl Machine for FixedMachine {
    fn env(&self) -> Env {
        self.env.clone()
    }
    fn platform(&self) -> Platform {
        self.platform
    }
    fn process_id(&self) -> u32 {
        self.process_id
    }
    fn program_path(&self) -> io::Result<PathBuf> {
        self.program_path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no program path"))
    }
    fn path_dirs(&self) -> Vec<PathBuf> {
        self.path_dirs.clone()
    }
}
