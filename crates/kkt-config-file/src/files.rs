//! Reading and writing the config file and the instructions file whole,
//! over the [`Disk`] the program gives: what a missing file means for each,
//! and the wording of what went wrong. The program adapts the machine's
//! files to [`Disk`].

use std::io;
use std::path::{Path, PathBuf};

/// The files this crate reads and writes, as the program gives them.
pub trait Disk {
    /// The text of the file at `path`, whole.
    fn read(&self, path: &Path) -> io::Result<String>;
    /// Whether something can be seen at `path`. A path in a folder that
    /// cannot be searched cannot be seen.
    fn exists(&self, path: &Path) -> bool;
    /// Makes the folder at `path` and any missing folders above it.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Writes `text` as the file at `path`, whole.
    fn write(&self, path: &Path, text: &str) -> io::Result<()>;
}

/// A file that could not be read: its path and why.
#[derive(Debug, thiserror::Error)]
#[error("could not read {}: {}", .path.display(), .cause)]
pub struct ReadError {
    pub path: PathBuf,
    #[source]
    pub cause: io::Error,
}

/// A file that could not be written: the step that failed, its path and why.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The folder the file goes in could not be made.
    #[error("could not create {}: {}", .path.display(), .cause)]
    Folder {
        path: PathBuf,
        #[source]
        cause: io::Error,
    },
    /// The file could not be written.
    #[error("could not write {}: {}", .path.display(), .cause)]
    File {
        path: PathBuf,
        #[source]
        cause: io::Error,
    },
}

/// The config file's text at `path`; `None` if there is no file there.
pub fn read(disk: &dyn Disk, path: &Path) -> Result<Option<String>, ReadError> {
    match disk.read(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(cause) => Err(ReadError {
            path: path.to_path_buf(),
            cause,
        }),
    }
}

/// The default config file's text, if one can be seen at `path`: `None`
/// when [`Disk::exists`] says there is none, which includes a file in a
/// folder that cannot be searched. `kkt serve` has always skipped a default
/// file it cannot see; one it can see but not read is an error.
pub fn read_default(disk: &dyn Disk, path: &Path) -> Result<Option<String>, ReadError> {
    match disk.exists(path) {
        true => read_whole(disk, path).map(Some),
        false => Ok(None),
    }
}

/// The config file's text at `path`, which must be there: a file someone
/// named, for whom a missing file is an error like any other.
pub fn read_named(disk: &dyn Disk, path: &Path) -> Result<String, ReadError> {
    read_whole(disk, path)
}

/// The instructions file's text at `path`, which must be there.
pub fn read_instructions(disk: &dyn Disk, path: &Path) -> Result<String, ReadError> {
    read_whole(disk, path)
}

fn read_whole(disk: &dyn Disk, path: &Path) -> Result<String, ReadError> {
    disk.read(path).map_err(|cause| ReadError {
        path: path.to_path_buf(),
        cause,
    })
}

