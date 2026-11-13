//! The name tied to an admin key: its bound file text and operator advice.
use kkt_names::AdminName;
use std::path::{Path, PathBuf};

/// The text of a name file that ties this name to the admin key `key`
/// alone: the name, then a line with a hash of the key (never the key).
/// Surrounding whitespace is not part of the key, as for `SharedKey`.
pub fn bound_to(name: &AdminName, key: &str) -> String {
    format!("{}\n{KEY_HASH_LABEL}{}\n", name, key_hash(key))
}

/// The name in the name file `text` if it is tied to the admin key
/// `key`; `None` if it is tied to another key, as when the key file was
/// replaced without its name. An error says what is wrong with `text`.
/// The name is checked as [`AdminName::new`] checks it, with at most
/// `max_chars`.
pub fn bound_in(text: &str, key: &str, max_chars: usize) -> Result<Option<AdminName>, String> {
    let mut lines = text.lines();
    let name = AdminName::new(lines.next().unwrap_or(""), max_chars)?;
    let Some(hash) = lines.next().and_then(|l| l.strip_prefix(KEY_HASH_LABEL)) else {
        return Err(format!(
            "its second line must be `{KEY_HASH_LABEL}` and a hash of the admin key"
        ));
    };
    if lines.any(|l| !l.trim().is_empty()) {
        return Err("it must have only the name and the key's hash".into());
    }
    Ok((hash.trim() == key_hash(key)).then_some(name))
}

/// How a name file's line with the key's hash starts.
const KEY_HASH_LABEL: &str = "key-sha256: ";

/// A SHA-256 of `key`, without surrounding whitespace, in hex. A fixed
/// prefix keeps it from equalling a hash of the key made for anything else.
fn key_hash(key: &str) -> String {
    kkt_hash::sha256(format!("kkt-admin-name:{}", key.trim()).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What to say, after `label`, about the name tied to the admin key `key`
/// in the file at `path`, if anything: a Docker setup's host copy has no
/// name beside it, as the name is kept in the container.
pub fn admin_name_line(
    path: &Path,
    key: &str,
    label: &str,
    docker_host_copy: bool,
    read: impl FnOnce(&Path) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    if docker_host_copy {
        return Ok(None);
    }
    let make = "`kkt key create --admin --force --name <name>` makes a named one";
    Ok(Some(match read_admin_name(path, key, read)? {
        AdminNameFile::Named(name) => format!("{label}: {name}"),
        AdminNameFile::None => format!("{label}: none, so the key may only read; {make}"),
        AdminNameFile::OtherKey(names) => format!(
            "{label}: none, so the key may only read: {} names a different key; {make}",
            names.display()
        ),
    }))
}

/// What the name file beside an admin key says about that key.
enum AdminNameFile {
    /// The admin agent's name, tied to this key.
    Named(AdminName),
    /// There is no name file: the key may only read.
    None,
    /// The name file, at this path, is for a different key, as when the key
    /// file was replaced without its name: this key may only read.
    OtherKey(PathBuf),
}

/// The name tied to the admin key `key`, kept in the file at `path`, from
/// the name file beside it.
fn read_admin_name(
    path: &Path,
    key: &str,
    read: impl FnOnce(&Path) -> Result<Option<String>, String>,
) -> Result<AdminNameFile, String> {
    let names = crate::admin_name_path(path);
    match read(&names)? {
        Some(text) => match bound_in(&text, key, kkt_names::MAX_ADMIN_NAME_CHARS) {
            Ok(Some(name)) => Ok(AdminNameFile::Named(name)),
            Ok(None) => Ok(AdminNameFile::OtherKey(names)),
            Err(e) => Err(format!(
                "invalid setting: admin name in {}: {e}",
                names.display()
            )),
        },
        None => Ok(AdminNameFile::None),
    }
}

/// The unchanged advice after the native name-file write or removal.
pub fn admin_name_written(
    path: &Path,
    name: Option<&AdminName>,
    written: Result<(), String>,
) -> Result<String, String> {
    let names = crate::admin_name_path(path);
    let unnamed = "It has no name, so it may only read. To let it make changes, make it again with --force --name <name>.";
    match (name, written) {
        (Some(name), Ok(())) => Ok(format!(
            "Its admin agent is named {name}: its changes are announced as \"operator (via {name})\"."
        )),
        (Some(_), Err(e)) => Err(format!(
            "the key was written, but its name could not be: could not write {}: {e}. {unnamed}",
            names.display()
        )),
        (None, Ok(())) => Ok(unnamed.into()),
        (None, Err(e)) => Ok(format!(
            "{unnamed} The name file of an earlier key, {}, could not be removed ({e}); it does not name this key.",
            names.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAX_CHARS: usize = 40;
    #[test]
    fn an_admin_name_belongs_only_to_the_key_it_was_written_for() {
        let first = "0123456789abcdef0123456789abcdef";
        let second = "fedcba9876543210fedcba9876543210";
        let sentinel = AdminName::new("sentinel", MAX_CHARS).unwrap();
        let text = bound_to(&sentinel, first);
        assert!(text.starts_with("admin-sentinel\nkey-sha256: "), "{text}");
        assert!(!text.contains(first));

        assert_eq!(
            bound_in(&text, first, MAX_CHARS),
            Ok(Some(sentinel.clone()))
        );
        // A key file's final newline is not part of the key.
        assert_eq!(
            bound_in(&text, &format!("{first}\n"), MAX_CHARS),
            Ok(Some(sentinel))
        );
        // Another key, or the same file after the key was replaced, has none.
        assert_eq!(bound_in(&text, second, MAX_CHARS), Ok(None));

        // A name alone, as an older or hand-written file might hold, is an
        // error, not a name for whatever key is beside it.
        for bad in [
            "admin-sentinel\n",
            "",
            "Night Owl\nkey-sha256: 00\n",
            "admin-sentinel\nsha: 00\n",
            &format!("{text}extra\n"),
        ] {
            assert!(bound_in(bad, first, MAX_CHARS).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn bound_names_use_the_supplied_name_length_limit() {
        let text = bound_to(&AdminName::new("abc", 9).unwrap(), "key");
        assert!(bound_in(&text, "key", 9).is_ok());
        assert!(bound_in(&text, "key", 8).is_err());
    }
    #[test]
    fn a_docker_host_copy_never_reads_an_adjacent_name_file() {
        assert_eq!(
            admin_name_line(Path::new("copy"), "key", "Admin name", true, |_| panic!(
                "no name beside the host copy"
            )),
            Ok(None)
        );
    }
}
