//! Agent credentials and watch state on disk: the token and recovery-code
//! files `kkt register` and `kkt reclaim` write, the token file `kkt watch`
//! reads, and its state file. The program supplies the credential-file rule
//! and where its warnings are sent.

mod error;

pub use error::{AgentFileError, CredentialAction, LockError, StateAction};
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};

const WATCH_TOKEN_ENV: &str = "KKT_TOKEN";

/// The inspected file and the caller whose access is checked.
#[derive(Clone, Copy)]
pub struct CredentialFileFacts {
    /// The file owner's Unix uid.
    pub owner: u32,
    /// The complete Unix permission mode reported by metadata.
    pub mode: u32,
    /// The effective uid of the process reading the file.
    pub caller_uid: u32,
}

/// The caller's rule over the path and named file/caller facts.
pub type CredentialCheck =
    fn(&Path, CredentialFileFacts) -> Result<Option<PrivateMode>, Box<dyn Error + Send + Sync>>;

/// A checked permission change, with caller-supplied advice and warning.
pub struct PrivateMode {
    /// The exact mode the caller's rule selected.
    pub mode: u32,
    /// Advice appended if setting the permissions fails.
    pub advice: String,
    /// What the caller reports after setting the permissions.
    pub warning: String,
}

/// Reserve destinations before changing an avatar on the server. A failed operation
/// removes only the empty files this invocation created.
pub struct CredentialFile {
    path: PathBuf,
    file: std::fs::File,
    saved: bool,
    on_cleanup_error: fn(&Path, &io::Error),
}

impl CredentialFile {
    /// Reserves a new private file; reports failed cleanup through the callback.
    pub fn create(
        path: PathBuf,
        on_cleanup_error: fn(&Path, &io::Error),
    ) -> Result<Self, AgentFileError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|e| AgentFileError::credential(CredentialAction::Create, &path, e))?;
        Ok(Self {
            path,
            file,
            saved: false,
            on_cleanup_error,
        })
    }

    /// Writes and syncs the secret before keeping the reserved file.
    pub fn save(&mut self, secret: &str) -> Result<(), AgentFileError> {
        use std::io::Write;
        writeln!(self.file, "{secret}")
            .and_then(|()| self.file.sync_all())
            .map_err(|e| AgentFileError::credential(CredentialAction::Save, &self.path, e))?;
        self.saved = true;
        Ok(())
    }
}

impl Drop for CredentialFile {
    fn drop(&mut self) {
        if !self.saved
            && let Err(e) = std::fs::remove_file(&self.path)
        {
            (self.on_cleanup_error)(&self.path, &e);
        }
    }
}

/// Reads from the same opened file whose owner and mode the caller checks.
/// The check receives the path and named file/caller facts.
/// A warning is delivered after tightening and before reading the secret.
pub(crate) fn read_secret(
    path: &Path,
    check: CredentialCheck,
    on_private: fn(&str),
) -> Result<String, AgentFileError> {
    use std::io::Read;
    // These callbacks are used only on systems with Unix ownership/modes.
    let _ = (&check, &on_private);
    #[cfg(unix)]
    let uid = {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid takes no arguments and has no preconditions.
        let uid = unsafe { libc::geteuid() };
        // Give the ownership fix even when opening another user's file would fail.
        let metadata = std::fs::metadata(path)
            .map_err(|e| AgentFileError::credential(CredentialAction::Inspect, path, e))?;
        // Validate first; permission changes use the opened handle below.
        let _ = check(
            path,
            CredentialFileFacts {
                owner: metadata.uid(),
                mode: metadata.permissions().mode(),
                caller_uid: uid,
            },
        )
        .map_err(AgentFileError::Refused)?;
        uid
    };
    let mut file = std::fs::File::open(path)
        .map_err(|e| AgentFileError::credential(CredentialAction::Open, path, e))?;
    #[cfg(unix)]
    if let Some(warning) = make_private(&file, path, uid, check)? {
        on_private(&warning);
    }
    let mut secret = String::new();
    file.read_to_string(&mut secret)
        .map_err(|e| AgentFileError::credential(CredentialAction::Read, path, e))?;
    Ok(secret)
}

/// Applies the caller's credential-file rule to this opened handle. Its
/// owner is rechecked before permissions change, and owner access is retained.
#[cfg(unix)]
pub fn make_private(
    file: &std::fs::File,
    path: &Path,
    uid: u32,
    check: CredentialCheck,
) -> Result<Option<String>, AgentFileError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = file
        .metadata()
        .map_err(|e| AgentFileError::credential(CredentialAction::Inspect, path, e))?;
    let Some(change) = check(
        path,
        CredentialFileFacts {
            owner: metadata.uid(),
            mode: metadata.permissions().mode(),
            caller_uid: uid,
        },
    )
    .map_err(AgentFileError::Refused)?
    else {
        return Ok(None);
    };
    file.set_permissions(std::fs::Permissions::from_mode(change.mode))
        .map_err(|cause| AgentFileError::NotPrivate {
            path: path.to_path_buf(),
            cause,
            advice: change.advice,
        })?;
    Ok(Some(change.warning))
}