/// Writes `text` as the config file at `path`, whole, making the folder it
/// goes in first if there is none.
pub fn write(disk: &dyn Disk, path: &Path, text: &str) -> Result<(), WriteError> {
    if let Some(dir) = path.parent() {
        disk.create_dir_all(dir)
            .map_err(|cause| WriteError::Folder {
                path: dir.to_path_buf(),
                cause,
            })?;
    }
    disk.write(path, text).map_err(|cause| WriteError::File {
        path: path.to_path_buf(),
        cause,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};

    /// Files in memory. A path in `hidden` cannot be seen but can be read;
    /// a path in `unreadable` can be seen but not read; a folder in
    /// `blocked` cannot be made.
    #[derive(Default)]
    struct Memory {
        files: RefCell<BTreeMap<PathBuf, String>>,
        folders: RefCell<BTreeSet<PathBuf>>,
        hidden: BTreeSet<PathBuf>,
        unreadable: BTreeSet<PathBuf>,
        blocked: BTreeSet<PathBuf>,
    }

    impl Disk for Memory {
        fn read(&self, path: &Path) -> io::Result<String> {
            if self.unreadable.contains(path) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
            }
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
        }
        fn exists(&self, path: &Path) -> bool {
            !self.hidden.contains(path)
                && (self.files.borrow().contains_key(path) || self.unreadable.contains(path))
        }
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            if self.blocked.contains(path) {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "a file"));
            }
            self.folders.borrow_mut().insert(path.to_path_buf());
            Ok(())
        }
        fn write(&self, path: &Path, text: &str) -> io::Result<()> {
            let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
            if !self.folders.borrow().contains(&folder) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no folder"));
            }
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), text.to_string());
            Ok(())
        }
    }

    #[test]
    fn a_missing_file_reads_as_none_unless_it_was_named() {
        let disk = Memory::default();
        let path = Path::new("/config/config.toml");
        assert!(read(&disk, path).expect("no error").is_none());
        assert!(read_default(&disk, path).expect("no error").is_none());
        let err = read_named(&disk, path).expect_err("missing");
        assert_eq!(err.cause.kind(), io::ErrorKind::NotFound);
        assert_eq!(err.path, path);
        assert_eq!(
            err.to_string(),
            "could not read /config/config.toml: no such file"
        );
        assert!(read_instructions(&disk, path).is_err());
    }

    #[test]
    fn a_default_file_that_cannot_be_seen_is_skipped() {
        let path = PathBuf::from("/locked/config.toml");
        let disk = Memory {
            hidden: BTreeSet::from([path.clone()]),
            ..Memory::default()
        };
        write(&disk, &path, "bind = 1\n").expect("written");
        assert!(read_default(&disk, &path).expect("skipped").is_none());
        assert_eq!(
            read(&disk, &path).expect("read").as_deref(),
            Some("bind = 1\n")
        );
    }

    #[test]
    fn a_file_that_can_be_seen_but_not_read_is_an_error() {
        let path = PathBuf::from("/config/config.toml");
        let disk = Memory {
            unreadable: BTreeSet::from([path.clone()]),
            ..Memory::default()
        };
        let err = read_default(&disk, &path).expect_err("unreadable");
        assert_eq!(
            err.to_string(),
            "could not read /config/config.toml: denied"
        );
        assert!(read(&disk, &path).is_err());
    }

    #[test]
    fn a_written_file_reads_back_with_its_folder_made() {
        let disk = Memory::default();
        let path = Path::new("/new/config.toml");
        write(&disk, path, "bind = \"127.0.0.1:1\"\n").expect("written");
        assert!(disk.folders.borrow().contains(Path::new("/new")));
        assert_eq!(
            read(&disk, path).expect("read").as_deref(),
            Some("bind = \"127.0.0.1:1\"\n")
        );
        assert_eq!(
            read_default(&disk, path).expect("read").as_deref(),
            Some("bind = \"127.0.0.1:1\"\n")
        );
        assert_eq!(
            read_named(&disk, path).expect("read"),
            "bind = \"127.0.0.1:1\"\n"
        );
        write(&disk, path, "").expect("replaced");
        assert_eq!(read_instructions(&disk, path).expect("read"), "");
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_named() {
        let disk = Memory {
            blocked: BTreeSet::from([PathBuf::from("/file")]),
            ..Memory::default()
        };
        let err = write(&disk, Path::new("/file/config.toml"), "").expect_err("not a folder");
        assert!(matches!(err, WriteError::Folder { .. }), "{err}");
        assert_eq!(err.to_string(), "could not create /file: a file");
    }

    #[test]
    fn a_file_that_cannot_be_written_is_named() {
        struct ReadOnly;
        impl Disk for ReadOnly {
            fn read(&self, _: &Path) -> io::Result<String> {
                Ok(String::new())
            }
            fn exists(&self, _: &Path) -> bool {
                true
            }
            fn create_dir_all(&self, _: &Path) -> io::Result<()> {
                Ok(())
            }
            fn write(&self, _: &Path, _: &str) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "read-only"))
            }
        }
        let err = write(&ReadOnly, Path::new("/c/config.toml"), "").expect_err("read-only");
        assert!(matches!(err, WriteError::File { .. }), "{err}");
        assert_eq!(err.to_string(), "could not write /c/config.toml: read-only");
    }
}
