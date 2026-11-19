//! The server's keys, as a contract: what a key must be and how long a new
//! one is and what it is drawn from; which access key a server uses; the
//! names, mode and default place of the files that hold keys; the admin
//! agent's name tied to the admin key; and [`KeyFiles`], reading and
//! writing private key files, with the reads and writes made of those
//! steps. Who a key lets in is kkt-admission's.
//!
//! kkt-keys-os implements [`KeyFiles`] on the files on disk.

mod admin_name;
mod files;
mod place;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use admin_name::{
    AdminNameError, AdminNameFile, NameFileError, bound_in, bound_to, read_admin_name,
};
pub use files::{KeyFileAction, KeyFileError, KeyFiles, ReadError, SelectedKeys};
pub use place::{
    ACCESS_KEY_FILE_MODE, ADMIN_KEY_COPY_FILE_NAME, NO_ADMIN_KEY_EXIT, WEB_KEY_COPY_FILE_NAME,
    access_key_file, admin_key_path, admin_name_path, web_key_path,
};

use std::path::PathBuf;

/// The characters a new key is drawn from: lowercase ASCII letters and
/// digits, as kkt-names drew them before. Keep it so, so new keys look like
/// the ones existing installs hold. Draw a key with kkt-entropy's
/// `random_chars(NEW_KEY_ALPHABET, NEW_KEY_CHARS)`; it passes
/// [`check_access_key`].
pub const NEW_KEY_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// A key file's text, and where it came from, such as the file's path.
pub struct KeyText {
    pub text: String,
    pub source: String,
}

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

/// Which access key a server uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessKey {
    /// None: `/mcp` and the web page are open to anyone who can reach them.
    None,
    /// The key in this file, which must exist.
    File(PathBuf),
    /// The key in this default file, created if it does not exist yet.
    Default(PathBuf),
}

/// Shortest access key, in bytes. 32 random hex digits carry 128 bits.
const MIN_ACCESS_KEY_BYTES: usize = 32;
/// Characters in a new key, such as `kkt key create` makes, from
/// [`NEW_KEY_ALPHABET`]'s 36 characters: about 258 bits.
pub const NEW_KEY_CHARS: usize = 50;
const _: () = assert!(NEW_KEY_CHARS >= MIN_ACCESS_KEY_BYTES);
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
    use std::path::Path;

    /// Existing Docker installs keep their key copies under these names,
    /// and kkt in a container tells "no admin key" by this exit code, which
    /// kkt-container-docker reads as 4.
    #[test]
    fn the_key_copies_names_and_the_no_admin_key_exit_stay_as_installs_have_them() {
        assert_eq!(WEB_KEY_COPY_FILE_NAME, "docker-web-key");
        assert_eq!(ADMIN_KEY_COPY_FILE_NAME, "docker-admin-key");
        assert_eq!(NO_ADMIN_KEY_EXIT, 4);
    }

    #[test]
    fn the_access_key_goes_in_the_config_directory() {
        assert_eq!(
            access_key_file(Path::new("/home/sam/.config/kkt")),
            PathBuf::from("/home/sam/.config/kkt/access-key")
        );
    }

    #[test]
    fn the_web_and_admin_keys_live_beside_the_access_key() {
        let access = Path::new("/keys/access-key");
        assert_eq!(web_key_path(access), PathBuf::from("/keys/web-key"));
        assert_eq!(
            admin_key_path(access),
            Some(PathBuf::from("/keys/admin-key"))
        );
        // An access key already in the admin key's place leaves no place
        // for an admin key.
        assert_eq!(admin_key_path(Path::new("/keys/admin-key")), None);
    }

    #[test]
    fn each_admin_key_file_has_its_own_name_file() {
        assert_eq!(
            admin_name_path(Path::new("/keys/admin-key")),
            PathBuf::from("/keys/admin-key.name")
        );
        assert_eq!(
            admin_name_path(Path::new("/keys/other")),
            PathBuf::from("/keys/other.name")
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

    #[test]
    fn new_keys_drawn_as_the_contract_says_are_accepted_and_differ() {
        use kkt_entropy::Entropy;
        use kkt_entropy::testing::SeqEntropy;
        let entropy = SeqEntropy::new(3);
        let first = entropy.random_chars(NEW_KEY_ALPHABET, NEW_KEY_CHARS);
        let second = entropy.random_chars(NEW_KEY_ALPHABET, NEW_KEY_CHARS);
        assert_eq!(first.len(), NEW_KEY_CHARS);
        assert_ne!(first, second);
        assert_eq!(check_access_key(&first), Ok(()));
    }

    #[test]
    fn new_keys_are_drawn_from_lowercase_letters_and_digits() {
        let expected: Vec<u8> = (b'a'..=b'z').chain(b'0'..=b'9').collect();
        assert_eq!(NEW_KEY_ALPHABET, expected.as_slice());
    }
}
