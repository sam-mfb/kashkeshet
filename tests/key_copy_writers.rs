//! The key copies' writer, on real files: setup and upgrade write a key
//! copy with kkt-keys-os' `OsKeyFiles::write_key`, forced. Each case is held
//! to the bytes, mode, outcome and error text that the writer the container
//! split retired gave in it (kkt-container-docker's own private write, fed
//! the trimmed key and a line's end), so an install's key copies are what
//! they always were.

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

/// Writes the key with `OsKeyFiles::write_key`, forced, in the case
/// `prepare` makes. Returns what it left and the error text a person sees,
/// with the scratch folder, which differs from run to run, named `<dir>`.
fn write(name: &str, prepare: impl Fn(&Path) -> PathBuf) -> (Outcome, Option<String>) {
    let dir = scratch::Dir::new(&format!("kkt-key-writer-{name}"));
    let base = dir.path().join("new");
    let path = prepare(&base);
    let written = OsKeyFiles.write_key(&path, KEY, true);
    let here = base.display().to_string();
    let error = written.as_ref().err().map(|e| e.replace(&here, "<dir>"));
    (
        Outcome {
            worked: written.is_ok(),
            left: left(&path),
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

/// The mode a key copy is left at: only its owner may read it.
#[cfg(unix)]
const PRIVATE: u32 = 0o600;
#[cfg(not(unix))]
const PRIVATE: u32 = 0;

/// A key copy as it is written: the key and a line's end, private.
fn written() -> Outcome {
    Outcome {
        worked: true,
        left: Left::File {
            bytes: format!("{KEY}\n").into_bytes(),
            mode: PRIVATE,
        },
    }
}

/// Whether this process may write a file its mode makes read-only, as the
/// superuser may.
#[cfg(unix)]
fn may_write_read_only_files() -> bool {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch::Dir::new("kkt-key-writer-probe");
    let path = dir.path().join("probe");
    std::fs::write(&path, "").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::OpenOptions::new().write(true).open(&path).is_ok()
}

#[test]
fn a_new_copy_is_the_key_and_a_line_end_kept_private() {
    let (outcome, error) = write("new", |base| {
        std::fs::create_dir_all(base).unwrap();
        base.join("copy")
    });
    assert_eq!(outcome, written());
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn a_copy_over_another_text_is_replaced_whole() {
    let (outcome, error) = write("other-text", |base| {
        with_file(
            base,
            "an older and much longer key text than the new one\n",
            0o600,
        )
    });
    assert_eq!(outcome, written());
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn a_copy_over_a_file_others_may_read_is_made_private() {
    let (outcome, error) = write("loose", |base| with_file(base, "old\n", 0o644));
    assert_eq!(outcome, written());
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn a_copy_over_a_read_only_file_fails_and_leaves_it() {
    let (outcome, error) = write("read-only", |base| with_file(base, "old\n", 0o400));
    if may_write_read_only_files() {
        assert_eq!(outcome, written());
        assert_eq!(error, None);
    } else {
        assert_eq!(
            outcome,
            Outcome {
                worked: false,
                left: Left::File {
                    bytes: b"old\n".to_vec(),
                    mode: 0o400
                },
            }
        );
        assert_eq!(
            error.as_deref(),
            Some("could not write <dir>/copy: Permission denied (os error 13)")
        );
    }
}

#[test]
fn a_copy_whose_folder_is_missing_fails_and_leaves_nothing() {
    let (outcome, error) = write("no-folder", |base| base.join("missing").join("copy"));
    assert_eq!(
        outcome,
        Outcome {
            worked: false,
            left: Left::Nothing
        }
    );
    assert_eq!(
        error.as_deref(),
        Some("could not write <dir>/missing/copy: No such file or directory (os error 2)")
    );
}

#[test]
fn a_copy_whose_path_is_a_folder_fails_and_leaves_the_folder() {
    let (outcome, error) = write("a-folder", |base| {
        let path = base.join("copy");
        std::fs::create_dir_all(&path).unwrap();
        path
    });
    assert_eq!(
        outcome,
        Outcome {
            worked: false,
            left: Left::Folder
        }
    );
    assert_eq!(
        error.as_deref(),
        Some("could not write <dir>/copy: Is a directory (os error 21)")
    );
}
