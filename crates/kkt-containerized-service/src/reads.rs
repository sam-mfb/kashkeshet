//! What the managed folder says, read through [`Projects`] without running
//! anything: whether its settings file is there, the version it selects,
//! the install it describes, and the Tailscale auth key a person names.

use crate::ContainerError;
use crate::Existing;
use crate::files::{ENV_FILE, PENDING_TAILNET, TAILSCALE_COMPOSE_FILE, embedded, version_in_env};
use kkt_container::{FileState, Folder, Project, Projects};
use std::path::{Path, PathBuf};

/// Whether anything is at the settings file's path, a folder included.
pub(crate) fn has_settings(projects: &dyn Projects, project: &Project) -> bool {
    projects.file(project, Path::new(ENV_FILE)) != FileState::Missing
}

/// Whether the settings file's path is a regular file.
pub(crate) fn settings_file(projects: &dyn Projects, project: &Project) -> bool {
    projects.file(project, Path::new(ENV_FILE)) == FileState::File
}

/// The version the settings file selects.
pub(crate) fn version(
    projects: &dyn Projects,
    project: &Project,
) -> Result<String, ContainerError> {
    let text = projects
        .read(project, Path::new(ENV_FILE))
        .map_err(ContainerError::from)?;
    version_in_env(&text).ok_or_else(|| ContainerError::Incomplete {
        reason: format!("{} has no KKT_VERSION", env_path(project).display()),
    })
}

/// The Tailscale auth key in the file at `path`, trimmed as setup's inputs
/// require. `path` is as the person gave it, so it is read where they are,
/// not in a project's folder.
pub(crate) fn sharing_key(projects: &dyn Projects, path: &Path) -> Result<String, ContainerError> {
    let here = Folder(PathBuf::new());
    let project = Project {
        folder: &here,
        shielded: &[],
    };
    projects
        .read(&project, path)
        .map(|key| key.trim().to_string())
        .map_err(ContainerError::from)
}

fn env_path(project: &Project) -> PathBuf {
    project.folder.path().join(ENV_FILE)
}

/// The install in the folder, if its settings file is there. A folder setup
/// did not finish writing, or whose Tailscale login never finished (so its
/// address is not known), is refused, saying what is wrong with it.
pub(crate) fn existing(
    projects: &dyn Projects,
    project: &Project,
) -> Result<Option<Existing>, ContainerError> {
    if !has_settings(projects, project) {
        return Ok(None);
    }
    let env_path = env_path(project);
    let env = projects
        .read(project, Path::new(ENV_FILE))
        .map_err(ContainerError::from)?;
    let value = |name: &str| {
        env.lines()
            .find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim().to_string())
    };
    let incomplete = |reason: String| ContainerError::Incomplete { reason };
    let missing = |what: &str| {
        incomplete(format!(
            "its settings file {} has no {what}",
            env_path.display()
        ))
    };
    let unknown = |line: String| {
        incomplete(format!(
            "its settings file {} has {line}, which kkt setup does not write",
            env_path.display()
        ))
    };
    // Setup writes no COMPOSE_FILE for a local install, and only this one
    // for Tailscale; anything else is some other setup, not kkt's.
    let tailscale = match value("COMPOSE_FILE").as_deref() {
        None => false,
        Some(TAILSCALE_COMPOSE_FILE) => true,
        Some(other) => return Err(unknown(format!("COMPOSE_FILE={other}"))),
    };
    for (name, _) in embedded(tailscale) {
        if projects.file(project, Path::new(name)) == FileState::Missing {
            return Err(incomplete(format!(
                "its Compose file {} is missing",
                project.folder.path().join(name).display()
            )));
        }
    }
    let port = value("KKT_PORT").ok_or_else(|| missing("KKT_PORT"))?;
    let port = match port.parse::<u16>() {
        Ok(port) if port != 0 => port,
        _ => {
            return Err(incomplete(format!(
                "its settings file {} has KKT_PORT={port}, which is not a port",
                env_path.display()
            )));
        }
    };
    let node = match tailscale {
        true => {
            let hostname = value("TS_HOSTNAME").ok_or_else(|| missing("TS_HOSTNAME"))?;
            let tailnet = value("TS_TAILNET").ok_or_else(|| missing("TS_TAILNET"))?;
            if tailnet == PENDING_TAILNET {
                return Err(incomplete(
                    "its Tailscale login never finished, so its address is not known".to_string(),
                ));
            }
            // The two make the address https://{hostname}.{tailnet}/mcp, so
            // each must be a DNS name: a label, and labels joined by dots.
            if !dns_name(&hostname) || hostname.contains('.') {
                return Err(unknown(format!("TS_HOSTNAME={hostname}")));
            }
            if !dns_name(&tailnet) {
                return Err(unknown(format!("TS_TAILNET={tailnet}")));
            }
            Some((hostname, tailnet))
        }
        false => None,
    };
    Ok(Some(Existing {
        sharing: tailscale,
        node,
        port,
    }))
}

/// Whether `name` is one or more DNS labels joined by dots: letters, digits
/// and inner hyphens, none empty.
fn dns_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}
