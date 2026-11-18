//! Which server `kkt backup` acts on, by where this machine's server is:
//! kkt in Docker in its managed folder, or the local database `kkt serve`
//! would use. A client machine has no server to act on.

use std::path::{Path, PathBuf};

/// Where this machine's server is, as the caller reads it from how the
/// machine is set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerHere {
    /// kkt runs on this machine, or nothing is set up: the local database
    /// `kkt serve` would use.
    ThisMachine,
    /// kkt runs in Docker, from its managed `folder`.
    InDocker { folder: PathBuf },
    /// This machine is a client of a server elsewhere.
    Elsewhere,
}

/// What `kkt backup` backs up, and where to.
#[derive(Debug, PartialEq, Eq)]
pub enum BackupOf {
    /// kkt in Docker run from `folder`, into `into` if given, else into the
    /// managed folder's own backups.
    Docker {
        folder: PathBuf,
        into: Option<PathBuf>,
    },
    /// The local database `kkt serve` would use, with its config file and
    /// keys, into `into`.
    Local { into: PathBuf },
}

/// What `kkt backup` backs up, given where this machine's server is
/// (`server`), the folder given (`into`) and whether `--db` or `--config`
/// was given (`local_options`).
pub fn backup_of(
    server: &ServerHere,
    into: Option<&Path>,
    local_options: bool,
) -> Result<BackupOf, String> {
    match server {
        ServerHere::InDocker { folder } => {
            if local_options {
                return Err(
                    "Docker backup uses the configured container; --db and --config \
                            apply only to a local service"
                        .into(),
                );
            }
            Ok(BackupOf::Docker {
                folder: folder.clone(),
                into: into.map(Path::to_path_buf),
            })
        }
        ServerHere::Elsewhere => {
            Err("this machine is a client; back up the server on the machine where it runs".into())
        }
        ServerHere::ThisMachine => {
            let into = into.ok_or("a local backup needs FOLDER; run `kkt backup <folder>`")?;
            Ok(BackupOf::Local {
                into: into.to_path_buf(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docker() -> ServerHere {
        ServerHere::InDocker {
            folder: PathBuf::from("/data/docker"),
        }
    }

    #[test]
    fn a_docker_backup_uses_its_folder_and_any_folder_given() {
        assert_eq!(
            backup_of(&docker(), None, false),
            Ok(BackupOf::Docker {
                folder: PathBuf::from("/data/docker"),
                into: None,
            })
        );
        assert_eq!(
            backup_of(&docker(), Some(Path::new("out")), false),
            Ok(BackupOf::Docker {
                folder: PathBuf::from("/data/docker"),
                into: Some(PathBuf::from("out")),
            })
        );
        assert_eq!(
            backup_of(&docker(), Some(Path::new("out")), true),
            Err("Docker backup uses the configured container; --db and --config apply only to a local service".into())
        );
    }

    #[test]
    fn a_local_backup_needs_a_folder() {
        let here = ServerHere::ThisMachine;
        assert_eq!(
            backup_of(&here, Some(Path::new("out")), true),
            Ok(BackupOf::Local {
                into: PathBuf::from("out")
            })
        );
        assert_eq!(
            backup_of(&here, None, false),
            Err("a local backup needs FOLDER; run `kkt backup <folder>`".into())
        );
    }

    #[test]
    fn a_backup_is_refused_on_a_client() {
        assert_eq!(
            backup_of(&ServerHere::Elsewhere, Some(Path::new("out")), false),
            Err("this machine is a client; back up the server on the machine where it runs".into())
        );
    }
}
