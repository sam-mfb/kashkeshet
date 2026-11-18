//! `Projects` over Docker against a recording host: each operation's whole
//! command line, the folder it runs in, the variables it runs without, and
//! how each way of failing comes back.

use super::super::{Browser, Command, Docker, Finished, Runner};
use kkt_container::{
    FileAction, FileState, Folder, Output, Project, ProjectError, ProjectFile, Projects,
    ServiceConfig, User,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// What the host was asked to run: how, the command line, the folder and
/// the variables left out, sorted.
#[derive(Debug, PartialEq, Eq)]
struct Asked {
    attached: bool,
    line: String,
    cwd: Option<PathBuf>,
    unset: Vec<String>,
}

/// A host that keeps every command and answers each with the next reply.
#[derive(Default)]
struct Host {
    asked: RefCell<Vec<Asked>>,
    replies: RefCell<Vec<io::Result<Finished>>>,
    files: RefCell<BTreeMap<PathBuf, (String, bool)>>,
    dirs: RefCell<Vec<PathBuf>>,
    refuse: RefCell<Option<io::ErrorKind>>,
}

impl Host {
    fn answer(&self, command: &Command, attached: bool) -> io::Result<Finished> {
        let mut unset = command.unset.clone();
        unset.sort();
        self.asked.borrow_mut().push(Asked {
            attached,
            line: command.to_string(),
            cwd: command.current_dir.clone(),
            unset,
        });
        let mut replies = self.replies.borrow_mut();
        if replies.is_empty() {
            Ok(exit(0, "", ""))
        } else {
            replies.remove(0)
        }
    }
}

impl Runner for Rc<Host> {
    fn run(&self, command: &Command) -> io::Result<Finished> {
        self.answer(command, false)
    }
    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        self.answer(command, true)
    }
    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.dirs.borrow().iter().any(|d| d == path)
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }
    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.files
            .borrow()
            .get(path)
            .map(|(text, _)| text.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        if let Some(kind) = *self.refuse.borrow() {
            return Err(io::Error::new(kind, "refused"));
        }
        self.dirs.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()> {
        if let Some(kind) = *self.refuse.borrow() {
            return Err(io::Error::new(kind, "refused"));
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), (contents.into(), private));
        Ok(())
    }
    fn say(&self, _line: &str) {
        unreachable!("Projects says nothing to the person")
    }
    fn working_dir(&self) -> io::Result<PathBuf> {
        unreachable!("Projects takes every path from its caller")
    }
}

struct NoBrowser;

impl Browser for NoBrowser {
    fn open(&self, _url: &str) -> io::Result<()> {
        unreachable!("Projects opens no link")
    }
    fn open_failure(&self) -> Option<String> {
        None
    }
}

fn exit(code: i32, stdout: &str, stderr: &str) -> Finished {
    Finished {
        code: Some(code),
        stdout: stdout.into(),
        stderr: stderr.into(),
    }
}

fn over(replies: Vec<io::Result<Finished>>) -> (Docker, Rc<Host>) {
    let host = Rc::new(Host::default());
    *host.replies.borrow_mut() = replies;
    let docker = Docker {
        browser: Box::new(NoBrowser),
        host: Box::new(host.clone()),
    };
    (docker, host)
}

const SHIELDED: &[&str] = &["APP_PORT", "APP_VERSION"];

fn folder() -> Folder {
    Folder("/srv/app".into())
}

fn project(folder: &Folder) -> Project<'_> {
    Project {
        folder,
        shielded: SHIELDED,
    }
}

/// Compose's own variables and the project's, sorted.
fn unset() -> Vec<String> {
    let mut all: Vec<String> = [
        "COMPOSE_ENV_FILES",
        "COMPOSE_FILE",
        "COMPOSE_PATH_SEPARATOR",
        "COMPOSE_PROFILES",
        "COMPOSE_PROJECT_NAME",
    ]
    .iter()
    .chain(SHIELDED)
    .map(|v| v.to_string())
    .collect();
    all.sort();
    all
}

