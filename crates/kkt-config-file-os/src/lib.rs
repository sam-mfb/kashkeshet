//! The config file and the instructions file on disk: reading and writing
//! them whole. Plain functions the program calls; it knows nothing of the
//! files' format, which is kkt-config-file's.

use std::fmt;
use std::io;
#[cfg(any(test, feature = "real"))]
use std::path::Path;
use std::path::PathBuf;

/// A file that could not be read: its path and why.
#[derive(Debug)]
pub struct ReadError {
    pub path: PathBuf,
    pub cause: io::Error,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not read {}: {}", self.path.display(), self.cause)
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// A file that could not be written: the step that failed, its path and why.
#[derive(Debug)]
pub enum WriteError {
    /// The folder the file goes in could not be made.
    Folder { path: PathBuf, cause: io::Error },
    /// The file could not be written.
    File { path: PathBuf, cause: io::Error },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Folder { path, cause } => {
                write!(f, "could not create {}: {cause}", path.display())
            }
            WriteError::File { path, cause } => {
                write!(f, "could not write {}: {cause}", path.display())
            }
        }
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WriteError::Folder { cause, .. } | WriteError::File { cause, .. } => Some(cause),
        }
    }
}

/// The config file's text at `path`; `None` if there is no file there.
#[cfg(any(test, feature = "real"))]
pub fn read(path: &Path) -> Result<Option<String>, ReadError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(cause) => Err(ReadError {
            path: path.to_path_buf(),
            cause,
        }),
    }
}

/// The default config file's text, if one can be seen at `path`: `None`
/// when [`Path::exists`] says there is none, which includes a file in a
/// folder that cannot be searched. `kkt serve` has always skipped a default
/// file it cannot see; one it can see but not read is an error.
#[cfg(any(test, feature = "real"))]
pub fn read_default(path: &Path) -> Result<Option<String>, ReadError> {
    match path.exists() {
        true => read_whole(path).map(Some),
        false => Ok(None),
    }
}

/// The config file's text at `path`, which must be there: a file someone
/// named, for whom a missing file is an error like any other.
#[cfg(any(test, feature = "real"))]
pub fn read_named(path: &Path) -> Result<String, ReadError> {
    read_whole(path)
}

/// The instructions file's text at `path`, which must be there.
#[cfg(any(test, feature = "real"))]
pub fn read_instructions(path: &Path) -> Result<String, ReadError> {
    read_whole(path)
}

#[cfg(any(test, feature = "real"))]
fn read_whole(path: &Path) -> Result<String, ReadError> {
    std::fs::read_to_string(path).map_err(|cause| ReadError {
        path: path.to_path_buf(),
        cause,
    })
}

/// Writes `text` as the config file at `path`, whole, making the folder it
/// goes in first if there is none.
#[cfg(any(test, feature = "real"))]
pub fn write(path: &Path, text: &str) -> Result<(), WriteError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|cause| WriteError::Folder {
            path: dir.to_path_buf(),
            cause,
        })?;
    }
    std::fs::write(path, text).map_err(|cause| WriteError::File {
        path: path.to_path_buf(),
        cause,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_scratch as scratch;

    #[test]
    fn a_missing_file_reads_as_none_unless_it_was_named() {
        let dir = scratch::Dir::new("config-file-os-missing");
        let path = dir.join("config.toml");
        assert!(read(&path).expect("no error").is_none());
        let err = read_named(&path).expect_err("missing");
        assert_eq!(err.cause.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string()
                .starts_with(&format!("could not read {}: ", path.display())),
            "{err}"
        );
        assert!(read_instructions(&path).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_default_file_in_a_folder_that_cannot_be_searched_is_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch::Dir::new("config-file-os-locked");
        let locked = dir.join("locked");
        let path = locked.join("config.toml");
        write(&path, "bind = 1\n").expect("written");
        assert_eq!(
            read_default(&path).expect("read").as_deref(),
            Some("bind = 1\n")
        );
        let mode = |m| std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(m));
        mode(0o000).expect("locked");
        // Root searches any folder; the check means something only when the
        // lock holds.
        let holds = std::fs::metadata(&path).is_err();
        let default = read_default(&path);
        let plain = read(&path);
        mode(0o755).expect("unlocked");
        if holds {
            assert!(default.expect("skipped, not an error").is_none());
            assert!(plain.is_err(), "read reports what it cannot see");
        }
    }

    #[test]
    fn a_written_file_reads_back_with_its_folder_made() {
        let dir = scratch::Dir::new("config-file-os-write");
        let path = dir.join("new").join("config.toml");
        write(&path, "bind = \"127.0.0.1:1\"\n").expect("written");
        assert_eq!(
            read(&path).expect("read").as_deref(),
            Some("bind = \"127.0.0.1:1\"\n")
        );
        assert_eq!(read_named(&path).expect("read"), "bind = \"127.0.0.1:1\"\n");
        write(&path, "").expect("replaced");
        assert_eq!(read_instructions(&path).expect("read"), "");
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_named() {
        let dir = scratch::Dir::new("config-file-os-folder");
        let blocker = dir.join("file");
        write(&blocker, "x").expect("written");
        let err = write(&blocker.join("config.toml"), "").expect_err("not a folder");
        assert!(matches!(err, WriteError::Folder { .. }), "{err}");
        assert!(
            err.to_string()
                .starts_with(&format!("could not create {}: ", blocker.display())),
            "{err}"
        );
    }
}
