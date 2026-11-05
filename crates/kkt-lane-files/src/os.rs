//! [`Disk`] on the real machine.

use super::Disk;
use crate::git::{self, Git, Ran};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The machine's files, and its `git`.
pub struct OsDisk;

impl Disk for OsDisk {
    fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
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

    fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        std::path::absolute(path)
    }

    fn git_commit(&self, folder: &Path) -> io::Result<Option<String>> {
        git::commit(&OsGit, folder)
    }
}

/// The machine's `git`.
struct OsGit;

impl Git for OsGit {
    fn run(&self, folder: &Path, args: &[&str]) -> io::Result<Ran> {
        let done = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args(args)
            // Git's messages untranslated, as `git::commit` reads one.
            .env("LC_ALL", "C")
            .output()?;
        Ok(Ran {
            code: done.status.code(),
            stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
        })
    }
}
