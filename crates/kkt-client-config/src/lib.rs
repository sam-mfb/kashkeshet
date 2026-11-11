//! Client configuration and its on-disk formats. Helpers refer to the key
//! file; only Antigravity and Muse Code require a copy of the key in their
//! configuration. Also each client's permission rules that let it always
//! use kkt ([`allow_kkt`]).

mod files;
mod formats;
mod host;
mod order;
mod permissions;

pub use host::Host;
pub use permissions::{KKT_AGENT_TOOLS, allow_kkt};

use kkt_config as config;
use kkt_settings::Env;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod fake;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    Codex,
    Agy,
    OpenCode,
    MuseCode,
}

/// Inputs chosen by the caller, including the absolute executable/key paths.
pub struct Setup<'a> {
    pub env: &'a Env,
    pub url: &'a str,
    pub executable: &'a Path,
    pub key_file: Option<&'a Path>,
    pub client_config: &'a Path,
    /// Where `client_config` came from, for errors: `--client-config`, or
    /// what [`Client::source`] names.
    pub source: &'a str,
}

impl Client {
    pub const ALL: [Client; 5] = [
        Client::ClaudeCode,
        Client::Codex,
        Client::OpenCode,
        Client::Agy,
        Client::MuseCode,
    ];

    /// The name `kkt connect` and `kkt setup --clients` take.
    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude-code",
            Client::Codex => "codex",
            Client::Agy => "agy",
            Client::OpenCode => "opencode",
            Client::MuseCode => "muse-code",
        }
    }

    pub fn from_name(name: &str) -> Option<Client> {
        Client::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The client's name for people.
    pub fn display_name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::Codex => "Codex",
            Client::Agy => "Antigravity",
            Client::OpenCode => "OpenCode",
            Client::MuseCode => "Muse Code",
        }
    }

    /// Whether this client has no way to read the key from its file, so
    /// `configure` copies the key into the config and warns about it.
    pub fn copies_key(self) -> bool {
        matches!(self, Client::Agy | Client::MuseCode)
    }

    /// The setting that chose [`Client::file`]: the client's own variable
    /// when it is set, otherwise HOME.
    pub fn source(self, env: &Env) -> &'static str {
        match self {
            Self::ClaudeCode if env.claude_config_dir.is_some() => "CLAUDE_CONFIG_DIR",
            Self::Codex if env.codex_home.is_some() => "CODEX_HOME",
            Self::OpenCode | Self::MuseCode
                if env
                    .xdg_config_home
                    .as_ref()
                    .is_some_and(|p| p.is_absolute()) =>
            {
                "XDG_CONFIG_HOME"
            }
            _ => "HOME",
        }
    }

    /// The file name the client requires, when it reads only that name.
    fn required_name(self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some(config::CLAUDE_CONFIG),
            Self::Codex => Some(config::CODEX_CONFIG),
            Self::MuseCode => Some(config::MUSE_CONFIG),
            Self::Agy | Self::OpenCode => None,
        }
    }

    /// Refuse a config file the client would never read because of its name.
    /// `source` is the setting that chose `path`.
    pub fn check_name(self, path: &Path, source: &str) -> Result<(), String> {
        match self.required_name() {
            Some(name) if path.file_name() != Some(std::ffi::OsStr::new(name)) => Err(format!(
                "{} config must be named {name}: {} (from {source})",
                self.display_name(),
                path.display()
            )),
            _ => Ok(()),
        }
    }
}

/// A shell word, for client helper commands (not for running a process here).
/// Fix text the operator reads uses `diagnostics::shell_word` instead.
pub(crate) fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The directory to pass as CLAUDE_CONFIG_DIR, or None to unset it.
/// Preserve a matching inherited profile; otherwise use Claude's normal
/// settings root for the default config and the parent for a custom config.
pub(crate) fn claude_config_dir(
    setup: &Setup<'_>,
    host: &dyn Host,
) -> Result<Option<PathBuf>, String> {
    let directory = |path: &Path| {
        let path = if path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            path
        };
        let absolute = host.absolute(path)?;
        // HOME and the current directory can name the same directory through
        // different symlinks. Missing profile directories still use absolute paths.
        match host.canonicalize(&absolute) {
            Ok(real) => Ok(real),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(absolute),
            Err(e) => Err(e),
        }
    };
    let unresolved = |problem: String| {
        format!(
            "could not resolve the directory of the Claude Code config {} (from {}): {problem}",
            setup.client_config.display(),
            setup.source
        )
    };
    let parent = setup
        .client_config
        .parent()
        .ok_or_else(|| unresolved("it has no parent directory".into()))?;
    let selected = directory(parent).map_err(|e| unresolved(e.to_string()))?;
    // Only the selected directory must resolve: an inherited profile or HOME
    // that cannot be resolved is not the selected one.
    let selects = |path: &PathBuf| directory(path).is_ok_and(|dir| dir == selected);
    if setup.env.claude_config_dir.as_ref().is_some_and(selects) {
        return Ok(Some(selected));
    }
    if setup.env.home.as_ref().is_some_and(selects) {
        return Ok(None);
    }
    Ok(Some(selected))
}

