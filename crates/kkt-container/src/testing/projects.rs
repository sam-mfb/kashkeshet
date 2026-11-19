//! Compose projects in memory: [`MemoryProjects`] implements [`Projects`]
//! with files, folders and running services kept in memory, and commands in
//! a service answered by the test.

use crate::{
    FileAction, FileState, Output, Project, ProjectError, ProjectFile, Projects, Ran,
    ServiceConfig, User,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

/// A command in a service that [`MemoryProjects`] was asked to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// In the running service.
    Exec {
        service: String,
        user: User,
        args: Vec<String>,
    },
    /// In a new container of the service.
    RunOnce {
        service: String,
        user: User,
        entrypoint: String,
        args: Vec<String>,
    },
}

/// How a test answers a command in a service: its exit status and output,
/// or that it could not run.
type Answer = Box<dyn Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError>>;

/// Compose projects in memory. Paths are taken relative to the project's
/// folder unless absolute. `up` with no services starts every service it
/// was given; `restart` fails for a service that is not running. Each
/// operation is noted, as a person would type it, in [`Self::log`]. The
/// tool is called `tool`.
pub struct MemoryProjects {
    services: Vec<String>,
    volumes: BTreeSet<String>,
    answer: Answer,
    available: bool,
    files: RefCell<BTreeMap<PathBuf, (String, bool)>>,
    folders: RefCell<BTreeSet<PathBuf>>,
    running: RefCell<BTreeSet<String>>,
    configs: RefCell<BTreeMap<String, ServiceConfig>>,
    log: RefCell<Vec<String>>,
}

impl MemoryProjects {
    /// Projects with `services`, whose commands every one exit 0 and print
    /// nothing until [`Self::answering`] says otherwise.
    pub fn new(services: &[&str]) -> MemoryProjects {
        MemoryProjects {
            services: services.iter().map(|s| s.to_string()).collect(),
            answer: Box::new(|_| Ok((Some(0), String::new(), String::new()))),
            available: true,
            volumes: BTreeSet::new(),
            files: RefCell::default(),
            folders: RefCell::default(),
            running: RefCell::default(),
            configs: RefCell::default(),
            log: RefCell::default(),
        }
    }

    /// Commands in a service are answered by `answer`.
    pub fn answering(
        mut self,
        answer: impl Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError> + 'static,
    ) -> MemoryProjects {
        self.answer = Box::new(answer);
        self
    }

    /// A named volume present before setup.
    pub fn with_volume(mut self, name: &str) -> Self {
        self.volumes.insert(name.to_owned());
        self
    }

    /// The tool is not installed.
    pub fn unavailable(mut self) -> MemoryProjects {
        self.available = false;
        self
    }

    /// `service`, once running, was started as `config` says.
    pub fn with_config(self, service: &str, config: ServiceConfig) -> MemoryProjects {
        self.configs.borrow_mut().insert(service.into(), config);
        self
    }

    /// A file there before the test, by absolute path.
    pub fn with_file(self, path: impl Into<PathBuf>, text: &str) -> MemoryProjects {
        self.files
            .borrow_mut()
            .insert(path.into(), (text.into(), false));
        self
    }

    /// The text of the file at `path`, and whether it is private.
    pub fn file_text(&self, path: &Path) -> Option<(String, bool)> {
        self.files.borrow().get(path).cloned()
    }

    /// Whether the folder at `path` was made.
    pub fn has_folder(&self, path: &Path) -> bool {
        self.folders.borrow().contains(path)
    }

    /// Every operation so far, as a person would type it, in order.
    pub fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn note(&self, line: String) {
        self.log.borrow_mut().push(line);
    }

    fn command(&self, args: &[&str]) -> String {
        let line = self.typed(args);
        self.note(line.clone());
        line
    }

    fn ask(&self, request: Request, command: String) -> Result<Ran, ProjectError> {
        let (status, stdout, stderr) = (self.answer)(&request)?;
        Ok(Ran {
            command,
            status,
            stdout,
            stderr,
        })
    }
}

fn at(project: &Project, path: &Path) -> PathBuf {
    project.folder.path().join(path)
}

fn user_args(user: User) -> Vec<&'static str> {
    match user {
        User::Default => vec![],
        User::Root => vec!["--user", "root"],
    }
}

impl Projects for MemoryProjects {
    fn available(&self, _: &Project) -> Result<(), ProjectError> {
        let command = self.command(&["version"]);
        if self.available {
            Ok(())
        } else {
            Err(ProjectError::Missing {
                command,
                cause: "not installed".into(),
            })
        }
    }

    fn has_volume(&self, _: &Project, name: &str) -> Result<bool, ProjectError> {
        Ok(self.volumes.contains(name))
    }

    fn tool(&self) -> &str {
        "tool"
    }

    fn typed(&self, args: &[&str]) -> String {
        let mut line = String::from("tool compose");
        for arg in args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }

