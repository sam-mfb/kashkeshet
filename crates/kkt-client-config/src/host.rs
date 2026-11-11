//! What client configuration needs from the machine: files, paths and the
//! `claude` command. [`OsHost`] is the real one, behind the `real` feature;
//! unit tests use a fake.

use std::io;
use std::path::{Path, PathBuf};

pub trait Host {
    /// The file's text.
    fn read(&self, path: &Path) -> io::Result<String>;

    fn exists(&self, path: &Path) -> bool;

    /// Creates `path`, which must not exist yet (`AlreadyExists` otherwise),
    /// readable only by its owner, and writes `text` to it durably.
    fn create_private(&self, path: &Path, text: &str) -> io::Result<()>;

    /// Makes the file readable only by its owner.
    fn restrict(&self, path: &Path) -> io::Result<()>;

    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Replaces `to` with `from` in one step.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// `path` made absolute against the current directory.
    fn absolute(&self, path: &Path) -> io::Result<PathBuf>;

    /// `path` with every symbolic link resolved; `NotFound` if it does not
    /// exist.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;

    /// A number that no other process running now has, for temporary names.
    fn process_id(&self) -> u32;

    /// Runs `claude` with `args`, setting `CLAUDE_CONFIG_DIR` to
    /// `config_dir`, or unsetting it for `None`. The error is for people and
    /// never holds the command's output, which may hold credentials.
    fn claude(&self, args: &[&str], config_dir: Option<&Path>) -> Result<(), String>;
}

#[cfg(feature = "real")]
pub use os::OsHost;

#[cfg(feature = "real")]
mod os {
    use super::Host;
    /// Unix permissions of a client config that holds a key: only the
    /// owner may read or write it.
    const PRIVATE_FILE_MODE: u32 = 0o600;
    use std::fs::{self, OpenOptions};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// The real machine.
    pub struct OsHost;

    impl Host for OsHost {
        fn read(&self, path: &Path) -> io::Result<String> {
            fs::read_to_string(path)
        }

        fn exists(&self, path: &Path) -> bool {
            path.exists()
        }

        fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
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

        fn restrict(&self, path: &Path) -> io::Result<()> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
            }
            #[cfg(not(unix))]
            let _ = path;
            Ok(())
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            fs::create_dir_all(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            fs::rename(from, to)
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            fs::remove_file(path)
        }

        fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
            std::path::absolute(path)
        }

        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            path.canonicalize()
        }

        fn process_id(&self) -> u32 {
            std::process::id()
        }

        fn claude(&self, args: &[&str], config_dir: Option<&Path>) -> Result<(), String> {
            let mut command = Command::new("claude");
            command.args(args);
            match config_dir {
                Some(dir) => command.env("CLAUDE_CONFIG_DIR", dir),
                None => command.env_remove("CLAUDE_CONFIG_DIR"),
            };
            let result = command.output().map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    "claude was not found on PATH; install Claude Code, then retry --write"
                        .to_string()
                } else {
                    format!("could not run claude: {e}")
                }
            })?;
            if !result.status.success() {
                // The CLI may echo credentials in its output. Report only the status.
                return Err(format!("claude mcp failed with {}", result.status));
            }
            Ok(())
        }
    }
}
