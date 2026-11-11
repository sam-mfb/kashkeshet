//! The server's keys as text and as files: what a key must be, how long a
//! new one is, and the names, mode and default place of the files that hold
//! them.

use std::path::{Path, PathBuf};

/// A key setting the server refuses to start with: the setting's name and
/// why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct SettingError {
    pub name: &'static str,
    pub rule: String,
}

impl SettingError {
    pub fn new(name: &'static str, rule: impl Into<String>) -> SettingError {
        SettingError {
            name,
            rule: rule.into(),
        }
    }
}

/// Shortest access key, in bytes. 32 random hex digits carry 128 bits.
pub(crate) const MIN_ACCESS_KEY_BYTES: usize = 32;
/// Characters in a key made by `kkt key create`, from the 36-character
/// alphabet: about 258 bits.
pub(crate) const NEW_ACCESS_KEY_CHARS: usize = 50;
const _: () = assert!(NEW_ACCESS_KEY_CHARS >= MIN_ACCESS_KEY_BYTES);
/// Unix permissions of a key file `kkt key create` writes: only the owner
/// may read or write it.
pub const ACCESS_KEY_FILE_MODE: u32 = 0o600;
/// Where `kkt key create` writes a key, in the config directory.
pub const ACCESS_KEY_FILE_NAME: &str = "access-key";
/// The operator login key, beside the access key.
pub const WEB_KEY_FILE_NAME: &str = "web-key";
/// The admin agent's key for `/mcp-admin`, beside the access key. Only made
/// when the operator asks; without it, `/mcp-admin` is off.
pub const ADMIN_KEY_FILE_NAME: &str = "admin-key";
/// Added to an admin key file's name for the file of its admin agent's name
/// (`admin-key.name` beside `admin-key`): written with the key by `kkt key
/// create --admin --name`, read with it by `kkt serve`. The file holds a
/// hash of the key too, so the name belongs to that key alone. Without it,
/// or for another key, the admin key may only read.
pub const ADMIN_NAME_FILE_SUFFIX: &str = ".name";

/// Where `kkt key create` writes a key when no `--path` is given: the
/// access key file in kkt's config directory, `config_dir`.
pub fn access_key_file(config_dir: &Path) -> PathBuf {
    config_dir.join(ACCESS_KEY_FILE_NAME)
}

/// Checks an access key, as read from `--access-key-file` without
/// surrounding whitespace. It must be at least [`MIN_ACCESS_KEY_BYTES`] long
/// and only printable ASCII with no spaces, so it can be sent in an HTTP
/// header as written.
pub fn check_access_key(key: &str) -> Result<(), SettingError> {
    if key.is_empty() {
        return Err(SettingError::new("access_key_file", "must not be empty"));
    }
    if !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(SettingError::new(
            "access_key_file",
            "must hold only printable ASCII with no spaces",
        ));
    }
    if key.len() < MIN_ACCESS_KEY_BYTES {
        return Err(SettingError::new(
            "access_key_file",
            format!("must hold a key of at least {MIN_ACCESS_KEY_BYTES} characters"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_access_key_goes_in_the_config_directory() {
        assert_eq!(
            access_key_file(Path::new("/home/sam/.config/kkt")),
            PathBuf::from("/home/sam/.config/kkt/access-key")
        );
    }

    #[test]
    fn access_keys_must_be_long_printable_and_unspaced() {
        let long = "k".repeat(MIN_ACCESS_KEY_BYTES);
        assert_eq!(check_access_key(&long), Ok(()));
        let short = "k".repeat(MIN_ACCESS_KEY_BYTES - 1);
        let spaced = format!("{long} {long}");
        let accented = format!("{long}é");
        for bad in ["", short.as_str(), spaced.as_str(), accented.as_str()] {
            let err = check_access_key(bad).expect_err(bad);
            assert_eq!(err.name, "access_key_file");
        }
    }
}
