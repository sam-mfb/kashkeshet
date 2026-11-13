//! The server's keys: what a key must be, how a new one is made, which
//! access key a server uses, the names, mode and default place of the files
//! that hold them, and the admin agent's name tied to the admin key. Who a
//! key lets in is kkt-admission's.
//!
//! Behind `real`, the key files on disk: reading their texts, private key
//! writes, admin-name writes and staged-key moves.

mod admin_name;
#[cfg(feature = "real")]
mod files;
#[cfg(feature = "testing")]
pub mod testing;

pub use admin_name::{admin_name_line, admin_name_written, bound_in, bound_to};
#[cfg(feature = "real")]
pub use files::{
    KeyFiles, ReadError, make_default_key, new_key_file, write_admin_name, write_key_file,
};

use kkt_entropy::Entropy;
use std::path::{Path, PathBuf};

/// Characters new access keys are drawn from: lowercase ASCII letters and
/// digits, as kkt-names drew them before. Keep it so, so new keys look like
/// the ones existing installs hold.
const RANDOM_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// A key file's text, and where it came from, such as the file's path.
pub struct KeyText {
    pub text: String,
    pub source: String,
}

/// A new random key, such as `kkt key create` writes. It passes
/// [`check_access_key`].
pub fn generate(entropy: &dyn Entropy) -> String {
    entropy.random_chars(RANDOM_CHARS, NEW_ACCESS_KEY_CHARS)
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
/// Characters in a key made by `kkt key create`, from the 36-character
/// alphabet: about 258 bits.
const NEW_ACCESS_KEY_CHARS: usize = 50;
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

/// The operator web key always lives beside the server's access key.
pub fn web_key_path(access_key: &Path) -> PathBuf {
    access_key.with_file_name(WEB_KEY_FILE_NAME)
}

/// The admin key lives beside the server's access key, unless that place
/// is already used by the access key (which older versions allowed).
pub fn admin_key_path(access_key: &Path) -> Option<PathBuf> {
    let admin = access_key.with_file_name(ADMIN_KEY_FILE_NAME);
    (admin != access_key).then_some(admin)
}

/// The admin agent's name, if the admin key has one, lives beside the admin
/// key, in a file of its own: each key file has its own name file.
pub fn admin_name_path(admin_key: &Path) -> PathBuf {
    let mut name = admin_key.file_name().unwrap_or_default().to_os_string();
    name.push(ADMIN_NAME_FILE_SUFFIX);
    admin_key.with_file_name(name)
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
    fn generated_keys_are_accepted_and_differ() {
        use kkt_entropy::testing::SeqEntropy;
        let entropy = SeqEntropy::new(3);
        let first = generate(&entropy);
        let second = generate(&entropy);
        assert_eq!(first.len(), NEW_ACCESS_KEY_CHARS);
        assert_ne!(first, second);
        assert_eq!(check_access_key(&first), Ok(()));
    }

    #[test]
    fn new_keys_are_drawn_from_lowercase_letters_and_digits() {
        let expected: Vec<u8> = (b'a'..=b'z').chain(b'0'..=b'9').collect();
        assert_eq!(RANDOM_CHARS, expected.as_slice());
    }
}
