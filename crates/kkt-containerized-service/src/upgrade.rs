//! Upgrading the installed server to another version: a backup first, this
//! program's Compose files, the new version in the settings file, the new
//! image and a start; the settings put back if that fails; then the keys'
//! copies saved again.

use crate::KeyCopies;
use crate::backup::backup_in_container;
use crate::files::{ENV_FILE, is_shared, set_version};
use crate::in_server::SERVER;
use crate::services::worded;
use crate::setup::{Parts, save_keys, say_saved, write_embedded, write_env};
use kkt_container::{Output, Project};
use std::path::Path;

/// Backs the server up, writes this program's Compose files and selects
/// `new_version` in the settings file, keeping its other lines, then pulls
/// and starts it. If the pull or the start fails, the settings file is put
/// back as it was, so it does not claim a version that did not start and a
/// later try is not refused. Then, with `keys`, saves the keys' copies
/// again; if that fails the upgrade stands, and the person is told how to
/// save them.
pub(crate) async fn upgrade(
    parts: &Parts<'_>,
    project: &Project<'_>,
    new_version: &str,
    keys: Option<&KeyCopies>,
) -> Result<(), String> {
    let projects = parts.projects;
    let tell = parts.tell;
    let env = projects
        .read(project, Path::new(ENV_FILE))
        .map_err(|e| e.to_string())?;
    backup_in_container(projects, project)?;
    write_embedded(projects, project, is_shared(&env))?;
    write_env(projects, project, &set_version(&env, new_version))?;
    let result = projects
        .pull(project, Output::Terminal)
        .and_then(|()| projects.up(project, &[], Output::Terminal))
        .map_err(|e| worded(projects, e));
    if let Err(error) = result {
        // Do not leave the selected version claiming that a failed upgrade
        // succeeded: the caller's version check must allow a later retry.
        write_env(projects, project, &env).map_err(|restore| {
            format!(
                "unexpected output from upgrade recovery: {error}; could not restore the previous settings: {restore}"
            )
        })?;
        return Err(error);
    }
    let Some(keys) = keys else {
        return Ok(());
    };
    match save_keys(parts, project, keys).await {
        Ok(saved) => say_saved(tell, &saved),
        Err(e) => tell.say(&format!(
            "kkt is running, but its keys could not be saved on this machine: {e}. \
             To save them, run `kkt setup` again with the same answers. \
             To read the web key now, run `{}` in {}.",
            projects.typed(&["exec", SERVER, "kkt", "key", "show", "--web"]),
            project.folder.path().display()
        )),
    }
    Ok(())
}
