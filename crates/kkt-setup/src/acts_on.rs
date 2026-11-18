//! Which server `kkt backup` acts on, by how this machine is set up: kkt in
//! Docker in its managed folder, or the local database `kkt serve` would
//! use. A client machine has no server to act on.

use super::record::Mode;
use std::path::{Path, PathBuf};

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

/// What `kkt backup` backs up, given the recorded `mode` (`None` with no
/// record), the folder given (`into`) and whether `--db` or `--config` was
/// given (`local_options`).
pub fn backup_of(
    mode: Option<&Mode>,
    into: Option<&Path>,
    local_options: bool,
) -> Result<BackupOf, String> {
    match mode {
        Some(Mode::Docker(install)) => {
            if local_options {
                return Err(
                    "Docker backup uses the configured container; --db and --config \
                            apply only to a local service"
                        .into(),
                );
            }
            Ok(BackupOf::Docker {
                folder: install.folder.clone(),
                into: into.map(Path::to_path_buf),
            })
        }
        Some(Mode::Client) => {
            Err("this machine is a client; back up the server on the machine where it runs".into())
        }
        Some(Mode::Service { .. }) | None => {
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
    use crate::DockerSetup;

    fn docker() -> Mode {
        Mode::Docker(DockerSetup {
            folder: PathBuf::from("/data/docker"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        })
    }

    const SERVICE: Mode = Mode::Service { tailscale: false };

    #[test]
    fn a_docker_backup_uses_its_folder_and_any_folder_given() {
        assert_eq!(
            backup_of(Some(&docker()), None, false),
            Ok(BackupOf::Docker {
                folder: PathBuf::from("/data/docker"),
                into: None,
            })
        );
        assert_eq!(
            backup_of(Some(&docker()), Some(Path::new("out")), false),
            Ok(BackupOf::Docker {
                folder: PathBuf::from("/data/docker"),
                into: Some(PathBuf::from("out")),
            })
        );
        assert_eq!(
            backup_of(Some(&docker()), Some(Path::new("out")), true),
            Err("Docker backup uses the configured container; --db and --config apply only to a local service".into())
        );
    }

    #[test]
    fn a_local_backup_needs_a_folder() {
        for mode in [Some(&SERVICE), None] {
            assert_eq!(
                backup_of(mode, Some(Path::new("out")), true),
                Ok(BackupOf::Local {
                    into: PathBuf::from("out")
                })
            );
            assert_eq!(
                backup_of(mode, None, false),
                Err("a local backup needs FOLDER; run `kkt backup <folder>`".into())
            );
        }
    }

    #[test]
    fn a_backup_is_refused_on_a_client() {
        assert_eq!(
            backup_of(Some(&Mode::Client), Some(Path::new("out")), false),
            Err("this machine is a client; back up the server on the machine where it runs".into())
        );
    }
}
