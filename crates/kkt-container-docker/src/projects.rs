//! kkt-container's general `Projects` over Docker Compose: each operation
//! as the `docker compose` command it runs in the project's folder, without
//! Compose's own variables or the project's shielded ones, through the
//! private runner. It knows nothing of what runs in the project.

use super::{Command, Docker, Finished};
use kkt_container::{
    FileAction, FileState, Output, Project, ProjectError, ProjectFile, Projects, Ran,
    ServiceConfig, User,
};
use std::path::Path;

/// The variables Compose reads itself, which would take the place of the
/// project folder's own settings if they were set where the tool runs.
const COMPOSE_VARIABLES: &[&str] = &[
    "COMPOSE_FILE",
    "COMPOSE_PATH_SEPARATOR",
    "COMPOSE_PROFILES",
    "COMPOSE_PROJECT_NAME",
    "COMPOSE_ENV_FILES",
];

/// A `docker` command without Compose's variables or `shielded`.
fn docker(shielded: &[&str], args: Vec<String>) -> Command {
    Command {
        program: "docker",
        args,
        unset: COMPOSE_VARIABLES
            .iter()
            .chain(shielded)
            .map(|v| v.to_string())
            .collect(),
        current_dir: None,
    }
}

/// `docker compose` for the project, with `args` after it, run in its
/// folder. The folder is given outright, so nothing depends on where the
/// tool is run from.
fn compose(project: &Project, args: &[&str]) -> Command {
    let folder = project.folder.path();
    let mut all = vec![
        "compose".to_string(),
        "--project-directory".to_string(),
        folder.display().to_string(),
    ];
    all.extend(args.iter().map(|a| a.to_string()));
    Command {
        current_dir: Some(folder.to_path_buf()),
        ..docker(project.shielded, all)
    }
}

fn ran(command: &Command, done: Finished) -> Ran {
    Ran {
        command: command.to_string(),
        status: done.code,
        stdout: done.stdout,
        stderr: done.stderr,
    }
}

impl Docker {
    /// Runs `command`, with its output kept or on the terminal; an error
    /// only if it could not run.
    fn ran(&self, command: &Command, out: Output) -> Result<Ran, ProjectError> {
        let done = match out {
            Output::Captured => self.host.run(command),
            Output::Terminal => self.host.run_attached(command),
        }
        .map_err(|e| ProjectError::Missing {
            command: command.to_string(),
            cause: e.to_string(),
        })?;
        Ok(ran(command, done))
    }

    /// Runs `command`; its output, or an error saying how it failed.
    fn succeed(&self, command: &Command, out: Output) -> Result<String, ProjectError> {
        self.ran(command, out)?.succeeded()
    }
}

impl Projects for Docker {
    fn available(&self) -> Result<(), ProjectError> {
        let command = docker(&[], vec!["compose".into(), "version".into()]);
        self.succeed(&command, Output::Captured).map(|_| ())
    }

    fn tool(&self) -> &str {
        "Docker"
    }

    fn typed(&self, args: &[&str]) -> String {
        let mut all = vec!["compose".to_string()];
        all.extend(args.iter().map(|a| a.to_string()));
        docker(&[], all).to_string()
    }

    fn write(&self, project: &Project, files: &[ProjectFile]) -> Result<(), ProjectError> {
        for file in files {
            let path = project.folder.path().join(file.path);
            self.host
                .write_file(&path, file.text, file.private)
                .map_err(|e| ProjectError::file(FileAction::Write, &path, &e))?;
        }
        Ok(())
    }

    fn make_folders(&self, project: &Project, folders: &[&Path]) -> Result<(), ProjectError> {
        for folder in folders {
            let path = project.folder.path().join(folder);
            self.host
                .create_dir_all(&path)
                .map_err(|e| ProjectError::file(FileAction::Make, &path, &e))?;
        }
        Ok(())
    }

    fn read(&self, project: &Project, file: &Path) -> Result<String, ProjectError> {
        let path = project.folder.path().join(file);
        self.host
            .read_file(&path)
            .map_err(|e| ProjectError::file(FileAction::Read, &path, &e))
    }