pub(crate) fn helper(setup: &Setup<'_>) -> Result<Option<String>, String> {
    let Some(key) = setup.key_file else {
        return Ok(None);
    };
    let executable = setup.executable.to_str().ok_or("kkt's path is not UTF-8")?;
    let key = key.to_str().ok_or("the key path is not UTF-8")?;
    Ok(Some(format!(
        "{} key header --json --key-file {}",
        shell_word(executable),
        shell_word(key)
    )))
}

/// The requested plain header, or the JSON header map both helper clients use.
pub fn header(key: &str, json: bool) -> Result<String, String> {
    config::check_access_key(key).map_err(|e| e.to_string())?;
    if json {
        formats::header_json(key)
    } else {
        Ok(format!("Authorization: Bearer {key}"))
    }
}

/// Whether Codex has kkt configured without an explicit MCP credential store.
/// This only reads the config; choosing a store belongs to the person using it.
pub fn codex_password_store_hint(env: &Env, host: &dyn Host) -> Result<bool, String> {
    let path = Client::Codex.file(env, host)?;
    let Some(text) = files::read(host, &path)? else {
        return Ok(false);
    };
    let document = formats::parse_toml(&text)
        .map_err(|e| format!("invalid Codex TOML in {}: {e}", path.display()))?;
    Ok(document
        .get("mcp_servers")
        .and_then(|servers| servers.get("kkt"))
        .is_some_and(|kkt| kkt.as_table_like().is_some())
        && !document.contains_key("mcp_oauth_credentials_store"))
}

/// Print a plan or apply one change. A plan never contains the key, even for
/// clients that copy it; --write reads it directly from the key file instead.
pub fn configure(
    client: Client,
    setup: &Setup<'_>,
    write: bool,
    host: &dyn Host,
) -> Result<String, String> {
    client.check_name(setup.client_config, setup.source)?;
    let claude_config_dir = match client {
        Client::ClaudeCode => claude_config_dir(setup, host)?,
        _ => None,
    };
    // File errors name the setting that chose the path.
    let from = |error: String| format!("{error} (from {})", setup.source);
    let original = files::read(host, setup.client_config).map_err(from)?;
    if !write {
        let entry = formats::preview(client, setup, claude_config_dir.as_deref())?;
        let mut out = format!("Config: {}\n{entry}\n", setup.client_config.display());
        if client.copies_key() && setup.key_file.is_some() {
            out.push_str(&format!(
                "{} stores the key in this file. --write reads the key file and sets permissions to 0600; the preview redacts it.\n",
                client.display_name()
            ));
        }
        out.push_str("Run again with --write to apply this configuration.\n");
        return Ok(out);
    }
    let updated = formats::merge(client, setup, original.as_deref(), host).map_err(|error| {
        format!(
            "{} (from {}): {error}",
            setup.client_config.display(),
            setup.source
        )
    })?;
    if updated.is_none() {
        if client.copies_key() {
            files::restrict(host, setup.client_config).map_err(from)?;
        }
        return Ok(format!(
            "Already configured: {}",
            setup.client_config.display()
        ));
    }
    let updated = updated.ok_or("missing merged client configuration")?;
    if client == Client::ClaudeCode {
        let absolute = host.absolute(setup.client_config).map_err(|e| {
            format!(
                "could not resolve the Claude Code config {} (from {}): {e}",
                setup.client_config.display(),
                setup.source
            )
        })?;
        files::apply_claude(
            host,
            &absolute,
            original.as_deref(),
            &updated,
            claude_config_dir.as_deref(),
            setup.source,
        )?;
    } else {
        files::replace(host, setup.client_config, original.as_deref(), &updated).map_err(from)?;
    }
    let mut out = format!("Configured: {}", setup.client_config.display());
    if client.copies_key() && setup.key_file.is_some() {
        out.push_str(&format!(
            "\n{} stores the key in this file; permissions are 0600.",
            client.display_name()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
