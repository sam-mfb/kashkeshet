//! Starting and stopping the installed service's containers.

use crate::ContainerError;
use kkt_container::{Output, Project, Projects};

/// Starts the containers, with the tool's output on the terminal.
pub(crate) fn up(projects: &dyn Projects, project: &Project) -> Result<(), ContainerError> {
    projects
        .up(project, &[], Output::Terminal)
        .map_err(|e| ContainerError::from_tool(projects, e))
}

/// Stops and removes the containers. The data stays, in its volumes,
/// unless `remove_data`.
pub(crate) fn down(
    projects: &dyn Projects,
    project: &Project,
    remove_data: bool,
) -> Result<(), ContainerError> {
    projects
        .down(project, remove_data, Output::Terminal)
        .map_err(|e| ContainerError::from_tool(projects, e))
}
