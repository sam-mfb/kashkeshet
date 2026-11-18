//! Why a client's config could not be read, written or changed: the kinds
//! a caller can act on, each worded here.

use crate::Client;
use std::io;
use std::path::PathBuf;

/// Why configuring a client failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientConfigError {
    /// No home folder to find the client's config in.
    #[error("set HOME to find the client's config")]
    NoHome,
    /// A step on a file or folder of the client's config failed.
    #[error("could not {} {}: {cause}", action.verb(), path.display())]
    File {
        action: FileAction,
        path: PathBuf,
        #[source]
        cause: io::Error,
    },
    /// Writing the config failed, and so did removing the temporary file.
    #[error(
        "could not write {}: {error}; could not remove temporary file: {cleanup}",
        path.display()
    )]
    TemporaryLeft {
        path: PathBuf,
        error: io::Error,
        cleanup: io::Error,
    },
    /// Every backup name beside the config is taken.
    #[error("no unused backup name")]
    NoBackupName,
    /// A config path with no folder above it.
    #[error("{0}")]
    NoFolder(Folder),
    /// The client's config is not a shape kkt can edit.
    #[error("{0}")]
    Malformed(Shape),
    /// A path that is not UTF-8, where text is needed.
    #[error("{0}")]
    NotText(Text),
    /// A value that could not be written out as JSON.
    #[error("{0}")]
    Encode(Encoded),
    /// A config file the client would never read because of its name.
    #[error(
        "{} config must be named {name}: {} (from {setting})",
        client.display_name(),
        path.display()
    )]
    WrongFile {
        client: Client,
        name: &'static str,
        path: PathBuf,
        setting: String,
    },
    /// The access key the config would hold is not a valid key.
    #[error("{0}")]
    Key(#[from] kkt_keys::SettingError),
    /// The client's own command failed. The text is the `ClientCommand`
    /// contract's; typing that contract is a later step.
    #[error("{0}")]
    Command(String),
    /// The client has no setting that allows kkt's tools.
    #[error(
        "{} has no setting that allows one MCP server's tools or one command, so it still asks before each",
        client.display_name()
    )]
    NoAllowSetting { client: Client },
    /// Codex's config has no kkt server to allow.
    #[error("Codex has no kkt server here; connect it first")]
    NotConnected,
    /// Claude Code's config folder is not UTF-8.
    #[error(
        "the Claude Code config directory {} is not UTF-8 (from {setting})",
        dir.display()
    )]
    ClaudeDirNotText { dir: PathBuf, setting: String },
    /// The folder of Claude Code's config could not be resolved.
    #[error(
        "could not resolve the directory of the Claude Code config {} (from {setting}): {problem}",
        path.display()
    )]
    DirectoryUnresolved {
        path: PathBuf,
        setting: String,
        problem: Unresolved,
    },
    /// Claude Code's config could not be made absolute.
    #[error(
        "could not resolve the Claude Code config {} (from {setting}): {cause}",
        path.display()
    )]
    ConfigUnresolved {
        path: PathBuf,
        setting: String,
        cause: io::Error,
    },
    /// A change failed, and putting the config back failed too.
    #[error("{error}; could not restore: {restore}")]
    NotRestored {
        error: Box<ClientConfigError>,
        restore: Box<ClientConfigError>,
    },
    /// A change failed, and removing the config it made failed too.
    #[error("{error}; could not restore {}: {cause}", path.display())]
    NotRemoved {
        error: Box<ClientConfigError>,
        path: PathBuf,
        cause: io::Error,
    },
    /// The merge said a change was needed and gave none.
    #[error("missing merged client configuration")]
    NotMerged,
    /// `error`, from the config the setting `setting` chose.
    #[error("{error} (from {setting})")]
    From {
        setting: String,
        error: Box<ClientConfigError>,
    },
    /// `error`, in the config at `path`, which `setting` chose.
    #[error("{} (from {setting}): {error}", path.display())]
    InConfig {
        path: PathBuf,
        setting: String,
        error: Box<ClientConfigError>,
    },
    /// `error`, in the file at `path`, which `setting` chose.
    #[error("{}: {error} (from {setting})", path.display())]
    InFile {
        path: PathBuf,
        setting: String,
        error: Box<ClientConfigError>,
    },
    /// `error`, in `client`'s config at `path`, which `setting` chose.
    #[error("{}: {} (from {setting}): {error}", client.display_name(), path.display())]
    OfClient {
        client: Client,
        path: PathBuf,
        setting: String,
        error: Box<ClientConfigError>,
    },
}

