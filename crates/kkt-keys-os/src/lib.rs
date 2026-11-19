//! Private key files on disk: [`OsKeyFiles`] implements kkt-keys'
//! [`KeyFiles`] contract with the operating system's files, for the
//! server's key files and the client key copies setup writes, as
//! kkt-keys names and places them. Key generation is the caller's.

use kkt_keys::{KeyFileAction, KeyFileError, KeyFiles};
use std::io::{self, Write};
use std::path::Path;

/// The key files on this machine's disk.
pub struct OsKeyFiles;

/// A file's permission bits.
#[cfg(unix)]
fn permission_bits(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

/// No permission bits: this system does not have them.
#[cfg(not(unix))]
fn permission_bits(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

impl KeyFiles for OsKeyFiles {
    fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn try_exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn mode(&self, path: &Path) -> io::Result<Option<u32>> {
        std::fs::metadata(path).map(|metadata| permission_bits(&metadata))
    }

    fn create_dir_all(&self, dir: &Path) -> Result<(), KeyFileError> {
        std::fs::create_dir_all(dir).map_err(|e| KeyFileError::file(KeyFileAction::Create, dir, e))
    }

    fn keep_staged(&self, staged: &Path, path: &Path) -> Result<(), KeyFileError> {
        std::fs::rename(staged, path).map_err(|e| KeyFileError::file(KeyFileAction::Move, path, e))
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), KeyFileError> {
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
            options.mode(kkt_keys::ACCESS_KEY_FILE_MODE);
        }
        let mut file = options.open(path).map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists => KeyFileError::Exists {
                path: path.to_owned(),
            },
            _ => KeyFileError::file(KeyFileAction::Write, path, e),
        })?;
        // The mode above applies only to a new file; an existing one replaced
        // with --force keeps its old mode unless it is set here.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(
                kkt_keys::ACCESS_KEY_FILE_MODE,
            ))
            .map_err(|e| KeyFileError::file(KeyFileAction::Restrict, path, e))?;
        }
        writeln!(file, "{key}").map_err(|e| KeyFileError::file(KeyFileAction::Write, path, e))
    }

    fn write_admin_name(&self, admin_key: &Path, bound: Option<&str>) -> io::Result<()> {
        let names = kkt_keys::admin_name_path(admin_key);
        match bound {
            Some(text) => std::fs::write(names, text),
            None => match std::fs::remove_file(&names) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_scratch as scratch;

    #[test]
    fn a_files_mode_is_its_permission_bits() {
        let dir = scratch::Dir::new("keys-os-mode");
        let path = dir.join("access-key");
        OsKeyFiles
            .write_key(&path, "a key", false)
            .expect("written");
        let mode = OsKeyFiles.mode(&path).expect("mode");
        if cfg!(unix) {
            assert_eq!(mode, Some(kkt_keys::ACCESS_KEY_FILE_MODE));
        } else {
            assert_eq!(mode, None);
        }
        assert_eq!(
            OsKeyFiles
                .mode(&dir.join("missing"))
                .expect_err("missing")
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    /// Every message the disk's steps give that a test can cause without
    /// root, pinned exactly: kkt's words around the system's own cause,
    /// which each case gets by doing the same thing itself.
    #[test]
    fn every_key_file_message_says_what_it_always_said() {
        let dir = scratch::Dir::new("keys-os-messages");
        let file = dir.join("a-file");
        std::fs::write(&file, "").expect("file");

        let under_file = file.join("keys");
        let cause = std::fs::create_dir_all(&under_file).expect_err("not a folder");
        assert_eq!(
            OsKeyFiles
                .create_dir_all(&under_file)
                .map_err(|e| e.to_string()),
            Err(format!(
                "could not create {}: {cause}",
                under_file.display()
            ))
        );

        let taken = dir.join("taken");
        std::fs::write(&taken, "old").expect("taken");
        assert_eq!(
            OsKeyFiles
                .write_key(&taken, "key", false)
                .map_err(|e| e.to_string()),
            Err(format!(
                "{} already exists; pass --force to replace it",
                taken.display()
            ))
        );

        let missing = dir.join("no-folder").join("access-key");
        let cause = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&missing)
            .expect_err("no folder");
        assert_eq!(
            OsKeyFiles
                .write_key(&missing, "key", false)
                .map_err(|e| e.to_string()),
            Err(format!("could not write {}: {cause}", missing.display()))
        );

        let staged = dir.join("staged");
        std::fs::write(&staged, "key").expect("staged");
        let cause = std::fs::rename(dir.join("nothing-staged"), &missing).expect_err("no folder");
        assert_eq!(
            OsKeyFiles
                .keep_staged(&dir.join("nothing-staged"), &missing)
                .map_err(|e| e.to_string()),
            Err(format!(
                "could not move the key to {}: {cause}",
                missing.display()
            ))
        );
    }
}
