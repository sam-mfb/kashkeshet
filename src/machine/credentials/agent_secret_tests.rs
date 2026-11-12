#![cfg(unix)]

use kkt_scratch as scratch;

use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
fn private_modes_are_untouched_and_tightening_never_adds_owner_permissions() {
    let dir = scratch::Dir::new("agent-secret-modes");
    let path = dir.join("secret");
    std::fs::write(&path, "private-credential").expect("write");
    let file = std::fs::File::open(&path).expect("open");
    let uid = file.metadata().expect("metadata").uid();
    for (before, after, changed) in [
        (0o600, 0o600, false),
        (0o400, 0o400, false),
        (0o644, 0o600, true),
        (0o444, 0o400, true),
        (0o640, 0o600, true),
    ] {
        file.set_permissions(std::fs::Permissions::from_mode(before))
            .expect("mode");
        assert_eq!(make_agent_secret_private(&file, &path, uid), Ok(changed));
        assert_eq!(file.metadata().expect("metadata").mode() & 0o777, after);
    }
}

#[test]
fn another_owners_file_is_refused_even_if_private() {
    let dir = scratch::Dir::new("agent-secret-owner");
    let path = dir.join("secret file");
    std::fs::write(&path, "private-credential").expect("write");
    let file = std::fs::File::open(&path).expect("open");
    let owner = file.metadata().expect("metadata").uid();
    // Inject a different caller uid: no privileged chown is needed.
    let caller = owner.wrapping_add(1);
    for mode in [0o600, 0o644] {
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .expect("mode");
        let error = make_agent_secret_private(&file, &path, caller).expect_err("owner");
        assert!(error.contains(&path.display().to_string()));
        assert!(error.contains("owned by another user"));
        assert!(error.contains(&format!("chown {caller} ")));
        assert!(error.contains("chmod 600"));
        assert!(!error.contains("private-credential"));
        assert_eq!(file.metadata().expect("metadata").mode() & 0o777, mode);
    }
}

fn make_agent_secret_private(file: &std::fs::File, path: &Path, uid: u32) -> Result<bool, String> {
    kkt_agent_files_os::make_private(file, path, uid, credential_check)
        .map(|warning| warning.is_some())
}
