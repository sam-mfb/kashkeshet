//! The config file and the instructions file on disk: reading and writing
//! them whole. Plain functions the program calls; it knows nothing of the
//! files' format, which is kkt-config-file's.

#[cfg(any(test, feature = "real"))]
use std::fmt;
#[cfg(any(test, feature = "real"))]
use std::io;
#[cfg(any(test, feature = "real"))]
use std::path::{Path, PathBuf};

/// A file that could not be read: its path and why.
#[cfg(any(test, feature = "real"))]
#[derive(Debug)]
pub struct ReadError {
    pub path: PathBuf,
    pub cause: io::Error,
}

#[cfg(any(test, feature = "real"))]
impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not read {}: {}", self.path.display(), self.cause)
    }
}

#[cfg(any(test, feature = "real"))]
impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// A file that could not be written: the step that failed, its path and why.
#[cfg(any(test, feature = "real"))]
#[derive(Debug)]
pub enum WriteError {
    /// The folder the file goes in could not be made.
    Folder { path: PathBuf, cause: io::Error },
    /// The file could not be written.
    File { path: PathBuf, cause: io::Error },
}

#[cfg(any(test, feature = "real"))]
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

#[cfg(any(test, feature = "real"))]
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