/// A `docker compose` command in the project's folder.
fn in_folder(attached: bool, args: &str) -> Asked {
    Asked {
        attached,
        line: format!("docker compose --project-directory /srv/app {args}"),
        cwd: Some(PathBuf::from("/srv/app")),
        unset: unset(),
    }
}

fn asked(host: &Host) -> Vec<Asked> {
    host.asked.take()
}

#[test]
fn available_asks_compose_for_its_version_anywhere() {
    let (docker, host) = over(vec![]);
    docker.available().unwrap();
    let mut compose_only: Vec<String> = unset()
        .into_iter()
        .filter(|v| v.starts_with("COMPOSE_"))
        .collect();
    compose_only.sort();
    assert_eq!(
        asked(&host),
        vec![Asked {
            attached: false,
            line: "docker compose version".into(),
            cwd: None,
            unset: compose_only,
        }]
    );
}

#[test]
fn the_tool_is_named_and_typed_as_a_person_would() {
    let (docker, host) = over(vec![]);
    assert_eq!(docker.tool(), "Docker");
    assert_eq!(docker.typed(&["logs", "web"]), "docker compose logs web");
    assert_eq!(
        docker.typed(&["exec", "web", "sh", "-c", "echo hi"]),
        "docker compose exec web sh -c \"echo hi\""
    );
    assert!(asked(&host).is_empty());
}

#[test]
fn up_starts_the_services_named_or_all_in_the_background() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    docker.up(&p, &[], Output::Terminal).unwrap();
    docker.up(&p, &["proxy"], Output::Captured).unwrap();
    assert_eq!(
        asked(&host),
        vec![in_folder(true, "up -d"), in_folder(false, "up -d proxy")]
    );
}

#[test]
fn down_keeps_the_volumes_unless_asked() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    docker.down(&p, false, Output::Terminal).unwrap();
    docker.down(&p, true, Output::Terminal).unwrap();
    assert_eq!(
        asked(&host),
        vec![in_folder(true, "down"), in_folder(true, "down --volumes")]
    );
}

#[test]
fn pull_and_restart() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    docker.pull(&p, Output::Terminal).unwrap();
    docker.restart(&p, "web", Output::Terminal).unwrap();
    assert_eq!(
        asked(&host),
        vec![in_folder(true, "pull"), in_folder(true, "restart web")]
    );
}

#[test]
fn running_lists_the_running_services() {
    let (docker, host) = over(vec![Ok(exit(0, "web\n\n proxy \n", ""))]);
    let folder = folder();
    assert_eq!(
        docker.running(&project(&folder)).unwrap(),
        vec!["web", "proxy"]
    );
    assert_eq!(
        asked(&host),
        vec![in_folder(false, "ps --status running --services")]
    );
}

#[test]
fn exec_runs_in_the_service_without_a_terminal_as_the_user_named() {
    let (docker, host) = over(vec![
        Ok(exit(0, "said\n", "")),
        Ok(exit(4, "", "none")),
        Ok(exit(0, "", "")),
    ]);
    let folder = folder();
    let p = project(&folder);
    let ran = docker
        .exec(
            &p,
            "web",
            User::Default,
            &["app", "key", "show"],
            Output::Captured,
        )
        .unwrap();
    assert_eq!(
        ran.command,
        "docker compose --project-directory /srv/app exec -T web app key show"
    );
    assert_eq!(ran.succeeded().unwrap(), "said\n");
    // A command that ran and failed is still an answer: the caller reads
    // its status.
    let failed = docker
        .exec(&p, "web", User::Default, &["app"], Output::Captured)
        .unwrap();
    assert_eq!(failed.status, Some(4));
    docker
        .exec(
            &p,
            "web",
            User::Root,
            &["rm", "-rf", "/tmp/x"],
            Output::Terminal,
        )
        .unwrap();
    assert_eq!(
        asked(&host),
        vec![
            in_folder(false, "exec -T web app key show"),
            in_folder(false, "exec -T web app"),
            in_folder(true, "exec -T -u root web rm -rf /tmp/x"),
        ]
    );
}

