//! What a person is told about an admin key's name, from what kkt-keys reads
//! of the name file beside it: `kkt status`, `kkt key show` and `kkt key
//! create` print these lines.

use kkt_keys::{
    ADMIN_KEY_COPY_FILE_NAME, AdminNameFile, KeyFiles, admin_name_path, read_admin_name,
};
use std::ffi::OsStr;
use std::path::Path;

/// What to say, after `label`, about the name tied to the admin key `key`
/// in the file at `path`, if anything, reading the name file beside it from
/// `files`. A Docker setup's host copy has no name beside it, as the name
/// is kept in the container.
pub fn admin_name_line(
    files: &dyn KeyFiles,
    path: &Path,
    key: &str,
    label: &str,
) -> Result<Option<String>, String> {
    let docker_host_copy = path.file_name() == Some(OsStr::new(ADMIN_KEY_COPY_FILE_NAME));
    name_line(path, key, label, docker_host_copy, |names| {
        files.read_optional(names).map_err(|e| {
            format!(
                "invalid setting: the admin name could not be read from {}: {e}",
                names.display()
            )
        })
    })
}

/// [`admin_name_line`], over the name file `read` reads.
fn name_line(
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
/// written for the admin name `name`, or removed with no name: `written` is
/// how that went.
pub fn admin_name_written(
    path: &Path,
    name: Option<&str>,
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
    //! Every line a person is told about an admin key's name, pinned byte
    //! for byte: `kkt status`, `kkt key show`, `kkt key create` and `kkt
    //! setup` print these.

    use super::*;
    use kkt_keys::testing::Memory;
    use kkt_names::AdminName;

    const KEY: &str = "the-admin-key";

    fn name(text: &str) -> AdminName {
        AdminName::new(text, kkt_names::MAX_ADMIN_NAME_CHARS).expect("a name")
    }

    /// Writes the name file as `kkt key create` and `kkt setup` do.
    fn write(
        files: &Memory,
        path: &Path,
        key: &str,
        name: Option<&AdminName>,
    ) -> Result<(), String> {
        let bound = name.map(|name| kkt_keys::bound_to(name, key));
        files
            .write_admin_name(path, bound.as_deref())
            .map_err(|e| e.to_string())
    }

    /// What `kkt key create` and `kkt setup` say after writing the name.
    fn written(
        files: &Memory,
        path: &Path,
        key: &str,
        name: Option<&AdminName>,
    ) -> Result<String, String> {
        let text = name.map(ToString::to_string);
        admin_name_written(path, text.as_deref(), write(files, path, key, name))
    }

    #[test]
    fn the_admin_name_lines_say_what_they_always_said() {
        let path = Path::new("/k/admin-key");
        let named = Memory::new().with_dir("/k");
        write(&named, path, KEY, Some(&name("sentinel"))).expect("written");
        let other = Memory::new().with_dir("/k");
        write(&other, path, "another-key", Some(&name("sentinel"))).expect("written");
        let lines: Vec<_> = [&named, &Memory::new(), &other]
            .into_iter()
            .map(|files| admin_name_line(files, path, KEY, "admin name").expect("read"))
            .collect();
        assert_eq!(
            lines,
            [
                Some("admin name: admin-sentinel".to_string()),
                Some(
                    "admin name: none, so the key may only read; `kkt key create --admin \
                     --force --name <name>` makes a named one"
                        .to_string()
                ),
                Some(
                    "admin name: none, so the key may only read: /k/admin-key.name names a \
                     different key; `kkt key create --admin --force --name <name>` makes a \
                     named one"
                        .to_string()
                ),
            ]
        );
        // A Docker setup's host copy keeps no name beside it.
        let copy = Path::new("/k").join(kkt_keys::ADMIN_KEY_COPY_FILE_NAME);
        assert_eq!(admin_name_line(&named, &copy, KEY, "Admin name"), Ok(None));
    }

    #[test]
    fn writing_a_name_says_what_it_always_said() {
        let path = Path::new("/k/admin-key");
        let files = Memory::new().with_dir("/k");
        assert_eq!(
            written(&files, path, KEY, Some(&name("sentinel"))),
            Ok(
                "Its admin agent is named admin-sentinel: its changes are announced as \
                \"operator (via admin-sentinel)\"."
                    .to_string()
            )
        );
        assert_eq!(
            written(&files, path, KEY, None),
            Ok(
                "It has no name, so it may only read. To let it make changes, make it again \
                with --force --name <name>."
                    .to_string()
            )
        );
        let failing = Memory::new().with_dir("/k").failing("/k/admin-key.name");
        assert_eq!(
            written(&failing, path, KEY, Some(&name("sentinel"))),
            Err(
                "the key was written, but its name could not be: could not write \
                 /k/admin-key.name: refused. It has no name, so it may only read. To let \
                 it make changes, make it again with --force --name <name>."
                    .to_string()
            )
        );
        assert_eq!(
            written(&failing, path, KEY, None),
            Ok(
                "It has no name, so it may only read. To let it make changes, make it again \
                with --force --name <name>. The name file of an earlier key, \
                /k/admin-key.name, could not be removed (refused); it does not name this key."
                    .to_string()
            )
        );
    }

    #[test]
    fn a_docker_host_copy_never_reads_an_adjacent_name_file() {
        assert_eq!(
            name_line(Path::new("copy"), "key", "Admin name", true, |_| panic!(
                "no name beside the host copy"
            )),
            Ok(None)
        );
    }
}