    fn file(&self, project: &Project, file: &Path) -> FileState {
        let path = project.folder.path().join(file);
        if self.host.is_file(&path) {
            FileState::File
        } else if self.host.exists(&path) {
            FileState::Other
        } else {
            FileState::Missing
        }
    }

    fn up(&self, project: &Project, services: &[&str], out: Output) -> Result<(), ProjectError> {
        let mut args = vec!["up", "-d"];
        args.extend(services);
        self.succeed(&compose(project, &args), out).map(|_| ())
    }

    fn down(
        &self,
        project: &Project,
        remove_volumes: bool,
        out: Output,
    ) -> Result<(), ProjectError> {
        let args: &[&str] = if remove_volumes {
            &["down", "--volumes"]
        } else {
            &["down"]
        };
        self.succeed(&compose(project, args), out).map(|_| ())
    }

    fn pull(&self, project: &Project, out: Output) -> Result<(), ProjectError> {
        self.succeed(&compose(project, &["pull"]), out).map(|_| ())
    }

    fn restart(&self, project: &Project, service: &str, out: Output) -> Result<(), ProjectError> {
        self.succeed(&compose(project, &["restart", service]), out)
            .map(|_| ())
    }

    fn running(&self, project: &Project) -> Result<Vec<String>, ProjectError> {
        let command = compose(project, &["ps", "--status", "running", "--services"]);
        let said = self.succeed(&command, Output::Captured)?;
        Ok(said
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn exec(
        &self,
        project: &Project,
        service: &str,
        user: User,
        args: &[&str],
        out: Output,
    ) -> Result<Ran, ProjectError> {
        let mut line = vec!["exec", "-T"];
        if user == User::Root {
            line.extend(["-u", "root"]);
        }
        line.push(service);
        line.extend(args);
        self.ran(&compose(project, &line), out)
    }

    fn run_once(
        &self,
        project: &Project,
        service: &str,
        user: User,
        entrypoint: &str,
        args: &[&str],
        out: Output,
    ) -> Result<Ran, ProjectError> {
        let mut line = vec!["run", "--rm", "--no-deps"];
        // Without a terminal to give it, the command's output is read
        // whole; with one, it is shown as it comes.
        if out == Output::Captured {
            line.push("-T");
        }
        if user == User::Root {
            line.extend(["--user", "root"]);
        }
        line.extend(["--entrypoint", entrypoint, service]);
        line.extend(args);
        self.ran(&compose(project, &line), out)
    }

    fn copy_in(
        &self,
        project: &Project,
        service: &str,
        host: &Path,
        inside: &str,
    ) -> Result<(), ProjectError> {
        let from = host.display().to_string();
        let to = format!("{service}:{inside}");
        self.succeed(&compose(project, &["cp", &from, &to]), Output::Captured)
            .map(|_| ())
    }

    fn copy_out(
        &self,
        project: &Project,
        service: &str,
        inside: &str,
        host: &Path,
    ) -> Result<(), ProjectError> {
        let from = format!("{service}:{inside}");
        let to = host.display().to_string();
        self.succeed(&compose(project, &["cp", &from, &to]), Output::Captured)
            .map(|_| ())
    }

    fn inspect(&self, project: &Project, service: &str) -> Result<ServiceConfig, ProjectError> {
        let ps = compose(project, &["ps", "--quiet", service]);
        let id = self.succeed(&ps, Output::Captured)?;
        let id = id.trim();
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ProjectError::Unexpected {
                command: ps.to_string(),
                detail: format!("no container ID for {service}"),
            });
        }
        let inspect = docker(
            project.shielded,
            vec![
                "inspect".into(),
                "--format".into(),
                "{{json .Config}}".into(),
                id.into(),
            ],
        );
        let text = self.succeed(&inspect, Output::Captured)?;
        let config: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| ProjectError::Unexpected {
                command: inspect.to_string(),
                detail: "not JSON".into(),
            })?;
        let strings = |value: &serde_json::Value| -> Option<Vec<String>> {
            value
                .as_array()?
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        Ok(ServiceConfig {
            command: strings(&config["Cmd"]),
            entrypoint: strings(&config["Entrypoint"]),
            env: strings(&config["Env"]),
            working_dir: config["WorkingDir"].as_str().unwrap_or("").to_string(),
        })
    }
}

#[cfg(test)]
mod tests;