#[test]
fn run_once_runs_a_new_container_alone_and_removes_it() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    docker
        .run_once(
            &p,
            "web",
            User::Default,
            "sh",
            &["-c", "cat f"],
            Output::Captured,
        )
        .unwrap();
    docker
        .run_once(
            &p,
            "web",
            User::Root,
            "chown",
            &["10001", "/backups"],
            Output::Terminal,
        )
        .unwrap();
    assert_eq!(
        asked(&host),
        vec![
            in_folder(
                false,
                "run --rm --no-deps -T --entrypoint sh web -c \"cat f\""
            ),
            in_folder(
                true,
                "run --rm --no-deps --user root --entrypoint chown web 10001 /backups"
            ),
        ]
    );
}

#[test]
fn copies_name_the_service_on_their_inside_end() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    docker
        .copy_in(&p, "web", Path::new("/home/me/a.md"), "/tmp/w.1/a.md")
        .unwrap();
    docker
        .copy_out(&p, "web", "/tmp/w.1/.", Path::new("/home/me/out"))
        .unwrap();
    assert_eq!(
        asked(&host),
        vec![
            in_folder(false, "cp /home/me/a.md web:/tmp/w.1/a.md"),
            in_folder(false, "cp web:/tmp/w.1/. /home/me/out"),
        ]
    );
}

#[test]
fn inspect_finds_the_container_then_reads_how_it_was_started() {
    let config = r#"{"Cmd":["serve","--bind","0.0.0.0:80"],"Entrypoint":["/bin/app"],"Env":["HOME=/home/app","PATH"],"WorkingDir":"/data"}"#;
    let (docker, host) = over(vec![Ok(exit(0, "0123abcd\n", "")), Ok(exit(0, config, ""))]);
    let folder = folder();
    let found = docker.inspect(&project(&folder), "web").unwrap();
    assert_eq!(
        found,
        ServiceConfig {
            command: Some(vec!["serve".into(), "--bind".into(), "0.0.0.0:80".into()]),
            entrypoint: Some(vec!["/bin/app".into()]),
            env: Some(vec!["HOME=/home/app".into(), "PATH".into()]),
            working_dir: "/data".into(),
        }
    );
    assert_eq!(
        asked(&host),
        vec![
            in_folder(false, "ps --quiet web"),
            Asked {
                attached: false,
                line: "docker inspect --format \"{{json .Config}}\" 0123abcd".into(),
                cwd: None,
                unset: unset(),
            },
        ]
    );
}

