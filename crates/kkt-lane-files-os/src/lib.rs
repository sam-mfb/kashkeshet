//! The machine's files and git for lane folders, as plain [`OsDisk`] methods.
//! The program adapts them to the contracts of the lane-file logic.

#![cfg(feature = "real")]

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The machine's files, and its `git`.
pub struct OsDisk;

impl OsDisk {
    pub fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|name| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the file name {name:?} is not UTF-8"),
                )
            })?;
            names.push(name);
        }
        Ok(names)
    }

    pub fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    pub fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        std::path::absolute(path)
    }

    /// Runs `git -C <folder> <args>` in the C locale, without interpreting
    /// its exit status. A missing git executable returns `NotFound`.
    pub fn git(&self, folder: &Path, args: &[&str]) -> io::Result<GitOutput> {
        let done = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args(args)
            .env("LC_ALL", "C")
            .output()?;
        Ok(GitOutput {
            code: done.status.code(),
            stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
        })
    }
}

/// The exit status and text returned by the machine's git.
pub struct GitOutput {
    /// `None` when git was stopped by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
