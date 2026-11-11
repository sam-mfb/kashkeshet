//! Agent credential files: saving a new token or recovery code, reading one
//! that only its owner may read, `kkt watch`'s state file, and the lock that
//! keeps two processes from writing one of these files at once. The only
//! user of `libc::geteuid`.

use kkt_diagnostics as diagnostics;
use kkt_terminal_os::Terminal;
use kkt_watch as watch;
use std::path::{Path, PathBuf};

/// The environment variable `kkt watch` reads the agent's token from when
/// no `--token-file` is given.
const WATCH_TOKEN_ENV: &str = "KKT_TOKEN";

/// Where unit tests put their scratch files: shared with the integration tests.
#[cfg(all(test, unix))]
use kkt_scratch as scratch;

/// Reserve destinations before changing an avatar on the server. A failed operation
/// removes only the empty files this invocation created.
pub(crate) struct CredentialFile {
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
            Terminal.errors(format_args!(
                "credential file {} was readable by others and is now private; if anyone else could have read it, get a new token with `reclaim`\n",
                path.display()
            ));
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
pub(crate) fn watch_token(file: Option<&Path>) -> Result<String, String> {
    let token = match file {
        Some(path) => agent_secret(path)?,
        None => std::env::var(WATCH_TOKEN_ENV).map_err(|_| {
            format!(
                "give the agent's token in --token-file or the {} environment variable",
                WATCH_TOKEN_ENV
            )
        })?,
    };
    let token = token.trim();
    if token.is_empty() {
        return Err("the agent's token is empty".into());
    }
    Ok(token.to_string())
}

/// One process's exclusive hold on a file that it alone may write, such as
/// `kkt watch`'s state file. The lock is taken on a separate `<file>.lock`
/// beside it, so the file itself can still be replaced by renaming. The hold
/// lasts until this is dropped or the process ends, however it ends. The lock
/// file is left in place: removing it could let two processes each lock a
/// different file of the same name.
pub(crate) struct FileLock {
    _file: std::fs::File,
}

/// Why a [`FileLock`] was not taken.
#[derive(Debug)]
pub(crate) enum LockRefused {
    /// Another process holds it now.
    Held,
    /// The lock file could not be made or locked; the text says why.
    Failed(String),
}

impl FileLock {
    /// Take the lock for `path` at once, or refuse: it never waits.
    pub(crate) fn take(path: &Path) -> Result<Self, LockRefused> {
        let lock_path = lock_path(path);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&lock_path).map_err(|e| {
            LockRefused::Failed(format!(
                "could not open lock file {}: {e}",
                lock_path.display()
            ))
        })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(LockRefused::Held),
            Err(std::fs::TryLockError::Error(e)) => Err(LockRefused::Failed(format!(
                "could not lock {}: {e}",
                lock_path.display()
            ))),
        }
    }
}

/// `<path>.lock`, the file [`FileLock`] locks for `path`.
fn lock_path(path: &Path) -> PathBuf {
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    PathBuf::from(lock)
}

/// `kkt watch --state-file`: the highest message ID seen, as text. Only one
/// watcher at a time may use a state file; it holds the file's lock while it
/// runs.
pub(crate) struct StateFile {
    path: PathBuf,
    _lock: FileLock,
}

impl StateFile {
    /// Claim `path` for this watcher, refusing it if another watcher has it.
    pub(crate) fn claim(path: PathBuf) -> Result<Self, String> {
        match FileLock::take(&path) {
            Ok(lock) => Ok(Self { path, _lock: lock }),
            Err(LockRefused::Held) => Err(format!(
                "state file {} is in use by another kkt watch; stop that one, or give each watcher its own --state-file",
                path.display()
            )),
            Err(LockRefused::Failed(cause)) => Err(format!(
                "could not claim state file {}: {cause}",
                path.display()
            )),
        }
    }
}

impl watch::Place for StateFile {
    fn load(&self) -> Result<Option<i64>, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(format!(
                    "could not read state file {}: {e}",
                    self.path.display()
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
                self.path.display()
            )
        })
    }

    fn save(&self, id: i64) -> Result<(), String> {
        // Written beside it and renamed, so a stop halfway leaves the old ID.
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        std::fs::write(&temporary, format!("{id}\n"))
            .and_then(|()| std::fs::rename(&temporary, &self.path))
            .map_err(|e| format!("could not write state file {}: {e}", self.path.display()))
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

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn a_second_lock_is_refused_until_the_first_is_dropped() {
        let dir = kkt_scratch::Dir::new("file-lock");
        let path = dir.join("state");
        let first = FileLock::take(&path).expect("first lock");
        assert!(dir.join("state.lock").exists());
        assert!(matches!(FileLock::take(&path), Err(LockRefused::Held)));
        drop(first);
        FileLock::take(&path).expect("lock after the first is dropped");
    }

    #[test]
    fn a_claimed_state_file_is_refused_to_a_second_watcher_by_name() {
        let dir = kkt_scratch::Dir::new("state-file-claim");
        let path = dir.join("state");
        let _first = StateFile::claim(path.clone()).expect("first claim");
        let error = StateFile::claim(path.clone()).err().expect("second claim");
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(error.contains("in use by another kkt watch"), "{error}");
    }
}
