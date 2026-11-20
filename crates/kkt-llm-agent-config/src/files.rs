//! Backups and atomic replacement; Claude Code writes its own user config.
//! Also the parts of [`Client`] that look at the disk: where its config is,
//! whether it is installed, and whether it is connected. All of it reaches
//! the machine through [`Host`].

use super::error::{ClientConfigError, FileAction, Folder};
use super::{
    AGY_CONFIG, AGY_LEGACY_CONFIG, CLAUDE_CONFIG, CLAUDE_PROGRAM, CODEX_CONFIG, CODEX_HOME_DEFAULT,
    CODEX_PROGRAM, Client, Host, MUSE_CONFIG, MUSE_CONFIG_DIR, MUSE_PROGRAM, OPENCODE_CONFIG,
    OPENCODE_PROGRAM, formats,
};
use kkt_machine::Env;
use std::io;
use std::path::{Path, PathBuf};

impl Client {
    /// Whether the client looks installed: its config file exists, or its
    /// program is found by `on_path`.
    pub fn installed(self, env: &Env, on_path: &dyn Fn(&str) -> bool, host: &dyn Host) -> bool {
        let program = match self {
            Client::ClaudeCode => Some(CLAUDE_PROGRAM),
            Client::Codex => Some(CODEX_PROGRAM),
            Client::OpenCode => Some(OPENCODE_PROGRAM),
            Client::MuseCode => Some(MUSE_PROGRAM),
            // Antigravity is an editor; its config file is the sign.
            Client::Agy => None,
        };
        program.is_some_and(on_path) || self.file(env, host).is_ok_and(|path| host.exists(&path))
    }

    /// Whether the client's config ([`Client::file`]) has a kkt entry now,
    /// whatever server it names. A missing config has none; one that
    /// cannot be read or parsed is an error naming it.
    pub fn connected(self, env: &Env, host: &dyn Host) -> Result<bool, ClientConfigError> {
        Ok(self.read_entry(env, host)?.is_some())
    }

    /// The MCP URL configured in this client's kkt entry, if present.
    pub fn configured_url(
        self,
        env: &Env,
        host: &dyn Host,
    ) -> Result<Option<String>, ClientConfigError> {
        Ok(self.read_entry(env, host)?.and_then(|entry| entry.url))
    }

    fn read_entry(
        self,
        env: &Env,
        host: &dyn Host,
    ) -> Result<Option<formats::KktEntry>, ClientConfigError> {
        let path = self.file(env, host)?;
        let Some(text) = read(host, &path)? else {
            return Ok(None);
        };
        formats::kkt_entry(self, &text).map_err(|e| ClientConfigError::OfClient {
            client: self,
            path: path.clone(),
            setting: self.source(env).to_string(),
            error: Box::new(e),
        })
    }

    /// The config file the client reads by default, following the client's
    /// own variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_CONFIG_HOME`)
    /// when it is set.
    pub fn file(self, env: &Env, host: &dyn Host) -> Result<PathBuf, ClientConfigError> {
        let home = || env.home.as_ref().ok_or(ClientConfigError::NoHome);
        Ok(match self {
            Self::ClaudeCode => match env.named(crate::CLAUDE_CONFIG_DIR) {
                Some(dir) => dir.join(CLAUDE_CONFIG),
                None => home()?.join(CLAUDE_CONFIG),
            },
            Self::Codex => match env.named(crate::CODEX_HOME) {
                Some(dir) => dir.join(CODEX_CONFIG),
                None => home()?.join(CODEX_HOME_DEFAULT).join(CODEX_CONFIG),
            },
            Self::Agy => {
                let home = home()?;
                if !host.exists(&home.join(AGY_CONFIG))
                    && host.exists(&home.join(AGY_LEGACY_CONFIG))
                {
                    home.join(AGY_LEGACY_CONFIG)
                } else {
                    home.join(AGY_CONFIG)
                }
            }
            Self::OpenCode => env
                .config_home()
                .ok_or(ClientConfigError::NoHome)?
                .join(OPENCODE_CONFIG),
            Self::MuseCode => env
                .config_home()
                .ok_or(ClientConfigError::NoHome)?
                .join(MUSE_CONFIG_DIR)
                .join(MUSE_CONFIG),
        })
    }
}

pub(super) fn read(host: &dyn Host, path: &Path) -> Result<Option<String>, ClientConfigError> {
    match host.read(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ClientConfigError::file(FileAction::Read, path, e)),
    }
}

/// Whether `entry_name` is a backup sibling of `base_name`: `<base>.bak` or
/// `<base>.bak.<N>` for a positive decimal integer `N`. Matches on native
/// encoded bytes so non-UTF-8 file names are handled without loss.
fn is_backup_name(base_name: &std::ffi::OsStr, entry_name: &std::ffi::OsStr) -> bool {
    let Some(rest) = entry_name
        .as_encoded_bytes()
        .strip_prefix(base_name.as_encoded_bytes())
    else {
        return false;
    };
    if rest == b".bak" {
        return true;
    }
    let Some(num) = rest.strip_prefix(b".bak.") else {
        return false;
    };
    !num.is_empty()
        && num.iter().all(u8::is_ascii_digit)
        && std::str::from_utf8(num)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n >= 1)
}

