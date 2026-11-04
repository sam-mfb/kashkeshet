//! Agent credential files: saving a new token or recovery code, reading one
//! that only its owner may read, and `kkt watch`'s state file. The only
//! user of `libc::geteuid`.

use kkt_config as config;
use kkt_diagnostics as diagnostics;
use kkt_watch as watch;
use std::path::{Path, PathBuf};

/// Where unit tests put their scratch files: shared with the integration tests.
#[cfg(all(test, unix))]
use kkt_scratch as scratch;

/// Reserve destinations before changing an avatar on the server. A failed operation
/// removes only the empty files this invocation created.
pub struct CredentialFile {
    path: PathBuf,
    file: std::fs::File,
    saved: bool,
}

impl CredentialFile {
    pub fn create(path: PathBuf) -> Result<Self, String> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|e| format!("could not create credential file {}: {e}", path.display()))?;
        Ok(Self {
            path,
            file,
            saved: false,
        })
    }

    pub fn save(&mut self, secret: &str) -> Result<(), String> {
        use std::io::Write;
        writeln!(self.file, "{secret}")
            .and_then(|()| self.file.sync_all())
            .map_err(|e| {
                format!(
                    "could not save credential file {}: {e}",
                    self.path.display()
                )
            })?;
        self.saved = true;
        Ok(())
    }
}

impl Drop for CredentialFile {
    fn drop(&mut self) {
        if !self.saved
            && let Err(e) = std::fs::remove_file(&self.path)
        {
            tracing::warn!(
                "could not remove incomplete credential file {}: {e}",
                self.path.display()
            );
        }
    }
}

/// Read an agent credential from the same file whose ownership and mode we check.
pub(crate) fn agent_secret(path: &Path) -> Result<String, String> {
    use std::io::Read;
    #[cfg(unix)]
    let uid = {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid takes no arguments and has no preconditions.
        let uid = unsafe { libc::geteuid() };
        // Give the ownership fix even when opening another user's file would fail.
        let metadata = std::fs::metadata(path)
            .map_err(|e| format!("could not inspect credential file {}: {e}", path.display()))?;
        check_agent_secret_owner(metadata.uid(), path, uid)?;
        uid
    };
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("could not open credential file {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        if make_agent_secret_private(&file, path, uid)? {
            eprintln!(
                "credential file {} was readable by others and is now private; if anyone else could have read it, get a new token with `reclaim`",
                path.display()
            );
        }
    }
    let mut secret = String::new();
    file.read_to_string(&mut secret)
        .map_err(|e| format!("could not read credential file {}: {e}", path.display()))?;
    Ok(secret)
}

#[cfg(unix)]
fn check_agent_secret_owner(owner: u32, path: &Path, uid: u32) -> Result<(), String> {
    if owner != uid {
        let quoted = diagnostics::shell_word(&path.to_string_lossy());
        return Err(format!(
            "credential file {} is owned by another user (uid {owner}); ask its owner or an administrator to run chown {uid} {quoted}, then chmod 600 {quoted}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn make_agent_secret_private(file: &std::fs::File, path: &Path, uid: u32) -> Result<bool, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = file
        .metadata()
        .map_err(|e| format!("could not inspect credential file {}: {e}", path.display()))?;
    check_agent_secret_owner(metadata.uid(), path, uid)?;
    let mode = metadata.permissions().mode();
    if mode & 0o077 == 0 {
        return Ok(false);
    }
    // Remove others' access without adding any owner permission.
    let quoted = diagnostics::shell_word(&path.to_string_lossy());
    file.set_permissions(std::fs::Permissions::from_mode(mode & !0o077))
        .map_err(|e| {
            format!(
                "could not make credential file {} private: {e}; run chmod {:o} {quoted}",
                path.display(),
                mode & 0o700
            )
        })?;
    Ok(true)
}

/// The agent's token: from a private file, or else from the environment.
pub fn watch_token(file: Option<&Path>) -> Result<String, String> {
    let token = match file {
        Some(path) => agent_secret(path)?,
        None => std::env::var(config::WATCH_TOKEN_ENV).map_err(|_| {
            format!(
                "give the agent's token in --token-file or the {} environment variable",
                config::WATCH_TOKEN_ENV
            )
        })?,
    };
    let token = token.trim();
    if token.is_empty() {
        return Err("the agent's token is empty".into());
    }
    Ok(token.to_string())
}

/// `kkt watch --state-file`: the highest message ID seen, as text.
pub struct StateFile(pub PathBuf);

impl watch::Place for StateFile {
    fn load(&self) -> Result<Option<i64>, String> {
        let text = match std::fs::read_to_string(&self.0) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(format!(
                    "could not read state file {}: {e}",
                    self.0.display()
                ));
            }
        };
        let text = text.trim();
        // An empty file, such as one made with `touch`, holds no ID yet.
        if text.is_empty() {
            return Ok(None);
        }
        text.parse().map(Some).map_err(|_| {
            format!(
                "state file {} does not hold a message ID; remove it to start after the latest message",
                self.0.display()
            )
        })
    }

    fn save(&self, id: i64) -> Result<(), String> {
        // Written beside it and renamed, so a stop halfway leaves the old ID.
        let mut temporary = self.0.clone().into_os_string();
        temporary.push(".tmp");
        std::fs::write(&temporary, format!("{id}\n"))
            .and_then(|()| std::fs::rename(&temporary, &self.0))
            .map_err(|e| format!("could not write state file {}: {e}", self.0.display()))
    }
}

#[cfg(all(test, unix))]
mod agent_secret_tests {
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
}
