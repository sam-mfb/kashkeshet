//! The machine's files for lane folders, as plain [`OsDisk`] methods. The
//! program adapts them to the lane-file logic's `Disk`. The folder's git
//! commit is kkt-lane-commit's.

#![cfg(feature = "real")]

use std::io;
use std::path::{Path, PathBuf};

/// The machine's files.
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
}