    fn write(&self, project: &Project, files: &[ProjectFile]) -> Result<(), ProjectError> {
        for file in files {
            let path = at(project, file.path);
            self.note(format!("write {}", path.display()));
            self.files
                .borrow_mut()
                .insert(path, (file.text.into(), file.private));
        }
        Ok(())
    }

    fn make_folders(&self, project: &Project, folders: &[&Path]) -> Result<(), ProjectError> {
        for folder in folders {
            let path = at(project, folder);
            self.note(format!("mkdir {}", path.display()));
            self.folders.borrow_mut().insert(path);
        }
        Ok(())
    }

    fn read(&self, project: &Project, file: &Path) -> Result<String, ProjectError> {
        let path = at(project, file);
        self.files
            .borrow()
            .get(&path)
            .map(|(text, _)| text.clone())
            .ok_or_else(|| {
                ProjectError::file(
                    FileAction::Read,
                    &path,
                    &io::Error::new(io::ErrorKind::NotFound, "no such file"),
                )
            })
    }

    fn file(&self, project: &Project, file: &Path) -> FileState {
        let path = at(project, file);
        if self.files.borrow().contains_key(&path) {
            FileState::File
        } else if self.folders.borrow().contains(&path) {
            FileState::Other
        } else {
            FileState::Missing
        }
    }

    fn up(&self, _project: &Project, services: &[&str], _out: Output) -> Result<(), ProjectError> {
        let mut args = vec!["up", "-d"];
        args.extend(services);
        self.command(&args);
        let mut running = self.running.borrow_mut();
        if services.is_empty() {
            running.extend(self.services.iter().cloned());
        } else {
            running.extend(services.iter().map(|s| s.to_string()));
        }
        Ok(())
    }

    fn down(
        &self,
        _project: &Project,
        remove_volumes: bool,
        _out: Output,
    ) -> Result<(), ProjectError> {
        self.command(if remove_volumes {
            &["down", "--volumes"]
        } else {
            &["down"]
        });
        self.running.borrow_mut().clear();
        Ok(())
    }

    fn pull(&self, _project: &Project, _out: Output) -> Result<(), ProjectError> {
        self.command(&["pull"]);
        Ok(())
    }

    fn restart(&self, _project: &Project, service: &str, _out: Output) -> Result<(), ProjectError> {
        let command = self.command(&["restart", service]);
        if self.running.borrow().contains(service) {
            Ok(())
        } else {
            Err(ProjectError::Failed {
                command,
                status: Some(1),
                stdout: String::new(),
                stderr: format!("{service} is not running"),
            })
        }
    }

    fn running(&self, _project: &Project) -> Result<Vec<String>, ProjectError> {
        self.command(&["ps", "--status", "running", "--services"]);
        Ok(self.running.borrow().iter().cloned().collect())
    }

    fn exec(
        &self,
        _project: &Project,
        service: &str,
        user: User,
        args: &[&str],
        _out: Output,
    ) -> Result<Ran, ProjectError> {
        let mut line = vec!["exec"];
        line.extend(user_args(user));
        line.push(service);
        line.extend(args);
        let command = self.command(&line);
        self.ask(
            Request::Exec {
                service: service.into(),
                user,
                args: args.iter().map(|a| a.to_string()).collect(),
            },
            command,
        )
    }

    fn run_once(
        &self,
        _project: &Project,
        service: &str,
        user: User,
        entrypoint: &str,
        args: &[&str],
        _out: Output,
    ) -> Result<Ran, ProjectError> {
        let mut line = vec!["run", "--rm"];
        line.extend(user_args(user));
        line.extend(["--entrypoint", entrypoint, service]);
        line.extend(args);
        let command = self.command(&line);
        self.ask(
            Request::RunOnce {
                service: service.into(),
                user,
                entrypoint: entrypoint.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
            },
            command,
        )
    }

    fn copy_in(
        &self,
        _project: &Project,
        service: &str,
        host: &Path,
        inside: &str,
    ) -> Result<(), ProjectError> {
        let from = host.display().to_string();
        let to = format!("{service}:{inside}");
        self.command(&["cp", &from, &to]);
        Ok(())
    }

    fn copy_out(
        &self,
        _project: &Project,
        service: &str,
        inside: &str,
        host: &Path,
    ) -> Result<(), ProjectError> {
        let from = format!("{service}:{inside}");
        let to = host.display().to_string();
        self.command(&["cp", &from, &to]);
        Ok(())
    }

