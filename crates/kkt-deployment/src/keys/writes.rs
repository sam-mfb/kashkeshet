//! Writing key files: a new random key, and the name file that ties an
//! admin agent's name to its key, over the key files and the randomness the
//! caller gives. A given key is written with kkt-keys' `KeyFiles::write_key`.

use kkt_keys::AdminName;
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

#[cfg(test)]
mod tests {
    //! What the key writers leave, pinned: a new key's file, and the name
    //! file beside an admin key.

    use super::*;
    use kkt_entropy::testing::SeqEntropy;
    use kkt_keys::MAX_ADMIN_NAME_CHARS;
    use kkt_keys::testing::Memory;

    #[test]
    fn a_new_key_file_holds_the_key_it_returns() {
        let files = Memory::new().with_dir("/k");
        let path = Path::new("/k/access-key");
        let key = new_key_file(&files, &SeqEntropy::new(0), path, false).expect("written");
        assert_eq!(key.chars().count(), kkt_keys::NEW_KEY_CHARS);
        assert!(key.bytes().all(|b| kkt_keys::NEW_KEY_ALPHABET.contains(&b)));
        let file = files.file(path).expect("a file");
        assert_eq!(file.text, format!("{key}\n"));
        assert_eq!(file.mode, Some(kkt_keys::ACCESS_KEY_FILE_MODE));
        assert_eq!(
            new_key_file(&files, &SeqEntropy::new(1), path, false),
            Err(format!(
                "{} already exists; pass --force to replace it",
                path.display()
            ))
        );
        let again = new_key_file(&files, &SeqEntropy::new(1), path, true).expect("replaced");
        assert_eq!(files.file(path).expect("a file").text, format!("{again}\n"));
    }

    #[test]
    fn an_admin_name_is_bound_to_its_key_and_removed_with_none() {
        let files = Memory::new().with_dir("/k");
        let path = Path::new("/k/admin-key");
        let name = AdminName::new("sentinel", MAX_ADMIN_NAME_CHARS).expect("a name");
        assert_eq!(
            write_admin_name(&files, path, "the-key", Some(&name)),
            Ok(())
        );
        let names = kkt_keys::admin_name_path(path);
        assert_eq!(
            files.file(&names).expect("a name file").text,
            kkt_keys::bound_to(&name, "the-key")
        );
        assert_eq!(write_admin_name(&files, path, "the-key", None), Ok(()));
        assert_eq!(files.file(&names), None);
    }
}
