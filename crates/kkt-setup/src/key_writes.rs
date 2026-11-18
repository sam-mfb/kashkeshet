//! Writing key files: a new random key, a given key, and the name file that
//! ties an admin agent's name to its key, over the key files and the
//! randomness the caller gives.

use kkt_names::AdminName;
use std::path::Path;

/// Ties `name` to the new admin key `key`, just written to the file at
/// `path`, in the name file beside it. The name file holds a hash of the
/// key, so a name never passes to another key: one left from an earlier key
/// names nothing. With no name, such a file is removed. Returns how the
/// write went; the caller words it.
pub fn write_admin_name(
    files: &dyn kkt_keys::KeyFiles,
    path: &Path,
    key: &str,
    name: Option<&AdminName>,
) -> Result<(), String> {
    let bound = name.map(|name| kkt_keys::bound_to(name, key));
    files
        .write_admin_name(path, bound.as_deref())
        .map_err(|e| e.to_string())
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
