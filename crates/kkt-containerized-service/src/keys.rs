//! The server's keys as the running container shows them, beside their
//! copies on this machine.

use crate::ContainerError;
use crate::KeyCopy;
use crate::here;
use crate::in_server::{SERVER, host};
use kkt_container::{Output, Project, Projects, User};
use std::path::Path;

/// Compares the web key's host copy at `web_key_file` with the running
/// server's web key. Neither key is shown, nor put in an error.
pub(crate) fn compare_web_key(
    projects: &dyn Projects,
    project: &Project,
    web_key_file: &Path,
) -> Result<KeyCopy, ContainerError> {
    let here_folder = here();
    let copy = projects
        .read(&host(&here_folder), web_key_file)
        .map_err(ContainerError::from)?;
    let ran = projects
        .exec(
            project,
            SERVER,
            User::Default,
            &["kkt", "key", "show", "--web"],
            Output::Captured,
        )
        .map_err(|e| ContainerError::from_tool(projects, e))?;
    let server = ran.stdout.trim();
    if !ran.success() || server.is_empty() {
        // Its output is left out: it may hold the key.
        let detail = match ran.status {
            Some(code) => format!("exit {code}, no key"),
            None => "ended by a signal".to_string(),
        };
        return Err(ContainerError::Command {
            command: ran.command,
            detail,
        });
    }
    Ok(if copy.trim() == server {
        KeyCopy::Same
    } else {
        KeyCopy::Differs
    })
}
