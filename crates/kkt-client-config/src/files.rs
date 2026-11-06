//! Backups and atomic replacement; Claude Code writes its own user config.
//! Also the parts of [`Client`] that look at the disk: where its config is,
//! whether it is installed, and whether it is connected. All of it reaches
//! the machine through [`Host`].

use super::{Client, Host, formats};
use kkt_config as config;
use kkt_settings::Env;
use std::io;
use std::path::{Path, PathBuf};

impl Client {
    /// Whether the client looks installed: its config file exists, or its
    /// program is found by `on_path`.
    pub fn installed(self, env: &Env, on_path: &dyn Fn(&str) -> bool, host: &dyn Host) -> bool {
        let program = match self {
            Client::ClaudeCode => Some(config::CLAUDE_PROGRAM),
            Client::Codex => Some(config::CODEX_PROGRAM),
            Client::OpenCode => Some(config::OPENCODE_PROGRAM),
            Client::MuseCode => Some(config::MUSE_PROGRAM),
            // Antigravity is an editor; its config file is the sign.
            Client::Agy => None,
        };
        program.is_some_and(on_path) || self.file(env, host).is_ok_and(|path| host.exists(&path))
    }

    /// Whether the client's config ([`Client::file`]) has a kkt entry now,
    /// whatever server it names. A missing config has none; one that
    /// cannot be read or parsed is an error naming it.
    pub fn connected(self, env: &Env, host: &dyn Host) -> Result<bool, String> {
        let path = self.file(env, host)?;
        let Some(text) = read(host, &path)? else {
            return Ok(false);
        };
        formats::has_kkt_entry(self, &text).map_err(|e| {
            format!(
                "{}: {} (from {}): {e}",
                self.display_name(),
                path.display(),
                self.source(env)
            )
        })
    }

    /// The config file the client reads by default, following the client's
    /// own variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_CONFIG_HOME`)
    /// when it is set.
    pub fn file(self, env: &Env, host: &dyn Host) -> Result<PathBuf, String> {
        let home = || {
            env.home
                .as_ref()
                .ok_or("set HOME to find the client's config")
        };
        Ok(match self {
            Self::ClaudeCode => match &env.claude_config_dir {
                Some(dir) => dir.join(config::CLAUDE_CONFIG),
                None => home()?.join(config::CLAUDE_CONFIG),
            },
            Self::Codex => match &env.codex_home {
                Some(dir) => dir.join(config::CODEX_CONFIG),
                None => home()?
                    .join(config::CODEX_HOME_DEFAULT)
                    .join(config::CODEX_CONFIG),
            },
            Self::Agy => {
                let home = home()?;
                if !host.exists(&home.join(config::AGY_CONFIG))
                    && host.exists(&home.join(config::AGY_LEGACY_CONFIG))
                {
                    home.join(config::AGY_LEGACY_CONFIG)
                } else {
                    home.join(config::AGY_CONFIG)
                }
            }
            Self::OpenCode => env
                .config_home()
                .ok_or("set HOME to find the client's config")?
                .join(config::OPENCODE_CONFIG),
            Self::MuseCode => env
                .config_home()
                .ok_or("set HOME to find the client's config")?
                .join(config::MUSE_CONFIG_DIR)
                .join(config::MUSE_CONFIG),
        })
    }
}

pub(super) fn read(host: &dyn Host, path: &Path) -> Result<Option<String>, String> {
    match host.read(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// Saves `text`, the file's old contents, beside `path` as `<path>.bak`,
/// or `.bak.1`, `.bak.2` and on if that is taken. Nothing to save, nothing
/// saved.
fn backup(host: &dyn Host, path: &Path, text: Option<&str>) -> Result<(), String> {
    let Some(text) = text else { return Ok(()) };
    for number in 0u64.. {
        let suffix = if number == 0 {
            ".bak".to_string()
        } else {
            format!(".bak.{number}")
        };
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match host.create_private(&PathBuf::from(name), text) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not back up {}: {e}", path.display())),
        }
    }
    Err("no unused backup name".to_string())
}

pub(super) fn restrict(host: &dyn Host, path: &Path) -> Result<(), String> {
    host.restrict(path)
        .map_err(|e| format!("could not restrict {}: {e}", path.display()))
}

/// Writes `text` to a private temporary file beside `path` and moves it
/// over `path`, so a reader sees the old file or the new one, never part
/// of either. A temporary file left by a failure is removed.
fn atomic(host: &dyn Host, path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("config path has no parent")?;
    host.create_dir_all(parent)
        .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".kkt-{}", host.process_id()));
    let temporary = PathBuf::from(name);
    let written = match host.create_private(&temporary, text) {
        Err(e) if !host.exists(&temporary) => {
            return Err(format!("could not prepare {}: {e}", path.display()));
        }
        written => written.and_then(|()| host.rename(&temporary, path)),
    };
    if let Err(error) = written {
        host.remove_file(&temporary).map_err(|cleanup| {
            format!(
                "could not write {}: {error}; could not remove temporary file: {cleanup}",
                path.display()
            )
        })?;
        return Err(format!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

pub(super) fn replace(
    host: &dyn Host,
    path: &Path,
    original: Option<&str>,
    updated: &str,
) -> Result<(), String> {
    backup(host, path, original)?;
    atomic(host, path, updated)
}

/// Apply a Claude Code change through its CLI. `client_config` is absolute;
/// `source` is the setting that chose it, named in file errors. If the CLI
/// fails, the config is put back as it was.
pub(super) fn apply_claude(
    host: &dyn Host,
    client_config: &Path,
    original: Option<&str>,
    updated: &str,
    config_dir: Option<&Path>,
    source: &str,
) -> Result<(), String> {
    let from = |error: String| format!("{error} (from {source})");
    let entry = formats::claude_entry(updated)?;
    let replacing = match original {
        Some(text) => formats::has_claude_entry(text)?,
        None => false,
    };
    backup(host, client_config, original).map_err(from)?;

    let applied = (|| {
        if replacing {
            host.claude(&["mcp", "remove", "--scope", "user", "kkt"], config_dir)?;
        }
        host.claude(
            &["mcp", "add-json", "--scope", "user", "kkt", &entry],
            config_dir,
        )
    })();
    if let Err(error) = applied {
        match original {
            Some(text) => atomic(host, client_config, text)
                .map_err(|e| from(format!("{error}; could not restore: {e}")))?,
            None if host.exists(client_config) => host.remove_file(client_config).map_err(|e| {
                from(format!(
                    "{error}; could not restore {}: {e}",
                    client_config.display()
                ))
            })?,
            None => (),
        }
        return Err(error);
    }
    Ok(())
}
