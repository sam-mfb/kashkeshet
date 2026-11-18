//! What can stop a backup, each kind worded for the person.

use std::path::PathBuf;

/// Why [`backup_of`](crate::backup_of) has no backup to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackupOfError {
    /// `--db` or `--config` was given for kkt in Docker.
    #[error(
        "Docker backup uses the configured container; --db and --config apply only to a local service"
    )]
    DockerTakesNoLocalOptions,
    /// This machine is a client of a server elsewhere.
    #[error("this machine is a client; back up the server on the machine where it runs")]
    OnAClient,
    /// A local backup was asked for with no folder to write it into.
    #[error("a local backup needs FOLDER; run `kkt backup <folder>`")]
    NoFolder,
}

/// Why the folder backups go in is not known.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    /// The database path is in no folder, such as `/`.
    #[error("{} is not a database file in a folder", database.display())]
    NotInFolder { database: PathBuf },
    /// There is no home, so no config folder.
    #[error("set HOME to find the backup folder")]
    NoHome,
}

/// A key file a backup needs could not be inspected.
#[derive(Debug, thiserror::Error)]
#[error("could not inspect {}: {cause}", path.display())]
pub struct InspectError {
    pub path: PathBuf,
    #[source]
    pub cause: std::io::Error,
}

/// What a backup did to a file or folder when it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    Create,
    Rename,
    Copy,
    Read,
    Write,
    List,
    Remove,
}

impl FileAction {
    fn verb(self) -> &'static str {
        match self {
            FileAction::Create => "create",
            FileAction::Rename => "rename",
            FileAction::Copy => "copy",
            FileAction::Read => "read",
            FileAction::Write => "write",
            FileAction::List => "list",
            FileAction::Remove => "remove",
        }
    }
}

/// Why [`Backups`](crate::Backups) could not write or prune.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// A backup of that name is already there.
    #[error("{} already exists", path.display())]
    Exists { path: PathBuf },
    /// A step on a file or folder failed.
    #[error("could not {} {}: {cause}", action.verb(), path.display())]
    File {
        action: FileAction,
        path: PathBuf,
        #[source]
        cause: std::io::Error,
    },
    /// The database at `path` could not be copied, for the reason the
    /// database copy the implementation was given.
    #[error("could not copy the database {}: {cause}", path.display())]
    Database {
        path: PathBuf,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The backup failed, and its unfinished folder could not be removed.
    #[error("{error}; could not remove the unfinished {}: {cause}", partial.display())]
    NotRemoved {
        error: Box<BackupError>,
        partial: PathBuf,
        #[source]
        cause: std::io::Error,
    },
}

impl BackupError {
    /// `action` on `path` failed with `cause`.
    pub fn file(action: FileAction, path: impl Into<PathBuf>, cause: std::io::Error) -> Self {
        BackupError::File {
            action,
            path: path.into(),
            cause,
        }
    }
}