/// The agent's token: from a private file, or else from the environment.
pub fn watch_token(
    file: Option<&Path>,
    check: CredentialCheck,
    on_private: fn(&str),
) -> Result<String, AgentFileError> {
    let token = match file {
        Some(path) => read_secret(path, check, on_private)?,
        None => std::env::var(WATCH_TOKEN_ENV).map_err(|_| AgentFileError::NoToken {
            variable: WATCH_TOKEN_ENV,
        })?,
    };
    let token = token.trim();
    if token.is_empty() {
        return Err(AgentFileError::EmptyToken);
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
pub enum LockRefused {
    /// Another process holds it now.
    Held,
    /// The lock file could not be made or locked.
    Failed(LockError),
}

impl FileLock {
    /// Take the lock for `path` at once, or refuse: it never waits.
    pub fn take(path: &Path) -> Result<Self, LockRefused> {
        let lock_path = lock_path(path);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&lock_path).map_err(|cause| {
            LockRefused::Failed(LockError::Open {
                lock: lock_path.clone(),
                cause,
            })
        })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(LockRefused::Held),
            Err(std::fs::TryLockError::Error(cause)) => Err(LockRefused::Failed(LockError::Lock {
                lock: lock_path,
                cause,
            })),
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
pub struct StateFile {
    path: PathBuf,
    _lock: FileLock,
}

impl StateFile {
    /// Claim `path` for this watcher, refusing it if another watcher has it.
    pub fn claim(path: PathBuf) -> Result<Self, AgentFileError> {
        match FileLock::take(&path) {
            Ok(lock) => Ok(Self { path, _lock: lock }),
            Err(LockRefused::Held) => Err(AgentFileError::StateInUse { path }),
            Err(LockRefused::Failed(cause)) => Err(AgentFileError::NotClaimed { path, cause }),
        }
    }
}

impl StateFile {
    /// Reads the highest seen ID, or None for a missing or empty state file.
    pub fn load(&self) -> Result<Option<i64>, AgentFileError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(cause) => {
                return Err(AgentFileError::State {
                    action: StateAction::Read,
                    path: self.path.clone(),
                    cause,
                });
            }
        };
        let text = text.trim();
        // An empty file, such as one made with `touch`, holds no ID yet.
        if text.is_empty() {
            return Ok(None);
        }
        text.parse().map(Some).map_err(|_| AgentFileError::NotAnId {
            path: self.path.clone(),
        })
    }

    /// Writes the ID beside the state file, then renames it into place.
    pub fn save(&self, id: i64) -> Result<(), AgentFileError> {
        // Written beside it and renamed, so a stop halfway leaves the old ID.
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        std::fs::write(&temporary, format!("{id}\n"))
            .and_then(|()| std::fs::rename(&temporary, &self.path))
            .map_err(|cause| AgentFileError::State {
                action: StateAction::Write,
                path: self.path.clone(),
                cause,
            })
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn credential_checks_bracket_open_and_finish_before_reading() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CHECKS: AtomicUsize = AtomicUsize::new(0);

        fn remove_before_open(
            path: &Path,
            _: CredentialFileFacts,
        ) -> Result<Option<PrivateMode>, Box<dyn Error + Send + Sync>> {
            CHECKS.fetch_add(1, Ordering::SeqCst);
            std::fs::remove_file(path).expect("remove before open");
            Ok(None)
        }
        fn refuse_opened_handle(
            _: &Path,
            _: CredentialFileFacts,
        ) -> Result<Option<PrivateMode>, Box<dyn Error + Send + Sync>> {
            if CHECKS.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(None)
            } else {
                Err("the opened handle was refused".into())
            }
        }
        fn no_warning(_: &str) {
            panic!("no permission change requested");
        }

        let dir = kkt_scratch::Dir::new("credential-check-order");
        let path = dir.join("credential");
        std::fs::write(&path, "private-credential").expect("write");
        let error = read_secret(&path, remove_before_open, no_warning)
            .expect_err("file removed before open")
            .to_string();
        assert!(
            error.starts_with("could not open credential file"),
            "{error}"
        );
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);

        CHECKS.store(0, Ordering::SeqCst);
        // Reading this as text would fail: the second check must win first.
        std::fs::write(&path, b"private-credential\xff").expect("write");
        assert_eq!(
            read_secret(&path, refuse_opened_handle, no_warning).map_err(|e| e.to_string()),
            Err("the opened handle was refused".into())
        );
        assert_eq!(CHECKS.load(Ordering::SeqCst), 2);
    }

    /// Every message kkt-agent-files words that a test can cause without
    /// root, pinned exactly: kkt's words around the system's own cause,
    /// which each case gets by doing the same thing itself.
    #[test]
    fn every_agent_file_message_says_what_it_always_said() {
        fn allow(
            _: &Path,
            _: CredentialFileFacts,
        ) -> Result<Option<PrivateMode>, Box<dyn Error + Send + Sync>> {
            Ok(None)
        }
        fn quiet(_: &str) {}
        fn kept(_: &Path, _: &io::Error) {}
        let dir = kkt_scratch::Dir::new("agent-file-messages");

        // A credential file that is already there is not replaced.
        let taken = dir.join("taken");
        std::fs::write(&taken, "old").expect("write");
        let cause = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&taken)
            .expect_err("there");
        assert_eq!(
            CredentialFile::create(taken.clone(), kept)
                .err()
                .expect("there")
                .to_string(),
            format!(
                "could not create credential file {}: {cause}",
                taken.display()
            )
        );

        // A missing token file cannot be inspected.
        let missing = dir.join("missing");
        let cause = std::fs::metadata(&missing).expect_err("missing");
        #[cfg(unix)]
        let verb = "inspect";
        #[cfg(not(unix))]
        let verb = "open";
        assert_eq!(
            watch_token(Some(&missing), allow, quiet).map_err(|e| e.to_string()),
            Err(format!(
                "could not {verb} credential file {}: {cause}",
                missing.display()
            ))
        );

        // A token file that is not text cannot be read.
        let binary = dir.join("binary");
        std::fs::write(&binary, b"token\xff").expect("write");
        let cause = std::fs::read_to_string(&binary).expect_err("not text");
        assert_eq!(
            watch_token(Some(&binary), allow, quiet).map_err(|e| e.to_string()),
            Err(format!(
                "could not read credential file {}: {cause}",
                binary.display()
            ))
        );

        // An empty token.
        let empty = dir.join("empty");
        std::fs::write(&empty, " \n").expect("write");
        assert_eq!(
            watch_token(Some(&empty), allow, quiet).map_err(|e| e.to_string()),
            Err("the agent's token is empty".to_string())
        );

        // A claimed state file.
        let state = dir.join("state");
        let _first = StateFile::claim(state.clone()).expect("first claim");
        assert_eq!(
            StateFile::claim(state.clone())
                .err()
                .expect("second claim")
                .to_string(),
            format!(
                "state file {} is in use by another kkt watch; stop that one, or give each watcher its own --state-file",
                state.display()
            )
        );

        // A state file whose lock cannot be made: its folder is a file.
        let under_file = taken.join("state");
        let lock = taken.join("state.lock");
        let cause = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .expect_err("not a folder");
        assert_eq!(
            StateFile::claim(under_file.clone())
                .err()
                .expect("no lock")
                .to_string(),
            format!(
                "could not claim state file {}: could not open lock file {}: {cause}",
                under_file.display(),
                lock.display()
            )
        );

        // A state file that holds no ID.
        std::fs::write(&state, "not an id").expect("write");
        let claimed = StateFile::claim(dir.join("other"))
            .map(|mut file| {
                file.path = state.clone();
                file
            })
            .expect("claim");
        assert_eq!(
            claimed.load().map_err(|e| e.to_string()),
            Err(format!(
                "state file {} does not hold a message ID; remove it to start after the latest message",
                state.display()
            ))
        );

        // A state file that cannot be read: it is a folder.
        let folder = dir.join("folder");
        std::fs::create_dir(&folder).expect("folder");
        let cause = std::fs::read_to_string(&folder).expect_err("a folder");
        let claimed = StateFile::claim(dir.join("third"))
            .map(|mut file| {
                file.path = folder.clone();
                file
            })
            .expect("claim");
        assert_eq!(
            claimed.load().map_err(|e| e.to_string()),
            Err(format!(
                "could not read state file {}: {cause}",
                folder.display()
            ))
        );

        // A state file that cannot be written: its folder is a file.
        let cause = std::fs::write(taken.join("x.tmp"), "1\n").expect_err("not a folder");
        let claimed = StateFile::claim(dir.join("fourth"))
            .map(|mut file| {
                file.path = taken.join("x");
                file
            })
            .expect("claim");
        assert_eq!(
            claimed.save(1).map_err(|e| e.to_string()),
            Err(format!(
                "could not write state file {}: {cause}",
                taken.join("x").display()
            ))
        );
    }

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
        let error = StateFile::claim(path.clone())
            .err()
            .expect("second claim")
            .to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(error.contains("in use by another kkt watch"), "{error}");
    }
}
