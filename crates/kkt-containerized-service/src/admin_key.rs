//! Giving the installed server an admin key for `/mcp-admin`, and saving
//! its copy on this machine.

use crate::in_server::SERVER;
use crate::services::worded;
use crate::setup::{Parts, ServerKey, read_admin_key, save_keys, say_saved, wait_for_key};
use kkt_container::{KeyCopies, Output, Project, User};
use std::path::PathBuf;

/// Gives the running server an admin key named `name`, unless it has one,
/// then restarts it so it serves `/mcp-admin`; then saves the keys' copies
/// where `keys` says, as setup does, and returns the admin key's. No key is
/// shown or put in a command's arguments or an error; the name, which is
/// not secret, is.
///
/// The server is restarted even when the key was already there: a run
/// that made the key and then failed to restart leaves the server without
/// `/mcp-admin`, and running this again must finish the job without making
/// another key.
pub(crate) async fn add_admin_key(
    parts: &Parts<'_>,
    project: &Project<'_>,
    keys: &KeyCopies,
    name: &str,
) -> Result<PathBuf, String> {
    let projects = parts.projects;
    let tell = parts.tell;
    // Wait for the server to be up before asking it about its admin key.
    wait_for_key(parts, project, ServerKey::Access).await?;
    if read_admin_key(projects, project)?.is_none() {
        tell.say(&format!(
            "Making the admin key for /mcp-admin, for the admin agent {name}..."
        ));
        projects
            .exec(
                project,
                SERVER,
                User::Default,
                &["kkt", "key", "create", "--admin", "--name", name],
                Output::Terminal,
            )
            .and_then(|ran| ran.succeeded())
            .map_err(|e| worded(projects, e))?;
    }
    tell.say("Restarting kkt so it serves /mcp-admin...");
    projects
        .restart(project, SERVER, Output::Terminal)
        .map_err(|e| worded(projects, e))?;
    let saved = save_keys(parts, project, keys).await?;
    say_saved(tell, &saved);
    saved.admin.ok_or_else(|| {
        "unexpected output from `kkt key show --admin`: kkt has no admin key after making one"
            .to_string()
    })
}
