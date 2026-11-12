//! Adapts the machine's files and commands to the service logic's system.

use kkt_service::{Command, Finished, System};
use kkt_service_os::{self as native, OsSystem};
use std::io;
use std::path::Path;

pub(crate) struct ServiceSystem;

impl System for ServiceSystem {
    fn user_id(&self) -> io::Result<u32> {
        OsSystem.user_id()
    }
    fn exists(&self, path: &Path) -> io::Result<bool> {
        OsSystem.exists(path)
    }
    fn read_file(&self, path: &Path) -> io::Result<String> {
        OsSystem.read_file(path)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        OsSystem.create_dir_all(path)
    }
    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        OsSystem.write_file(path, contents)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        OsSystem.remove_file(path)
    }
    fn run(&self, command: &Command) -> io::Result<Finished> {
        OsSystem.run(command.program, &command.args).map(finished)
    }
    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        OsSystem
            .run_attached(command.program, &command.args)
            .map(finished)
    }
}

fn finished(output: native::Finished) -> Finished {
    Finished {
        code: output.code,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

#[cfg(test)]
mod tests;
