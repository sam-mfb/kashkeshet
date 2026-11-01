//! [`Disk`] on the real machine.

use super::Disk;
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
        let head = match git(folder, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(head) => head,
            // No git here: the folder's commit cannot be known.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some(head) = head.filter(|h| !h.is_empty()) else {
            return Ok(None);
        };
        // A folder git ignores is in no commit.
        let ignored = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args(["check-ignore", "--quiet", "."])
            .output()?;
        if ignored.status.success() {
            return Ok(None);
        }
        // Changes to this folder's files that the commit does not have.
        let changes = git(folder, &["status", "--porcelain", "--", "."])?;
        Ok(Some(match changes {
            Some(changes) if changes.is_empty() => head,
            _ => format!("{head}-dirty"),
        }))
    }
}

/// Runs git in `folder`: its output, trimmed, or `None` when it failed (as
/// outside a git checkout).
fn git(folder: &Path, args: &[&str]) -> io::Result<Option<String>> {
    let done = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()?;
    if !done.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&done.stdout).trim().to_string(),
    ))
}
