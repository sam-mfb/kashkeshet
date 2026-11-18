//! The managed folder's settings file's writer, before and after the
//! container split: setup and upgrade wrote it with kkt-container-docker's
//! own private write, and now write it through Docker's `Projects::write`,
//! as a private file. On real files, both must leave the same bytes and
//! mode, succeed or fail alike, and say the same to a person, in each case
//! the file can meet; and an upgrade that writes the new version and then
//! puts the old settings back must end the same from both.

use kkt_container::{Folder, Project, ProjectFile, Projects};
use kkt_container_docker::testing::write_private_file;
use kkt_container_docker::{Browser, Docker};
use kkt_scratch as scratch;
use std::io;
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

/// A browser no write opens.
struct NoBrowser;

impl Browser for NoBrowser {
    fn open(&self, _: &str) -> io::Result<()> {
        unreachable!("writing a file opens no link")
    }
    fn open_failure(&self) -> Option<String> {
        None
    }
}

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

/// The error text each writer gives a person, with the managed folder the
/// same in both: the old one as setup and upgrade worded it (`could not
/// write <path>: <cause>`, kkt-container-docker's `DockerError::io`), the
/// new one as `ProjectError` words a failed write.
#[derive(Debug, PartialEq, Eq)]
struct Errors {
    old: Option<String>,
    new: Option<String>,
}

/// Writes `texts` in turn into the settings file of a managed folder that
/// `prepare` makes, once with the old writer and once through Docker's
/// `Projects::write`, each in a folder of its own, stopping at a write that
/// fails. Returns both outcomes and the error texts of the write that
/// failed, if one did.
fn both(name: &str, prepare: impl Fn(&Path), texts: &[&str]) -> (Outcome, Outcome, Errors) {
    let dir = scratch::Dir::new(&format!("kkt-settings-writers-{name}"));
    let old_folder = dir.path().join("old");
    let new_folder = dir.path().join("new");
    prepare(&old_folder);
    prepare(&new_folder);
    let old_path = old_folder.join(SETTINGS_FILE);
    let new_path = new_folder.join(SETTINGS_FILE);

    let mut old_error = None;
    for text in texts {
        if let Err(e) = write_private_file(&old_path, text) {
            // Setup and upgrade worded it so. The two folders differ, so the
            // old text names the new folder's file, to compare the rest.
            old_error = Some(format!("could not write {}: {e}", new_path.display()));
            break;
        }
    }

    let docker = Docker::new(Box::new(NoBrowser));
    let folder = Folder(new_folder.clone());
    let project = Project {
        folder: &folder,
        shielded: &[],
    };
    let mut new_error = None;
    for text in texts {
        if let Err(e) = docker.write(
            &project,
            &[ProjectFile {
                path: Path::new(SETTINGS_FILE),
                text,
                private: true,
            }],
        ) {
            new_error = Some(e.to_string());
            break;
        }
    }
    let errors = Errors {
        old: old_error,
        new: new_error,
    };
    eprintln!("{name}: {errors:?}");
    (
        Outcome {
            worked: errors.old.is_none(),
            left: left(&old_path),
        },
        Outcome {
            worked: errors.new.is_none(),
            left: left(&new_path),
        },
        errors,
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

fn empty_folder(folder: &Path) {
    std::fs::create_dir_all(folder).unwrap();
}

#[test]
fn a_new_settings_file_is_the_same_from_both_writers() {
    let (old, new, errors) = both("new", empty_folder, &[SETTINGS]);
    assert!(old.worked);
    assert_eq!(old, new);
    assert_eq!(errors.old, errors.new);
}

#[cfg(unix)]
#[test]
fn an_upgraded_settings_file_is_the_same_from_both_writers() {
    let (old, new, errors) = both("upgraded", with_settings(SETTINGS, 0o600), &[UPGRADED]);
    assert!(old.worked);
    assert_eq!(old, new);
    assert_eq!(errors.old, errors.new);
}

#[cfg(unix)]
#[test]
fn a_settings_file_put_back_after_a_failed_pull_is_the_same_from_both_writers() {
    // Upgrade selects the new version, the pull fails, and the old
    // settings are written back.
    let (old, new, errors) = both(
        "put-back",
        with_settings(SETTINGS, 0o600),
        &[UPGRADED, SETTINGS],
    );
    assert!(old.worked);
    assert_eq!(old, new);
    assert_eq!(errors.old, errors.new);
    assert_eq!(
        new.left,
        Left::File {
            bytes: SETTINGS.as_bytes().to_vec(),
            mode: 0o600
        }
    );
}

#[cfg(unix)]
#[test]
fn a_settings_file_others_may_read_is_made_private_by_both_writers() {
    let (old, new, errors) = both("loose", with_settings(SETTINGS, 0o644), &[UPGRADED]);
    assert!(old.worked);
    assert_eq!(old, new);
    assert_eq!(errors.old, errors.new);
}

#[cfg(unix)]
#[test]
fn a_read_only_settings_file_ends_the_same_from_both_writers() {
    let (old, new, errors) = both("read-only", with_settings(SETTINGS, 0o400), &[UPGRADED]);
    assert_eq!(old, new);
    assert_eq!(errors.old, errors.new);
}

#[test]
fn a_settings_file_whose_folder_is_missing_fails_alike_and_leaves_nothing() {
    let (old, new, errors) = both("no-folder", |_: &Path| {}, &[SETTINGS]);
    assert!(!old.worked);
    assert_eq!(old, new);
    assert_eq!(new.left, Left::Nothing);
    assert!(errors.new.is_some());
    assert_eq!(errors.old, errors.new);
}

#[test]
fn a_settings_file_that_is_a_folder_fails_alike_and_leaves_the_folder() {
    let (old, new, errors) = both(
        "a-folder",
        |folder: &Path| std::fs::create_dir_all(folder.join(SETTINGS_FILE)).unwrap(),
        &[SETTINGS],
    );
    assert!(!old.worked);
    assert_eq!(old, new);
    assert_eq!(new.left, Left::Folder);
    assert!(errors.new.is_some());
    assert_eq!(errors.old, errors.new);
}
