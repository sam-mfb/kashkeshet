//! Starting and stopping the installed service's containers.

use kkt_container::{Output, Project, ProjectError, Projects};

/// Starts the containers, with the tool's output on the terminal.
pub(crate) fn up(projects: &dyn Projects, project: &Project) -> Result<(), String> {
    projects
        .up(project, &[], Output::Terminal)
        .map_err(|e| worded(projects, e))
}

/// Stops and removes the containers. The data stays, in its volumes,
/// unless `remove_data`.
pub(crate) fn down(
    projects: &dyn Projects,
    project: &Project,
    remove_data: bool,
) -> Result<(), String> {
    projects
        .down(project, remove_data, Output::Terminal)
        .map_err(|e| worded(projects, e))
}

/// A failure as the person is told it: a tool that could not be run at
/// all is said to be not available, by its name.
pub(crate) fn worded(projects: &dyn Projects, error: ProjectError) -> String {
    match error {
        ProjectError::Missing { .. } => format!("{} is not available: {error}", projects.tool()),
        other => other.to_string(),
    }
}