impl ClientConfigError {
    /// A file step on `path` that failed with `cause`.
    pub(crate) fn file(action: FileAction, path: &std::path::Path, cause: io::Error) -> Self {
        ClientConfigError::File {
            action,
            path: path.to_path_buf(),
            cause,
        }
    }

    /// This error, from the config `setting` chose.
    pub(crate) fn with_setting(self, setting: &str) -> Self {
        ClientConfigError::From {
            setting: setting.to_string(),
            error: Box::new(self),
        }
    }
}

/// The step on a file or folder that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    Read,
    BackUp,
    Restrict,
    Create,
    Prepare,
    Write,
}

impl FileAction {
    fn verb(self) -> &'static str {
        match self {
            FileAction::Read => "read",
            FileAction::BackUp => "back up",
            FileAction::Restrict => "restrict",
            FileAction::Create => "create",
            FileAction::Prepare => "prepare",
            FileAction::Write => "write",
        }
    }
}

/// A config path with no folder above it.
#[derive(Debug, thiserror::Error)]
pub enum Folder {
    #[error("config path has no parent")]
    ConfigPath,
    #[error("the Codex config has no folder")]
    CodexConfig,
}

/// The shapes of a client's config kkt refuses to edit.
#[derive(Debug, thiserror::Error)]
pub enum Shape {
    #[error("client configuration must contain objects, not scalars or lists")]
    NotObjects,
    #[error("invalid client JSON at line {line}, column {column}; the file was not changed")]
    Json { line: usize, column: usize },
    #[error("invalid Codex TOML; the file was not changed: {0}")]
    TomlUnchanged(TomlProblem),
    #[error("invalid Codex TOML: {0}")]
    Toml(TomlProblem),
    #[error("invalid Codex TOML in {}: {problem}", path.display())]
    TomlIn { path: PathBuf, problem: TomlProblem },
    #[error("mcp_servers must be a TOML table")]
    McpServersNotTable,
    #[error("mcp_servers.kkt must be a TOML table")]
    KktNotTable,
    #[error("Codex HTTP headers must be a table")]
    HeadersNotTable,
    #[error("missing kkt entry")]
    NoKktEntry,
    #[error("Codex uses TOML")]
    CodexIsToml,
    #[error("permissions.allow must be a list")]
    AllowNotList,
    #[error("permissions must be a list of rules")]
    PermissionsNotList,
    #[error("permission.bash must be a word or an object")]
    BashNotWordOrObject,
    #[error(
        "OpenCode file references cannot represent braces, quotes, backslashes or control characters in a key path"
    )]
    KeyPathCharacters,
    #[error("could not read the order of the file's keys: {0}")]
    Order(#[source] serde_json::Error),
}

/// Why Codex's TOML could not be read: the parser's reason, its lines
/// joined, and where it found the problem. It never quotes the file, which
/// may hold a key.
#[derive(Debug)]
pub struct TomlProblem(pub kkt_toml::ParseError);

impl std::fmt::Display for TomlProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = self
            .0
            .reason()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        // Without a place in the text, the reason stands alone.
        match self.0.place() {
            Some(place) => write!(
                f,
                "{reason} at line {}, column {}",
                place.line, place.column
            ),
            None => f.write_str(&reason),
        }
    }
}

/// A path that is not UTF-8.
#[derive(Debug, thiserror::Error)]
pub enum Text {
    #[error("kkt's path is not UTF-8")]
    KktPath,
    #[error("the key path is not UTF-8")]
    KeyPath,
    #[error("key path is not UTF-8")]
    KeyFilePath,
}

/// A value that could not be written out.
#[derive(Debug, thiserror::Error)]
pub enum Encoded {
    #[error("could not encode the header")]
    Header,
    #[error("could not encode config")]
    Config,
    #[error("could not encode Claude config")]
    ClaudeConfig,
    #[error("could not encode the config")]
    Ordered,
}

/// Why the folder of Claude Code's config could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum Unresolved {
    #[error("it has no parent directory")]
    NoParent,
    #[error("{0}")]
    Io(io::Error),
}
