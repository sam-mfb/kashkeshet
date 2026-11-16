//! [`KeyFiles`]: reading and writing private key files. These are the
//! server's key files (the access, web and admin keys and the admin name
//! file) and the copies of a key that setup writes for clients. The trait's
//! required methods are the steps an implementation carries out on the
//! files; its provided methods are the reads and writes made of those
//! steps, the same for every implementation.

use crate::{AccessKey, KeyText, admin_name_path};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Private key files: the server's, and the client key copies setup
/// writes. Every path given is one of those files or the folder that holds
/// them; the plain file tests and steps (`exists`, `try_exists`, `is_file`,
/// `create_dir_all`, `keep_staged`, `remove`) are for those files only.
pub trait KeyFiles {
    /// The file's text, exactly as written.
    fn read(&self, path: &Path) -> io::Result<String>;
    /// Whether a file is at `path`, as `Path::exists` says.
    fn exists(&self, path: &Path) -> bool;
    /// Whether a file is at `path`, or why that cannot be told.
    fn try_exists(&self, path: &Path) -> io::Result<bool>;
    /// Whether `path` is a file, following links.
    fn is_file(&self, path: &Path) -> bool;
    /// Makes the folder `dir` and its parents.
    fn create_dir_all(&self, dir: &Path) -> Result<(), String>;
    /// Moves a staged key file to `path`.
    fn keep_staged(&self, staged: &Path, path: &Path) -> Result<(), String>;
    /// Removes one file.
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// Writes `key` to `path`, readable and writable only by its owner
    /// ([`ACCESS_KEY_FILE_MODE`](crate::ACCESS_KEY_FILE_MODE)). Refuses to
    /// replace an existing file unless `force`.
    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String>;
    /// Writes the name file beside the admin key at `admin_key`
    /// ([`admin_name_path`]) with `bound`, text already bound to the key;
    /// with `None`, removes that name file, and a missing one is fine.
    fn write_admin_name(&self, admin_key: &Path, bound: Option<&str>) -> io::Result<()>;

    /// The file's text, or `None` only when there is no file.
    fn read_optional(&self, path: &Path) -> io::Result<Option<String>> {
        match self.read(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The file's text, or why it could not be read, with its path.
    fn read_key(&self, path: &Path) -> Result<String, ReadError> {
        self.read(path).map_err(|error| ReadError {
            path: path.to_owned(),
            error,
        })
    }

    /// The access key's text, if `keys` names one; its file must exist.
    fn read_access(&self, keys: &SelectedKeys) -> Result<Option<KeyText>, ReadError> {
        required(self, keys.access)
    }

    /// The web key's text, if `keys` names one; its file must exist.
    fn read_web(&self, keys: &SelectedKeys) -> Result<Option<KeyText>, ReadError> {
        required(self, keys.web)
    }

    /// The admin key's text, if `keys` names one; a missing file means no
    /// admin key.
    fn read_admin(&self, keys: &SelectedKeys) -> Result<Option<KeyText>, ReadError> {
        let Some(path) = keys.admin else {
            return Ok(None);
        };
        Ok(optional(self, path)?.map(|text| KeyText {
            text,
            source: path.display().to_string(),
        }))
    }

    /// The text of the name file beside the admin key `keys` names, if
    /// both are there.
    fn read_admin_names(&self, keys: &SelectedKeys) -> Result<Option<String>, ReadError> {
        match keys.admin {
            Some(path) => optional(self, &admin_name_path(path)),
            None => Ok(None),
        }
    }

    /// Writes a new private key file at `path`, making its folder first,
    /// and only then drawing the key from `generate`. Refuses to replace an
    /// existing file unless `force`. Returns the key.
    fn new_key(
        &self,
        path: &Path,
        force: bool,
        generate: &mut dyn FnMut() -> String,
    ) -> Result<String, String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            self.create_dir_all(dir)?;
        }
        let key = generate();
        self.write_key(path, &key, force)?;
        Ok(key)
    }

    /// Writes the default key file if `access` is the default and its file
    /// is missing, making its folder first and only then drawing the key
    /// from `generate`. Returns the path only when a file was made, so the
    /// caller can say so.
    fn make_default_key(
        &self,
        access: &AccessKey,
        generate: &mut dyn FnMut() -> String,
    ) -> Result<Option<PathBuf>, String> {
        let AccessKey::Default(path) = access else {
            return Ok(None);
        };
        if self.exists(path) {
            return Ok(None);
        }
        if let Some(dir) = path.parent() {
            self.create_dir_all(dir)?;
        }
        self.write_key(path, &generate(), false)?;
        Ok(Some(path.clone()))
    }
}

/// The key files a server's settings name: no path means that key is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectedKeys<'a> {
    pub access: Option<&'a Path>,
    pub web: Option<&'a Path>,
    pub admin: Option<&'a Path>,
}

/// A key file that could not be read: its path and the system's reason.
#[derive(Debug)]
pub struct ReadError {
    /// The file that could not be read.
    pub path: PathBuf,
    /// The system's reason, unchanged.
    pub error: io::Error,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not read {}: {}", self.path.display(), self.error)
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

fn required<F: KeyFiles + ?Sized>(
    files: &F,
    path: Option<&Path>,
) -> Result<Option<KeyText>, ReadError> {
    path.map(|path| {
        files.read_key(path).map(|text| KeyText {
            text,
            source: path.display().to_string(),
        })
    })
    .transpose()
}

fn optional<F: KeyFiles + ?Sized>(files: &F, path: &Path) -> Result<Option<String>, ReadError> {
    files.read_optional(path).map_err(|error| ReadError {
        path: path.to_owned(),
        error,
    })
}

#[cfg(test)]
mod tests;
