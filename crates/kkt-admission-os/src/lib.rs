//! The server's key files: reading texts, private key writes and staged
//! key moves. Callers supply key generation and adapt reads to KeyTexts.

#![cfg(feature = "real")]

use kkt_admission::AccessKey;
use kkt_admission::server_keys::KeyText;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A failed key-file read, retaining its path and underlying I/O error.
#[derive(Debug)]
pub struct ReadError {
    /// The selected file that could not be read.
    pub path: PathBuf,
    /// The unchanged operating-system read failure.
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

/// The server's selected access, web and admin key files.
pub struct KeyFiles<'a> {
    access: Option<&'a Path>,
    web: Option<&'a Path>,
    admin: Option<&'a Path>,
}

impl<'a> KeyFiles<'a> {
    /// The already-selected key paths; no path means that key is off.
    pub fn new(access: Option<&'a Path>, web: Option<&'a Path>, admin: Option<&'a Path>) -> Self {
        Self { access, web, admin }
    }
    fn required(path: Option<&Path>) -> Result<Option<KeyText>, ReadError> {
        path.map(|path| {
            Self::read(path).map(|text| KeyText {
                text,
                source: path.display().to_string(),
            })
        })
        .transpose()
    }
    /// The access key's text, if configured; a selected file must exist.
    pub fn access(&self) -> Result<Option<KeyText>, ReadError> {
        Self::required(self.access)
    }
    /// The web key's text, if configured; a selected file must exist.
    pub fn web(&self) -> Result<Option<KeyText>, ReadError> {
        Self::required(self.web)
    }
    /// The admin key's text; a missing file means no admin key.
    pub fn admin(&self) -> Result<Option<KeyText>, ReadError> {
        match self.admin {
            Some(path) => Ok(Self::read_optional(path)?.map(|text| KeyText {
                text,
                source: path.display().to_string(),
            })),
            None => Ok(None),
        }
    }
    /// The name text beside the selected admin key, if its file exists.
    pub fn admin_names(&self) -> Result<Option<String>, ReadError> {
        match self.admin {
            Some(path) => Self::read_optional(&kkt_admission::admin_name_path(path)),
            None => Ok(None),
        }
    }
    /// Reads the key or name file exactly as written.
    pub fn read(path: &Path) -> Result<String, ReadError> {
        std::fs::read_to_string(path).map_err(|error| ReadError {
            path: path.to_owned(),
            error,
        })
    }
    /// Reads a key or name file, returning None only when it is missing.
    pub fn read_optional(path: &Path) -> Result<Option<String>, ReadError> {
        match Self::read(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    /// Whether the selected path is a file, following links.
    pub fn is_file(path: &Path) -> bool {
        path.is_file()
    }

    /// Whether a selected key path exists, retaining failures for backup checks.
    pub fn try_exists(path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    /// Whether a key file exists, with the same semantics as Path::exists.
    pub fn exists(path: &Path) -> bool {
        path.exists()
    }
    /// Creates the key folder and its parents.
    pub fn create_dir_all(dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))
    }
    /// Moves a staged key to the final path selected by setup.
    pub fn keep_staged(staged: &Path, path: &Path) -> Result<(), String> {
        std::fs::rename(staged, path)
            .map_err(|e| format!("could not move the key to {}: {e}", path.display()))
    }
    /// Removes one key file, retaining the I/O error for the caller.
    pub fn remove(path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

/// Writes a missing default key, drawing a key only after making its folder.
/// Returns its path only when a file was created, so the caller can report it.
pub fn make_default_key(
    access: &AccessKey,
    generate: impl FnOnce() -> String,
) -> Result<Option<PathBuf>, String> {
    let AccessKey::Default(path) = access else {
        return Ok(None);
    };
    if KeyFiles::exists(path) {
        return Ok(None);
    }
    if let Some(dir) = path.parent() {
        KeyFiles::create_dir_all(dir)?;
    }
    write_key_file(path, &generate(), false)?;
    Ok(Some(path.clone()))
}

/// Creates a new private key file, making its folder before drawing the key.
pub fn new_key_file(
    path: &Path,
    force: bool,
    generate: impl FnOnce() -> String,
) -> Result<String, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        KeyFiles::create_dir_all(dir)?;
    }
    let key = generate();
    write_key_file(path, &key, force)?;
    Ok(key)
}

/// Writes already-bound admin-name text beside the key, or removes the old
/// name file. The caller retains its own success and failure wording.
pub fn write_admin_name(path: &Path, bound: Option<&str>) -> io::Result<()> {
    let names = kkt_admission::admin_name_path(path);
    match bound {
        Some(text) => std::fs::write(names, text),
        None => match KeyFiles::remove(&names) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        },
    }
}

/// Writes `key` to `path`, readable and writable only by its owner. Refuses
/// to replace an existing file unless `force`.
pub fn write_key_file(path: &Path, key: &str, force: bool) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(kkt_admission::ACCESS_KEY_FILE_MODE);
    }
    let mut file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!(
            "{} already exists; pass --force to replace it",
            path.display()
        ),
        _ => format!("could not write {}: {e}", path.display()),
    })?;
    // The mode above applies only to a new file; an existing one replaced
    // with --force keeps its old mode unless it is set here.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(
            kkt_admission::ACCESS_KEY_FILE_MODE,
        ))
        .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
    }
    writeln!(file, "{key}").map_err(|e| format!("could not write {}: {e}", path.display()))
}