#[test]
fn inspect_reports_fields_it_cannot_read_as_missing() {
    let (docker, _host) = over(vec![
        Ok(exit(0, "ab\n", "")),
        Ok(exit(0, r#"{"Cmd":null,"Entrypoint":["a",1],"Env":{}}"#, "")),
    ]);
    let folder = folder();
    assert_eq!(
        docker.inspect(&project(&folder), "web").unwrap(),
        ServiceConfig {
            command: None,
            entrypoint: None,
            env: None,
            working_dir: String::new(),
        }
    );
}

#[test]
fn inspect_refuses_an_answer_it_cannot_read() {
    let folder = folder();
    for (replies, command, detail) in [
        (
            vec![Ok(exit(0, "\n", ""))],
            "ps --quiet web",
            "no container ID for web",
        ),
        (
            vec![Ok(exit(0, "not-hex\n", ""))],
            "ps --quiet web",
            "no container ID for web",
        ),
    ] {
        let (docker, _host) = over(replies);
        assert_eq!(
            docker.inspect(&project(&folder), "web").unwrap_err(),
            ProjectError::Unexpected {
                command: format!("docker compose --project-directory /srv/app {command}"),
                detail: detail.into(),
            }
        );
    }
    let (docker, _host) = over(vec![Ok(exit(0, "ab\n", "")), Ok(exit(0, "{", ""))]);
    assert_eq!(
        docker.inspect(&project(&folder), "web").unwrap_err(),
        ProjectError::Unexpected {
            command: "docker inspect --format \"{{json .Config}}\" ab".into(),
            detail: "not JSON".into(),
        }
    );
}

#[test]
fn a_command_that_cannot_run_is_missing_and_one_that_fails_is_failed() {
    let folder = folder();
    let (docker, _host) = over(vec![Err(io::Error::new(
        io::ErrorKind::NotFound,
        "No such file or directory (os error 2)",
    ))]);
    let missing = docker.available().unwrap_err();
    assert_eq!(
        missing,
        ProjectError::Missing {
            command: "docker compose version".into(),
            cause: "No such file or directory (os error 2)".into(),
        }
    );
    let (docker, _host) = over(vec![Ok(exit(1, "", "no such service: web"))]);
    let failed = docker
        .restart(&project(&folder), "web", Output::Terminal)
        .unwrap_err();
    assert_eq!(
        failed.to_string(),
        "`docker compose --project-directory /srv/app restart web` failed (exit 1: no such service: web)"
    );
    // Worded exactly as the old Container code words a failed command.
    let finished = exit(1, "", "no such service: web");
    assert_eq!(failed.detail(), finished.detail());
    let (docker, _host) = over(vec![Ok(Finished {
        code: None,
        stdout: "half".into(),
        stderr: String::new(),
    })]);
    let signalled = docker
        .pull(&project(&folder), Output::Terminal)
        .unwrap_err();
    assert_eq!(signalled.detail(), "ended by a signal: half");
}

#[test]
fn files_are_read_written_and_made_relative_to_the_folder() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    assert_eq!(docker.file(&p, Path::new(".env")), FileState::Missing);
    docker
        .make_folders(&p, &[Path::new("backups"), Path::new("/abs/dir")])
        .unwrap();
    docker
        .write(
            &p,
            &[
                ProjectFile {
                    path: Path::new(".env"),
                    text: "A=1\n",
                    private: false,
                },
                ProjectFile {
                    path: Path::new("/home/me/copy"),
                    text: "k",
                    private: true,
                },
            ],
        )
        .unwrap();
    assert_eq!(docker.read(&p, Path::new(".env")).unwrap(), "A=1\n");
    assert_eq!(docker.file(&p, Path::new(".env")), FileState::File);
    assert_eq!(docker.file(&p, Path::new("backups")), FileState::Other);
    assert_eq!(
        *host.dirs.borrow(),
        vec![PathBuf::from("/srv/app/backups"), PathBuf::from("/abs/dir")]
    );
    assert_eq!(
        host.files.borrow().get(Path::new("/home/me/copy")),
        Some(&("k".to_string(), true))
    );
    assert!(asked(&host).is_empty());
}

#[test]
fn a_file_that_fails_says_what_was_being_done_to_which_path() {
    let (docker, host) = over(vec![]);
    let folder = folder();
    let p = project(&folder);
    let missing = docker.read(&p, Path::new("gone")).unwrap_err();
    assert_eq!(
        missing,
        ProjectError::File {
            action: FileAction::Read,
            path: "/srv/app/gone".into(),
            cause: io::ErrorKind::NotFound,
            detail: "no such file".into(),
        }
    );
    *host.refuse.borrow_mut() = Some(io::ErrorKind::PermissionDenied);
    let file = ProjectFile {
        path: Path::new(".env"),
        text: "",
        private: false,
    };
    assert!(matches!(
        docker.write(&p, &[file]).unwrap_err(),
        ProjectError::File {
            action: FileAction::Write,
            cause: io::ErrorKind::PermissionDenied,
            ..
        }
    ));
    assert_eq!(
        docker
            .make_folders(&p, &[Path::new("backups")])
            .unwrap_err()
            .to_string(),
        "could not create /srv/app/backups: refused"
    );
}
