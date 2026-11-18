//! What a person is told about an admin key's name, from what kkt-keys reads
//! of the name file beside it: `kkt status`, `kkt key show` and `kkt key
//! create` print these lines.

use kkt_keys::{AdminNameFile, admin_name_path, read_admin_name};
use kkt_names::AdminName;
use std::path::Path;

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

/// The advice after the name file beside the admin key at `path` is
/// written for `name`, or removed with no name: `written` is how that went.
pub fn admin_name_written(
    path: &Path,
    name: Option<&AdminName>,
    written: Result<(), String>,
) -> Result<String, String> {
    let names = admin_name_path(path);
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