    fn inspect(&self, _project: &Project, service: &str) -> Result<ServiceConfig, ProjectError> {
        let command = self.command(&["inspect", service]);
        if !self.running.borrow().contains(service) {
            return Err(ProjectError::NoContainer {
                command,
                detail: format!("{service} is not running"),
            });
        }
        self.configs
            .borrow()
            .get(service)
            .cloned()
            .ok_or(ProjectError::Unexpected {
                command,
                detail: "no configuration".into(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Folder;

    fn project(folder: &Folder) -> Project<'_> {
        Project {
            folder,
            shielded: &["SETTING"],
        }
    }

    #[test]
    fn files_and_folders_are_kept_relative_to_the_project() {
        let projects = MemoryProjects::new(&["web"]);
        let folder = Folder("/p".into());
        let p = project(&folder);
        assert_eq!(projects.file(&p, Path::new(".env")), FileState::Missing);
        projects.make_folders(&p, &[Path::new("backups")]).unwrap();
        projects
            .write(
                &p,
                &[
                    ProjectFile {
                        path: Path::new(".env"),
                        text: "A=1\n",
                        private: false,
                    },
                    ProjectFile {
                        path: Path::new("/home/me/key"),
                        text: "secret",
                        private: true,
                    },
                ],
            )
            .unwrap();
        assert_eq!(projects.read(&p, Path::new(".env")).unwrap(), "A=1\n");
        assert_eq!(projects.file(&p, Path::new(".env")), FileState::File);
        assert_eq!(projects.file(&p, Path::new("backups")), FileState::Other);
        assert!(projects.has_folder(Path::new("/p/backups")));
        assert_eq!(
            projects.file_text(Path::new("/home/me/key")),
            Some(("secret".into(), true))
        );
        let missing = projects.read(&p, Path::new("gone")).unwrap_err();
        assert!(matches!(
            missing,
            ProjectError::File {
                action: FileAction::Read,
                cause: io::ErrorKind::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn services_start_stop_and_answer_as_the_test_says() {
        let projects = MemoryProjects::new(&["web", "proxy"])
            .answering(|request| match request {
                Request::Exec { args, .. } if args[0] == "fail" => {
                    Ok((Some(2), String::new(), "no".into()))
                }
                Request::Exec { .. } => Ok((Some(0), "said\n".into(), String::new())),
                Request::RunOnce { .. } => Err(ProjectError::Missing {
                    command: "x".into(),
                    cause: "gone".into(),
                }),
            })
            .with_config(
                "web",
                ServiceConfig {
                    command: Some(vec!["serve".into()]),
                    entrypoint: None,
                    env: None,
                    working_dir: None,
                },
            );
        let folder = Folder("/p".into());
        let p = project(&folder);
        projects.available(&p).unwrap();
        assert!(projects.running(&p).unwrap().is_empty());
        assert!(projects.restart(&p, "web", Output::Terminal).is_err());
        assert!(projects.inspect(&p, "web").is_err());
        projects.up(&p, &["web"], Output::Terminal).unwrap();
        assert_eq!(projects.running(&p).unwrap(), vec!["web".to_string()]);
        projects.up(&p, &[], Output::Terminal).unwrap();
        assert_eq!(projects.running(&p).unwrap(), vec!["proxy", "web"]);
        projects.restart(&p, "web", Output::Terminal).unwrap();
        assert_eq!(
            projects.inspect(&p, "web").unwrap().command,
            Some(vec!["serve".to_string()])
        );
        let ran = projects
            .exec(&p, "web", User::Root, &["ls"], Output::Captured)
            .unwrap();
        assert_eq!(ran.command, "tool compose exec --user root web ls");
        assert_eq!(ran.succeeded().unwrap(), "said\n");
        let failed = projects
            .exec(&p, "web", User::Default, &["fail"], Output::Captured)
            .unwrap();
        assert_eq!(failed.succeeded().unwrap_err().detail(), "exit 2: no");
        assert!(
            projects
                .run_once(&p, "web", User::Default, "sh", &[], Output::Captured)
                .is_err()
        );
        projects
            .copy_in(&p, "web", Path::new("/h/a"), "/w/a")
            .unwrap();
        projects
            .copy_out(&p, "web", "/w/.", Path::new("/h/out"))
            .unwrap();
        projects.pull(&p, Output::Terminal).unwrap();
        projects.down(&p, true, Output::Terminal).unwrap();
        assert!(projects.running(&p).unwrap().is_empty());
        assert_eq!(
            projects.log(),
            vec![
                "tool compose version",
                "tool compose ps --status running --services",
                "tool compose restart web",
                "tool compose inspect web",
                "tool compose up -d web",
                "tool compose ps --status running --services",
                "tool compose up -d",
                "tool compose ps --status running --services",
                "tool compose restart web",
                "tool compose inspect web",
                "tool compose exec --user root web ls",
                "tool compose exec web fail",
                "tool compose run --rm --entrypoint sh web",
                "tool compose cp /h/a web:/w/a",
                "tool compose cp web:/w/. /h/out",
                "tool compose pull",
                "tool compose down --volumes",
                "tool compose ps --status running --services",
            ]
        );
    }

    #[test]
    fn a_missing_tool_says_so() {
        let projects = MemoryProjects::new(&[]).unavailable();
        let folder = Folder("/p".into());
        assert!(matches!(
            projects.available(&project(&folder)),
            Err(ProjectError::Missing { .. })
        ));
        assert_eq!(projects.tool(), "tool");
    }
}
