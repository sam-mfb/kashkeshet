//! The machine's client-config files, as plain OsHost methods adapted by
//! the program to the client-config logic. (Claude Code's `claude` command
//! is kkt-client-command's.)

#![cfg(feature = "real")]

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

/// The real machine.
pub struct OsHost;

impl OsHost {
    pub fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    pub fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(PRIVATE_FILE_MODE);
        }
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    }

    pub fn restrict(&self, path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    pub fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    pub fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        std::path::absolute(path)
    }

    pub fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }

    pub fn process_id(&self) -> u32 {
        std::process::id()
    }
}
