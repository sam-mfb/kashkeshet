//! `kkt lanes import` and `export` for kkt in Docker: the database is in the
//! container's volume, so the lane files are copied in or out with `docker
//! compose cp` and `kkt lanes` runs in the container. Each run makes its own
//! folder in the container with `mktemp -d` and removes only that folder
//! afterwards, so two runs at the same time never see or remove each other's
//! files.

use super::compose::compose;
use super::provision::run;
use super::{Command, DockerError, Folder, Host};
use crate::config::{DOCKER_LANES_EXPORT_DIR, DOCKER_LANES_IMPORT_DIR};
use std::path::{Path, PathBuf};

/// What to import: the lane files, and the source to record for them, which
/// is the folder on the host and its git commit, not the copy in the
/// container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanesImport {
    /// Absolute paths of the `.md` files to import.
    pub files: Vec<PathBuf>,
    pub source_folder: String,
    pub source_commit: Option<String>,
    pub retire_missing: bool,
}

/// Imports the lane files into kkt in Docker. Returns what `kkt lanes
/// import` printed in the container.
pub fn import_lanes(
    host: &dyn Host,
    folder: &Folder,
    import: &LanesImport,
) -> Result<String, DockerError> {
    let dir = make_dir(host, folder, DOCKER_LANES_IMPORT_DIR)?;
    let done = copy_in_and_import(host, folder, &dir, import);
    finish(host, folder, &dir, done)
}

fn copy_in_and_import(
    host: &dyn Host,
    folder: &Folder,
    dir: &str,
    import: &LanesImport,
) -> Result<String, DockerError> {
    for file in &import.files {
        let name =
            file.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| DockerError::Unexpected {
                    what: "a lane file's name",
                    detail: file.display().to_string(),
                })?;
        let from = file.display().to_string();
        let to = format!("kkt:{dir}/{name}");
        succeed(host, &compose(folder, &["cp", &from, &to]))?;
    }
    // Copied files keep their host modes, and belong to root in the
    // container; the kkt user must be able to read them.
    succeed(
        host,
        &compose(
            folder,
            &[
                "exec", "-T", "-u", "root", "kkt", "chmod", "-R", "a+rX", dir,
            ],
        ),
    )?;
    let mut args = vec![
        "exec",
        "-T",
        "kkt",
        "kkt",
        "lanes",
        "import",
        dir,
        "--source-folder",
        &import.source_folder,
    ];
    if let Some(commit) = &import.source_commit {
        args.extend(["--source-commit", commit]);
    }
    if import.retire_missing {
        args.push("--retire-missing");
    }
    succeed(host, &compose(folder, &args))
}

/// Exports every current lane from kkt in Docker to `dest`, an absolute
/// folder on the host, created if missing. Returns what `kkt lanes export`
/// printed in the container.
pub fn export_lanes(host: &dyn Host, folder: &Folder, dest: &Path) -> Result<String, DockerError> {
    let dir = make_dir(host, folder, DOCKER_LANES_EXPORT_DIR)?;
    let done = export_and_copy_out(host, folder, &dir, dest);
    finish(host, folder, &dir, done)
}

fn export_and_copy_out(
    host: &dyn Host,
    folder: &Folder,
    dir: &str,
    dest: &Path,
) -> Result<String, DockerError> {
    let said = succeed(
        host,
        &compose(
            folder,
            &["exec", "-T", "kkt", "kkt", "lanes", "export", dir],
        ),
    )?;
    host.create_dir_all(dest)
        .map_err(|e| DockerError::io(format!("could not create {}", dest.display()), e))?;
    let from = format!("kkt:{dir}/.");
    let to = dest.display().to_string();
    succeed(host, &compose(folder, &["cp", &from, &to]))?;
    Ok(said)
}

/// Makes a new, empty folder in the kkt container whose name starts with
/// `prefix` and a dot, as the kkt user, and returns its path. `mktemp`
/// picks a name no other folder has, so this run owns it.
fn make_dir(host: &dyn Host, folder: &Folder, prefix: &str) -> Result<String, DockerError> {
    let template = format!("{prefix}.XXXXXXXXXX");
    let said = succeed(
        host,
        &compose(folder, &["exec", "-T", "kkt", "mktemp", "-d", &template]),
    )?;
    let dir = said.trim();
    let start = format!("{prefix}.");
    let named_well = dir.len() > start.len()
        && dir.starts_with(&start)
        && dir[start.len()..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric());
    if !named_well {
        return Err(DockerError::Unexpected {
            what: "the folder mktemp made in the container",
            detail: said,
        });
    }
    Ok(dir.to_string())
}

/// Removes this run's folder in the container, then returns `done`. A
/// failure to remove it after `done` worked is an error; after `done`
/// failed, it is said, and `done`'s error returned.
fn finish(
    host: &dyn Host,
    folder: &Folder,
    dir: &str,
    done: Result<String, DockerError>,
) -> Result<String, DockerError> {
    match (done, remove(host, folder, dir)) {
        (Ok(said), Ok(())) => Ok(said),
        (Ok(_), Err(e)) => Err(e),
        (Err(e), Ok(())) => Err(e),
        (Err(e), Err(cleanup)) => {
            host.say(&format!(
                "Could not remove {dir} in the container: {cleanup}"
            ));
            Err(e)
        }
    }
}

/// Removes `dir` in the kkt container, if it is there.
fn remove(host: &dyn Host, folder: &Folder, dir: &str) -> Result<(), DockerError> {
    succeed(
        host,
        &compose(
            folder,
            &["exec", "-T", "-u", "root", "kkt", "rm", "-rf", dir],
        ),
    )
    .map(|_| ())
}

/// Runs `command`; its output, or an error saying how it failed.
fn succeed(host: &dyn Host, command: &Command) -> Result<String, DockerError> {
    let done = run(host, command)?;
    if !done.success() {
        return Err(DockerError::Command {
            command: command.to_string(),
            detail: done.detail(),
        });
    }
    Ok(done.stdout)
}
