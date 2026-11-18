//! Writing key files: a new random key, over the key files and the
//! randomness the caller gives. A given key is written with kkt-keys'
//! `KeyFiles::write_key`.

use std::path::Path;

/// Makes a new random key and writes it to `path`, creating its folder if
/// needed. Returns the key.
pub fn new_key_file(
    files: &dyn kkt_keys::KeyFiles,
    entropy: &dyn kkt_entropy::Entropy,
    path: &Path,
    force: bool,
) -> Result<String, String> {
    files
        .new_key(path, force, &mut || {
            entropy.random_chars(kkt_keys::NEW_KEY_ALPHABET, kkt_keys::NEW_KEY_CHARS)
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! What the key writers leave, pinned: a new key's file, and the name
    //! file beside an admin key.

    use super::*;
    use kkt_entropy::testing::SeqEntropy;
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
}
