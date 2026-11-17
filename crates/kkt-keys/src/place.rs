//! Where the server's key files are, what they are called, and their mode.
//! These are fixed values that installs already hold files under.

use std::path::{Path, PathBuf};

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
/// is already used by the access key (an install may have one there).
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
