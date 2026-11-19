//! Backing the installed server up: `kkt backup` inside its container, into
//! the managed backups folder, and a copy of the new backup wherever the
//! person asks.

use crate::here;
use crate::in_server::{SERVER, host};
use crate::{ContainerError, Tell};
use kkt_container::{Folder, Output, Project, Projects, User};
use std::path::{Path, PathBuf};

/// The managed backups folder, in the managed folder.
const BACKUPS: &str = "backups";
/// Where the Compose file mounts the backups folder in the container.
const BACKUPS_MOUNT: &str = "/backups";

/// The backups folder on the host, in the managed `folder`.
pub(crate) fn backups(folder: &Folder) -> PathBuf {
    folder.path().join(BACKUPS)
}

/// Backs kkt up with `kkt backup` inside the container, then copies the new
/// backup folder into `dest`, if given, owned by the person running this.
/// Without a destination, returns the path in the managed backups folder.
/// The managed copy stays in either case.
pub(crate) fn backup(
    projects: &dyn Projects,
    project: &Project,
    tell: &dyn Tell,
    working_dir: Result<&Path, &crate::NoWorkingDir>,
    dest: Option<&Path>,
) -> Result<PathBuf, ContainerError> {
    let name = backup_in_container(projects, project, tell)?;
    let Some(dest) = dest else {
        return Ok(backups(project.folder).join(name));
    };
    // A folder typed relative to where the person is, not to the managed
    // folder the tool's commands run in.
    let dest = working_dir
        .map_err(|e| ContainerError::NoCurrentFolder(e.clone()))?
        .join(dest);
    let here_folder = here();
    projects
        .make_folders(&host(&here_folder), &[&dest])
        .map_err(ContainerError::from)?;
    projects
        .copy_out(project, SERVER, &format!("{BACKUPS_MOUNT}/{name}"), &dest)
        .map_err(|e| ContainerError::from_tool(projects, e))?;
    Ok(dest.join(name))
}

/// The line `kkt backup` prints when the highest message ID was not
/// recorded in the backup's about file.
const NOT_RECORDED: &str = "Highest message ID not recorded in about.txt:";

/// Runs `kkt backup` in the container and returns the new folder's name.
/// The line saying the highest message ID was not recorded, when the
/// in-container backup printed one, is told to the person here, as the
/// in-container command had no terminal of the person's to print it on.
pub(crate) fn backup_in_container(
    projects: &dyn Projects,
    project: &Project,
    tell: &dyn Tell,
) -> Result<String, ContainerError> {
    let said = projects
        .exec(
            project,
            SERVER,
            User::Default,
            &["kkt", "backup", BACKUPS_MOUNT],
            Output::Captured,
        )
        .and_then(|ran| ran.succeeded())
        .map_err(|e| ContainerError::from_tool(projects, e))?;
    if let Some(line) = said.lines().find(|l| l.starts_with(NOT_RECORDED)) {
        tell.say(line);
    }
    let line = said
        .lines()
        .find_map(|l| l.strip_prefix("Backed up to "))
        .ok_or_else(|| ContainerError::unexpected("`kkt backup`", said.trim()))?;
    Path::new(line.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| ContainerError::unexpected("`kkt backup`", line))
}
