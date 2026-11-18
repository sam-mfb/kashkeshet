//! The managed folder's settings file's writer, on real files: setup and
//! upgrade write it through Docker's `Projects::write`, as a private file.
//! Each case is held to the bytes, mode, outcome and error text that the
//! writer the container split retired gave in it (kkt-container-docker's
//! own private write), including an upgrade that writes the new version
//! and then puts the old settings back, so an install's settings file is
//! what it always was.

use kkt_container::{Folder, Project, ProjectFile, Projects};
use kkt_container_docker::Docker;
use kkt_scratch as scratch;
use std::path::Path;

/// The settings file as setup writes it, with an auth key, which is why it
/// is private.
const SETTINGS: &str = "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
KKT_VERSION=0.4.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\n\
TS_HOSTNAME=kkt-1\nTS_TAILNET=tail1234.ts.net\nTS_AUTHKEY=tskey-auth-k123456CNTRL-abcdefghijklmnop\n";

/// The same file once upgrade has selected another version.
const UPGRADED: &str = "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
KKT_VERSION=0.5.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\n\
TS_HOSTNAME=kkt-1\nTS_TAILNET=tail1234.ts.net\nTS_AUTHKEY=tskey-auth-k123456CNTRL-abcdefghijklmnop\n";

/// The settings file's name in the managed folder.
const SETTINGS_FILE: &str = ".env";

/// What the writes left: whether they worked, and what is at the path after.
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

/// Writes `texts` in turn into the settings file of a managed folder that
/// `prepare` makes, through Docker's `Projects::write`, stopping at a write
/// that fails. Returns what was left and the failed write's error text,
/// with the scratch folder, which differs from run to run, named `<dir>`.
fn write(name: &str, prepare: impl Fn(&Path), texts: &[&str]) -> (Outcome, Option<String>) {
    let dir = scratch::Dir::new(&format!("kkt-settings-writer-{name}"));
    let managed = dir.path().join("new");
    prepare(&managed);
    let path = managed.join(SETTINGS_FILE);
    let docker = Docker::new();
    let folder = Folder(managed.clone());
    let project = Project {
        folder: &folder,
        shielded: &[],
    };
    let mut error = None;
    for text in texts {
        if let Err(e) = docker.write(
            &project,
            &[ProjectFile {
                path: Path::new(SETTINGS_FILE),
                text,
                private: true,
            }],
        ) {
            error = Some(e.to_string());
            break;
        }
    }
    let here = managed.display().to_string();
    let error = error.map(|e| e.replace(&here, "<dir>"));
    (
        Outcome {
            worked: error.is_none(),
            left: left(&path),
        },
        error,
    )
}

/// A managed folder with a settings file holding `text` at `mode`.
#[cfg(unix)]
fn with_settings(text: &'static str, mode: u32) -> impl Fn(&Path) {
    move |folder| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(folder).unwrap();
        let path = folder.join(SETTINGS_FILE);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

/// The mode the settings file is left at: only its owner may read it.
#[cfg(unix)]
const PRIVATE: u32 = 0o600;
#[cfg(not(unix))]
const PRIVATE: u32 = 0;

/// A settings file written with `text`, private.
fn written(text: &str) -> Outcome {
    Outcome {
        worked: true,
        left: Left::File {
            bytes: text.as_bytes().to_vec(),
            mode: PRIVATE,
        },
    }
}

/// Whether this process may write a file its mode makes read-only, as the
/// superuser may.
#[cfg(unix)]
fn may_write_read_only_files() -> bool {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch::Dir::new("kkt-settings-writer-probe");
    let path = dir.path().join("probe");
    std::fs::write(&path, "").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::OpenOptions::new().write(true).open(&path).is_ok()
}

fn empty_folder(folder: &Path) {
    std::fs::create_dir_all(folder).unwrap();
}

#[test]
fn a_new_settings_file_is_its_text_kept_private() {
    let (outcome, error) = write("new", empty_folder, &[SETTINGS]);
    assert_eq!(outcome, written(SETTINGS));
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn an_upgraded_settings_file_is_replaced_whole() {
    let (outcome, error) = write("upgraded", with_settings(SETTINGS, 0o600), &[UPGRADED]);
    assert_eq!(outcome, written(UPGRADED));
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn a_settings_file_put_back_after_a_failed_pull_is_the_old_text_exactly() {
    // Upgrade selects the new version, the pull fails, and the old
    // settings are written back.
    let (outcome, error) = write(
        "put-back",
        with_settings(SETTINGS, 0o600),
        &[UPGRADED, SETTINGS],
    );
    assert_eq!(outcome, written(SETTINGS));
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn a_settings_file_others_may_read_is_made_private() {
    let (outcome, error) = write("loose", with_settings(SETTINGS, 0o644), &[UPGRADED]);
    assert_eq!(outcome, written(UPGRADED));
    assert_eq!(error, None);
}

#[cfg(unix)]
#[test]
fn a_read_only_settings_file_fails_and_is_left() {
    let (outcome, error) = write("read-only", with_settings(SETTINGS, 0o400), &[UPGRADED]);
    if may_write_read_only_files() {
        assert_eq!(outcome, written(UPGRADED));
        assert_eq!(error, None);
    } else {
        assert_eq!(
            outcome,
            Outcome {
                worked: false,
                left: Left::File {
                    bytes: SETTINGS.as_bytes().to_vec(),
                    mode: 0o400
                },
            }
        );
        assert_eq!(
            error.as_deref(),
            Some("could not write <dir>/.env: Permission denied (os error 13)")
        );
    }
}

#[test]
fn a_settings_file_whose_folder_is_missing_fails_and_leaves_nothing() {
    let (outcome, error) = write("no-folder", |_: &Path| {}, &[SETTINGS]);
    assert_eq!(
        outcome,
        Outcome {
            worked: false,
            left: Left::Nothing
        }
    );
    assert_eq!(
        error.as_deref(),
        Some("could not write <dir>/.env: No such file or directory (os error 2)")
    );
}

#[test]
fn a_settings_file_that_is_a_folder_fails_and_leaves_the_folder() {
    let (outcome, error) = write(
        "a-folder",
        |folder: &Path| std::fs::create_dir_all(folder.join(SETTINGS_FILE)).unwrap(),
        &[SETTINGS],
    );
    assert_eq!(
        outcome,
        Outcome {
            worked: false,
            left: Left::Folder
        }
    );
    assert_eq!(
        error.as_deref(),
        Some("could not write <dir>/.env: Is a directory (os error 21)")
    );
}
