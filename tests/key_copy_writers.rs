//! The key copies' writer, before and after the container split: setup
//! wrote a key copy with kkt-container-docker's own private write, and now
//! writes it with kkt-keys-os' `OsKeyFiles::write_key`, forced. On real
//! files, both must leave the same bytes and mode, and succeed or fail
//! alike, in each case a copy can meet.

use kkt_container_docker::testing::write_private_file;
use kkt_keys::KeyFiles;
use kkt_keys_os::OsKeyFiles;
use kkt_scratch as scratch;
use std::path::{Path, PathBuf};

const KEY: &str = "access0123456789abcdef0123456789abcdef0123456789";

/// What a write left: whether it worked, and what is at the path after.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    worked: bool,
    left: Left,
}

#[derive(Debug, PartialEq, Eq)]
enum Left {
    Nothing,
    Folder,
    File { bytes: Vec<u8>, mode: u32 },
}

fn left(path: &Path) -> Left {
    match std::fs::symlink_metadata(path) {
        Err(_) => Left::Nothing,
        Ok(meta) if meta.is_dir() => Left::Folder,
        Ok(meta) => Left::File {
            bytes: std::fs::read(path).unwrap_or_default(),
            mode: mode_of(&meta),
        },
    }
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(_: &std::fs::Metadata) -> u32 {
    0
}

/// Runs each writer on its own copy of the case `prepare` makes, and
/// returns both outcomes, with the new writer's error text when it fails.
fn both(name: &str, prepare: impl Fn(&Path) -> PathBuf) -> (Outcome, Outcome, Option<String>) {
    let dir = scratch::Dir::new(&format!("kkt-key-writers-{name}"));
    let old_path = prepare(&dir.path().join("old"));
    let new_path = prepare(&dir.path().join("new"));
    // Docker wrote the trimmed key and a line's end.
    let old = write_private_file(&old_path, &format!("{KEY}\n"));
    let new = OsKeyFiles.write_key(&new_path, KEY, true);
    let error = new.as_ref().err().cloned();
    if let Err(e) = &old {
        eprintln!("{name}: the old writer failed: {e}");
    }
    if let Some(e) = &error {
        eprintln!("{name}: the new writer failed: {e}");
    }
    (
        Outcome {
            worked: old.is_ok(),
            left: left(&old_path),
        },
        Outcome {
            worked: new.is_ok(),
            left: left(&new_path),
        },
        error,
    )
}

/// A folder `base` with a file `copy` in it holding `text` at `mode`.
#[cfg(unix)]
fn with_file(base: &Path, text: &str, mode: u32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(base).unwrap();
    let path = base.join("copy");
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn a_new_copy_is_the_same_from_both_writers() {
    let (old, new, _) = both("new", |base| {
        std::fs::create_dir_all(base).unwrap();
        base.join("copy")
    });
    assert!(old.worked);
    assert_eq!(old, new);
}

#[cfg(unix)]
#[test]
fn a_copy_over_another_text_is_the_same_from_both_writers() {
    let (old, new, _) = both("other-text", |base| {
        with_file(
            base,
            "an older and much longer key text than the new one\n",
            0o600,
        )
    });
    assert!(old.worked);
    assert_eq!(old, new);
}

#[cfg(unix)]
#[test]
fn a_copy_over_a_file_others_may_read_is_made_private_by_both_writers() {
    let (old, new, _) = both("loose", |base| with_file(base, "old\n", 0o644));
    assert!(old.worked);
    assert_eq!(old, new);
}

#[cfg(unix)]
#[test]
fn a_copy_over_a_read_only_file_ends_the_same_from_both_writers() {
    let (old, new, error) = both("read-only", |base| with_file(base, "old\n", 0o400));
    eprintln!("read-only: old {old:?}, new {new:?}, new error {error:?}");
    assert_eq!(old, new);
}

#[test]
fn a_copy_whose_folder_is_missing_fails_alike_and_leaves_nothing() {
    let (old, new, error) = both("no-folder", |base| base.join("missing").join("copy"));
    assert!(!old.worked);
    assert_eq!(old, new);
    assert_eq!(new.left, Left::Nothing);
    assert!(error.unwrap().starts_with("could not write "));
}

#[test]
fn a_copy_whose_path_is_a_folder_fails_alike_and_leaves_the_folder() {
    let (old, new, error) = both("a-folder", |base| {
        let path = base.join("copy");
        std::fs::create_dir_all(&path).unwrap();
        path
    });
    assert!(!old.worked);
    assert_eq!(old, new);
    assert_eq!(new.left, Left::Folder);
    assert!(error.unwrap().starts_with("could not write "));
}
