//! Writing key files: a new random key, a given key, and the name file that
//! ties an admin agent's name to its key, over the key files and the
//! randomness the caller gives.

use kkt_names::AdminName;
use std::path::Path;

/// What to say, after `label`, about the name tied to the admin key `key`
/// in the file at `path`, if anything.
pub fn admin_name_line(
    files: &dyn kkt_keys::KeyFiles,
    path: &Path,
    key: &str,
    label: &str,
) -> Result<Option<String>, String> {
    let docker_host_copy =
        path.file_name() == Some(std::ffi::OsStr::new(kkt_container::ADMIN_KEY_FILE_NAME));
    kkt_keys::admin_name_line(path, key, label, docker_host_copy, |names| {
        files.read_optional(names).map_err(|e| {
            format!(
                "invalid setting: the admin name could not be read from {}: {e}",
                names.display()
            )
        })
    })
}

/// Ties `name` to the new admin key `key`, just written to the file at
/// `path`, in the name file beside it. The name file holds a hash of the
/// key, so a name never passes to another key: one left from an earlier key
/// names nothing. With no name, such a file is removed. Returns what to tell
/// the person.
pub fn write_admin_name(
    files: &dyn kkt_keys::KeyFiles,
    path: &Path,
    key: &str,
    name: Option<&AdminName>,
) -> Result<String, String> {
    let bound = name.map(|name| kkt_keys::bound_to(name, key));
    let written = files
        .write_admin_name(path, bound.as_deref())
        .map_err(|e| e.to_string());
    kkt_keys::admin_name_written(path, name, written)
}

/// Makes a new random key and writes it to `path`, creating its folder if
/// needed. Returns the key.
pub fn new_key_file(
    files: &dyn kkt_keys::KeyFiles,
    entropy: &dyn kkt_entropy::Entropy,
    path: &Path,
    force: bool,
) -> Result<String, String> {
    files.new_key(path, force, &mut || {
        entropy.random_chars(kkt_keys::NEW_KEY_ALPHABET, kkt_keys::NEW_KEY_CHARS)
    })
}

pub fn write_key_file(
    files: &dyn kkt_keys::KeyFiles,
    path: &Path,
    key: &str,
    force: bool,
) -> Result<(), String> {
    files.write_key(path, key, force)
}