/// Saves `text`, the file's old contents, beside `path` as `<path>.bak`,
/// or `.bak.1`, `.bak.2` and on if that is taken. If any existing regular,
/// independent `.bak` or `.bak.N` beside `path` (even past a gap) already
/// holds the exact same `text`, no new backup file is created. Nothing to
/// save, nothing saved.
fn backup(host: &dyn Host, path: &Path, text: Option<&str>) -> Result<(), ClientConfigError> {
    let Some(text) = text else { return Ok(()) };
    if let Some(base_name) = path.file_name() {
        let dir = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        if let Ok(entries) = host.list_dir(dir) {
            for entry in entries {
                if is_backup_name(base_name, &entry) {
                    let candidate = dir.join(&entry);
                    if host
                        .read_independent_file(&candidate, path)
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some(text)
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
    for number in 0u64.. {
        let suffix = if number == 0 {
            ".bak".to_string()
        } else {
            format!(".bak.{number}")
        };
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let candidate = PathBuf::from(name);
        match host.create_private(&candidate, text) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if host
                    .read_independent_file(&candidate, path)
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(text)
                {
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(ClientConfigError::file(FileAction::BackUp, path, e)),
        }
    }
    Err(ClientConfigError::NoBackupName)
}

pub(super) fn restrict(host: &dyn Host, path: &Path) -> Result<(), ClientConfigError> {
    host.restrict(path)
        .map_err(|e| ClientConfigError::file(FileAction::Restrict, path, e))
}

/// Writes `text` to a private temporary file beside `path` and moves it
/// over `path`, so a reader sees the old file or the new one, never part
/// of either. The final mode is decided before the move and set on the
/// temporary file, so the file lands already correct: a key-holding file
/// (`private`) is 0600, a file that was there otherwise keeps the mode it
/// had, and a new file is private, as the temporary file is. A temporary
/// file left by a failure is removed.
fn atomic(
    host: &dyn Host,
    path: &Path,
    text: &str,
    private: bool,
) -> Result<(), ClientConfigError> {
    let parent = path
        .parent()
        .ok_or(ClientConfigError::NoFolder(Folder::ConfigPath))?;
    host.create_dir_all(parent)
        .map_err(|e| ClientConfigError::file(FileAction::Create, parent, e))?;
    let mode = match host.mode(path) {
        Ok(mode) => mode,
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(ClientConfigError::file(FileAction::Write, path, e)),
    };
    let final_mode = if private { Some(0o600) } else { mode };
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".kkt-{}", host.process_id()));
    let temporary = PathBuf::from(name);
    let written = match host.create_private(&temporary, text) {
        Err(e) if !host.exists(&temporary) => {
            return Err(ClientConfigError::file(FileAction::Prepare, path, e));
        }
        written => written.and_then(|()| {
            if let Some(mode) = final_mode {
                host.set_mode(&temporary, mode)?;
            }
            host.rename(&temporary, path)
        }),
    };
    if let Err(error) = written {
        if let Err(cleanup) = host.remove_file(&temporary) {
            return Err(ClientConfigError::TemporaryLeft {
                path: path.to_path_buf(),
                error,
                cleanup,
            });
        }
        return Err(ClientConfigError::file(FileAction::Write, path, error));
    }
    Ok(())
}

pub(super) fn replace(
    host: &dyn Host,
    path: &Path,
    original: Option<&str>,
    updated: &str,
    private: bool,
) -> Result<(), ClientConfigError> {
    backup(host, path, original)?;
    atomic(host, path, updated, private)
}

/// Apply a Claude Code change through its CLI. `client_config` is absolute;
/// `source` is the setting that chose it, named in file errors. If the CLI
/// fails, the config is put back as it was.
pub(super) fn apply_claude(
    host: &dyn Host,
    command: &dyn crate::ClientCommand,
    client_config: &Path,
    original: Option<&str>,
    updated: &str,
    config_dir: Option<&Path>,
    source: &str,
) -> Result<(), ClientConfigError> {
    let entry = formats::claude_entry(updated)?;
    let replacing = match original {
        Some(text) => formats::has_claude_entry(text)?,
        None => false,
    };
    backup(host, client_config, original).map_err(|e| e.with_setting(source))?;

    let applied = (|| {
        if replacing {
            command.remove_server(config_dir)?;
        }
        command.add_server(&entry, config_dir)
    })();
    let Err(failed) = applied else {
        return Ok(());
    };
    // Put the config back, then say what failed, with the undoing's own
    // failure beside it if that failed too.
    let error = Box::new(ClientConfigError::Command(failed));
    Err(match original {
        // The restored config may hold a key, so it is private from the
        // moment it lands, as a key-holding file is.
        Some(text) => match atomic(host, client_config, text, true) {
            Ok(()) => *error,
            Err(restore) => ClientConfigError::NotRestored {
                error,
                restore: Box::new(restore),
            }
            .with_setting(source),
        },
        None if host.exists(client_config) => match host.remove_file(client_config) {
            Ok(()) => *error,
            Err(cause) => ClientConfigError::NotRemoved {
                error,
                path: client_config.to_path_buf(),
                cause,
            }
            .with_setting(source),
        },
        None => *error,
    })
}
