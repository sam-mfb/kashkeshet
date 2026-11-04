//! The real [`System`]: files on disk and real commands.

use super::{Command, Finished, System};
use std::io;
use std::path::Path;
use std::process::Stdio;

/// The real system: files on disk and real commands.
pub struct OsSystem;

impl System for OsSystem {
    fn user_id(&self) -> io::Result<u32> {
        let finished = self.run(&Command::query("id", &["-u"]))?;
        finished
            .stdout
            .trim()
            .parse()
            .map_err(|_| io::Error::other(format!("id -u printed {:?}", finished.stdout)))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn run(&self, command: &Command) -> io::Result<Finished> {
        let output = std::process::Command::new(command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .output()?;
        Ok(Finished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        let status = std::process::Command::new(command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .status()?;
        Ok(Finished {
            code: status.code(),
            ..Finished::default()
        })
    }
}
