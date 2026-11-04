//! Backups and atomic replacement; Claude Code writes its own user config.
//! Also the parts of [`Client`] that look at the disk: where its config is,
//! whether it is installed, and whether it is connected.

use super::{Client, formats};
use crate::config::{self, ACCESS_KEY_FILE_MODE};
use crate::settings::Env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

impl Client {
    /// Whether the client looks installed: its config file exists, or its
    /// program is found by `on_path`.
    pub fn installed(self, env: &Env, on_path: &dyn Fn(&str) -> bool) -> bool {
        let program = match self {
            Client::ClaudeCode => Some(config::CLAUDE_PROGRAM),
            Client::Codex => Some(config::CODEX_PROGRAM),
            Client::OpenCode => Some(config::OPENCODE_PROGRAM),
            Client::MuseCode => Some(config::MUSE_PROGRAM),
            // Antigravity is an editor; its config file is the sign.
            Client::Agy => None,
        };
        program.is_some_and(on_path) || self.file(env).is_ok_and(|path| path.exists())
    }

    /// Whether the client's config ([`Client::file`]) has a kkt entry now,
    /// whatever server it names. A missing config has none; one that
    /// cannot be read or parsed is an error naming it.
    pub fn connected(self, env: &Env) -> Result<bool, String> {
        let path = self.file(env)?;
        let Some(text) = read(&path)? else {
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
    pub fn file(self, env: &Env) -> Result<PathBuf, String> {
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
                if !home.join(config::AGY_CONFIG).exists()
                    && home.join(config::AGY_LEGACY_CONFIG).exists()
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

pub(super) fn read(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(ACCESS_KEY_FILE_MODE);
    }
    options
}

fn backup(path: &Path, text: Option<&str>) -> Result<(), String> {
    let Some(text) = text else { return Ok(()) };
    for number in 0u64.. {
        let suffix = if number == 0 {
            ".bak".to_string()
        } else {
            format!(".bak.{number}")
        };
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let name = PathBuf::from(name);
        match private_options().open(&name) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|e| format!("could not back up {}: {e}", path.display()))?;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not back up {}: {e}", path.display())),
        }
    }
    Err("no unused backup name".to_string())
}

pub(super) fn restrict(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(ACCESS_KEY_FILE_MODE))
            .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
    }
    Ok(())
}

fn atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("config path has no parent")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".kkt-{}", std::process::id()));
    let temporary = PathBuf::from(name);
    let mut file = private_options()
        .open(&temporary)
        .map_err(|e| format!("could not prepare {}: {e}", path.display()))?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = written {
        fs::remove_file(&temporary).map_err(|cleanup| {
            format!(
                "could not write {}: {error}; could not remove temporary file: {cleanup}",
                path.display()
            )
        })?;
        return Err(format!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

pub(super) fn replace(path: &Path, original: Option<&str>, updated: &str) -> Result<(), String> {
    backup(path, original)?;
    atomic(path, updated)
}

fn claude(args: &[&str], config_dir: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new("claude");
    command.args(args);

    if let Some(dir) = config_dir {
        command.env("CLAUDE_CONFIG_DIR", dir);
    } else {
        command.env_remove("CLAUDE_CONFIG_DIR");
    }

    let result = command.output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            "claude was not found on PATH; install Claude Code, then retry --write".to_string()
        } else {
            format!("could not run claude: {e}")
        }
    })?;
    if !result.status.success() {
        // The CLI may echo credentials in its output. Report only the status.
        return Err(format!("claude mcp failed with {}", result.status));
    }
    Ok(())
}

/// Apply a Claude Code change through its CLI. `client_config` is absolute;
/// `source` is the setting that chose it, named in file errors.
pub(super) fn apply_claude(
    client_config: &Path,
    original: Option<&str>,
    updated: &str,
    config_dir: Option<&Path>,
    source: &str,
) -> Result<(), String> {
    let from = |error: String| format!("{error} (from {source})");
    let entry = super::formats::claude_entry(updated)?;
    let replacing = match original {
        Some(text) => super::formats::has_claude_entry(text)?,
        None => false,
    };
    backup(client_config, original).map_err(from)?;

    let applied = (|| {
        if replacing {
            claude(&["mcp", "remove", "--scope", "user", "kkt"], config_dir)?;
        }
        claude(
            &["mcp", "add-json", "--scope", "user", "kkt", &entry],
            config_dir,
        )
    })();
    if let Err(error) = applied {
        match original {
            Some(text) => atomic(client_config, text)
                .map_err(|e| from(format!("{error}; could not restore: {e}")))?,
            None if client_config.exists() => fs::remove_file(client_config).map_err(|e| {
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
