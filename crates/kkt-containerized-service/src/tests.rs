//! Containerized over projects in memory: each operation it does, and how
//! each can fail.

use crate::{
    Arg, Carry, ConfigReader, ContainerConfig, Existing, InServer, KeyCopy, Login,
    ServiceContainer, State,
};
use crate::{Containerized, NodeLogin, NodeStatus, SharingNode, Tell};
use futures::executor::block_on;
use kkt_clock::Time;
use kkt_clock::testing::FixedClock;
use kkt_container::testing::{MemoryProjects, Request};
use kkt_container::{
    FileAction, Folder, Output, Project, ProjectError, ProjectFile, Projects, Ran, ServiceConfig,
    User,
};
use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const FOLDER: &str = "/data/docker";

fn folder() -> Folder {
    Folder(PathBuf::from(FOLDER))
}

fn at(name: &str) -> PathBuf {
    Path::new(FOLDER).join(name)
}

fn service(projects: MemoryProjects) -> Containerized<MemoryProjects> {
    working_in(projects, Ok(PathBuf::from("/home/me"))).0
}

/// The lines the service said, in order.
#[derive(Clone, Default)]
struct Said(Rc<RefCell<Vec<String>>>);

impl Tell for Said {
    fn say(&self, line: &str) {
        self.0.borrow_mut().push(line.to_string());
    }
}

/// The service over `projects`, in the folder `working_dir`, with what it
/// says.
fn working_in(
    projects: MemoryProjects,
    working_dir: Result<PathBuf, String>,
) -> (Containerized<MemoryProjects>, Said) {
    let said = Said::default();
    (over(projects, said.clone(), working_dir), said)
}

fn over<P: Projects>(
    projects: P,
    said: Said,
    working_dir: Result<PathBuf, String>,
) -> Containerized<P> {
    Containerized::new(
        projects,
        Box::new(said),
        Box::new(Node),
        Box::new(NoBrowser),
        Box::new(KeysIn(Rc::new(kkt_keys::testing::Memory::new()))),
        working_dir,
    )
}

fn failing(error: ProjectError) -> Containerized<Failing> {
    over(
        Failing(error),
        Said::default(),
        Ok(PathBuf::from("/home/me")),
    )
}

const LOCAL_ENV: &str = "KKT_VERSION=0.4.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\n";
const TAILSCALE_ENV: &str = "KKT_VERSION=0.4.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_HOSTNAME=kkt\nTS_TAILNET=tail1234.ts.net\n";

fn local(env: &str) -> MemoryProjects {
    MemoryProjects::new(&["kkt"])
        .with_file(at(".env"), env)
        .with_file(at("compose.yaml"), "services: {}\n")
}

fn tailscale(env: &str) -> MemoryProjects {
    MemoryProjects::new(&["kkt", "tailscale"])
        .with_file(at(".env"), env)
        .with_file(at("compose.tailscale.yaml"), "services: {}\n")
        .with_file(at("tailscale/serve.json"), "{}\n")
}

// ------------------------------------------------------------ settings file

#[test]
fn the_settings_file_is_there_only_when_something_is_at_its_path() {
    let none = service(MemoryProjects::new(&["kkt"]));
    assert!(!none.has_settings(&folder()));
    assert!(!none.settings_file(&folder()));

    let file = service(local(LOCAL_ENV));
    assert!(file.has_settings(&folder()));
    assert!(file.settings_file(&folder()));

    // A folder where the settings file should be is there, but is no file
    // to resume from.
    let projects = MemoryProjects::new(&["kkt"]);
    let project = Project {
        folder: &folder(),
        shielded: &[],
    };
    projects
        .make_folders(&project, &[Path::new(".env")])
        .unwrap();
    let dir = service(projects);
    assert!(dir.has_settings(&folder()));
    assert!(!dir.settings_file(&folder()));
}

#[test]
fn the_version_is_the_settings_files_kkt_version() {
    assert_eq!(
        service(local(LOCAL_ENV)).version(&folder()),
        Ok("0.4.0".into())
    );
    assert_eq!(
        service(local("KKT_PORT=7420\n"))
            .version(&folder())
            .map_err(|e| e.to_string()),
        Err("/data/docker/.env has no KKT_VERSION".into())
    );
    assert_eq!(
        service(MemoryProjects::new(&["kkt"]))
            .version(&folder())
            .map_err(|e| e.to_string()),
        Err("could not read /data/docker/.env: no such file".into())
    );
}

#[test]
fn a_sharing_key_is_read_where_the_person_named_it_and_trimmed() {
    let projects = MemoryProjects::new(&[]).with_file("/home/me/ts-key", "  tskey-abc\n");
    let service = service(projects);
    assert_eq!(
        service.sharing_key(Path::new("/home/me/ts-key")),
        Ok("tskey-abc".into())
    );
    assert_eq!(
        service
            .sharing_key(Path::new("/home/me/none"))
            .map_err(|e| e.to_string()),
        Err("could not read /home/me/none: no such file".into())
    );
}

// ----------------------------------------------------------------- existing

#[test]
fn no_settings_file_is_no_install() {
    assert_eq!(
        service(MemoryProjects::new(&["kkt"])).existing(&folder()),
        Ok(None)
    );
}

#[test]
fn a_local_install_is_its_port() {
    assert_eq!(
        service(local(LOCAL_ENV)).existing(&folder()),
        Ok(Some(Existing {
            sharing: false,
            node: None,
            port: 7420,
        }))
    );
}

#[test]
fn a_tailscale_install_is_its_port_and_node() {
    assert_eq!(
        service(tailscale(TAILSCALE_ENV)).existing(&folder()),
        Ok(Some(Existing {
            sharing: true,
            node: Some(("kkt".into(), "tail1234.ts.net".into())),
            port: 7420,
        }))
    );
}

#[test]
fn an_unfinished_or_foreign_install_is_refused_saying_why() {
    let cases: Vec<(MemoryProjects, &str)> = vec![
        (
            local("KKT_VERSION=0.4.0\nKKT_PORT=7420\nCOMPOSE_FILE=other.yaml\n"),
            "its settings file /data/docker/.env has COMPOSE_FILE=other.yaml, which kkt setup does not write",
        ),
        (
            MemoryProjects::new(&["kkt"]).with_file(at(".env"), LOCAL_ENV),
            "its Compose file /data/docker/compose.yaml is missing",
        ),
        (
            local("KKT_VERSION=0.4.0\n"),
            "its settings file /data/docker/.env has no KKT_PORT",
        ),
        (
            local("KKT_PORT=0\n"),
            "its settings file /data/docker/.env has KKT_PORT=0, which is not a port",
        ),
        (
            local("KKT_PORT=port\n"),
            "its settings file /data/docker/.env has KKT_PORT=port, which is not a port",
        ),
        (
            tailscale(&TAILSCALE_ENV.replace("TS_HOSTNAME=kkt\n", "")),
            "its settings file /data/docker/.env has no TS_HOSTNAME",
        ),
        (
            tailscale(&TAILSCALE_ENV.replace("TS_TAILNET=tail1234.ts.net\n", "")),
            "its settings file /data/docker/.env has no TS_TAILNET",
        ),
        (
            tailscale(&TAILSCALE_ENV.replace("tail1234.ts.net", "pending.invalid")),
            "its Tailscale login never finished, so its address is not known",
        ),
        (
            tailscale(&TAILSCALE_ENV.replace("TS_HOSTNAME=kkt", "TS_HOSTNAME=a.b")),
            "its settings file /data/docker/.env has TS_HOSTNAME=a.b, which kkt setup does not write",
        ),
        (
            tailscale(&TAILSCALE_ENV.replace("TS_HOSTNAME=kkt", "TS_HOSTNAME=-kkt")),
            "its settings file /data/docker/.env has TS_HOSTNAME=-kkt, which kkt setup does not write",
        ),
        (
            tailscale(&TAILSCALE_ENV.replace("tail1234.ts.net", "tail..net")),
            "its settings file /data/docker/.env has TS_TAILNET=tail..net, which kkt setup does not write",
        ),
    ];
    for (projects, refusal) in cases {
        assert_eq!(
            service(projects)
                .existing(&folder())
                .map_err(|e| e.to_string()),
            Err(refusal.into())
        );
    }
}

// ------------------------------------------------------------- up and down

#[test]
fn up_and_down_run_the_projects_services_and_down_may_remove_the_data() {
    let service = service(local(LOCAL_ENV));
    service.up(&folder()).unwrap();
    service.down(&folder(), false).unwrap();
    service.down(&folder(), true).unwrap();
    assert_eq!(
        service.projects.log(),
        [
            "tool compose up -d",
            "tool compose down",
            "tool compose down --volumes"
        ]
    );
}

/// Projects whose `up` and `down` fail as they are told to.
struct Failing(ProjectError);

impl Projects for Failing {
    fn available(&self, _: &Project) -> Result<(), ProjectError> {
        Ok(())
    }
    fn tool(&self) -> &str {
        "Tool"
    }
    fn typed(&self, args: &[&str]) -> String {
        format!("tool compose {}", args.join(" "))
    }
    fn write(&self, _: &Project, _: &[ProjectFile]) -> Result<(), ProjectError> {
        unreachable!()
    }
    fn make_folders(&self, _: &Project, _: &[&Path]) -> Result<(), ProjectError> {
        unreachable!()
    }
    fn read(&self, _: &Project, _: &Path) -> Result<String, ProjectError> {
        unreachable!()
    }
    fn file(&self, _: &Project, _: &Path) -> kkt_container::FileState {
        unreachable!()
    }
    fn up(&self, project: &Project, _: &[&str], out: Output) -> Result<(), ProjectError> {
        assert_eq!(project.shielded, crate::files::SHIELDED);
        assert_eq!(out, Output::Terminal);
        Err(self.0.clone())
    }
    fn down(&self, _: &Project, _: bool, out: Output) -> Result<(), ProjectError> {
        assert_eq!(out, Output::Terminal);
        Err(self.0.clone())
    }
    fn pull(&self, _: &Project, _: Output) -> Result<(), ProjectError> {
        unreachable!()
    }
    fn restart(&self, _: &Project, _: &str, _: Output) -> Result<(), ProjectError> {
        unreachable!()
    }
    fn running(&self, _: &Project) -> Result<Vec<String>, ProjectError> {
        unreachable!()
    }
    fn exec(
        &self,
        _: &Project,
        _: &str,
        _: User,
        _: &[&str],
        _: Output,
    ) -> Result<Ran, ProjectError> {
        unreachable!()
    }
    fn run_once(
        &self,
        _: &Project,
        _: &str,
        _: User,
        _: &str,
        _: &[&str],
        _: Output,
    ) -> Result<Ran, ProjectError> {
        unreachable!()
    }
    fn copy_in(&self, _: &Project, _: &str, _: &Path, _: &str) -> Result<(), ProjectError> {
        unreachable!()
    }
    fn copy_out(&self, _: &Project, _: &str, _: &str, _: &Path) -> Result<(), ProjectError> {
        unreachable!()
    }
    fn inspect(&self, _: &Project, _: &str) -> Result<ServiceConfig, ProjectError> {
        unreachable!()
    }
}

#[test]
fn a_tool_that_cannot_run_is_not_available_by_its_name_and_a_failure_says_how() {
    let missing = failing(ProjectError::Missing {
        command: "tool compose up -d".into(),
        cause: "no such file".into(),
    });
    assert_eq!(
        missing.up(&folder()).map_err(|e| e.to_string()),
        Err("Tool is not available: could not run `tool compose up -d`: no such file".into())
    );
    let failed = failing(ProjectError::Failed {
        command: "tool compose down".into(),
        status: Some(1),
        stdout: String::new(),
        stderr: "no network\n".into(),
    });
    assert_eq!(
        failed.down(&folder(), false).map_err(|e| e.to_string()),
        Err("`tool compose down` failed (exit 1: no network)".into())
    );
    let file = failing(ProjectError::file(
        FileAction::Read,
        Path::new("/x"),
        &io::Error::other("gone"),
    ));
    assert_eq!(
        file.up(&folder()).map_err(|e| e.to_string()),
        Err("could not read /x: gone".into())
    );
}

// ----------------------------------------------------------------- runs in the server

/// Answers `mktemp` with a folder named as asked, and the rest with `rest`.
fn in_server(
    rest: impl Fn(&Request) -> Option<(Option<i32>, String, String)> + 'static,
) -> MemoryProjects {
    local(LOCAL_ENV).answering(move |request| {
        if let Some(answer) = rest(request) {
            return Ok(answer);
        }
        Ok(match request {
            Request::Exec { args, .. } if args[0] == "mktemp" => {
                (Some(0), "/tmp/run.Abc123\n".into(), String::new())
            }
            Request::Exec { args, .. } if args[0] == "kkt" => {
                (Some(0), "Imported.\n".into(), String::new())
            }
            _ => (Some(0), String::new(), String::new()),
        })
    })
}

fn command(carry: Carry) -> InServer<'static> {
    let args: &'static [Arg] =
        Box::leak(vec![Arg::Text("lanes".into()), Arg::Folder].into_boxed_slice());
    InServer {
        args,
        folder_prefix: "/tmp/run",
        carry,
    }
}

#[test]
fn a_run_in_the_server_carries_files_in_runs_and_removes_its_folder() {
    let service = service(in_server(|_| None));
    let carried = Carry::In(vec![PathBuf::from("/home/me/one.md")]);
    assert_eq!(
        service.run_in_server(&folder(), &command(carried)),
        Ok("Imported.\n".into())
    );
    assert_eq!(
        service.projects.log(),
        [
            "tool compose exec kkt mktemp -d /tmp/run.XXXXXXXXXX",
            "tool compose cp /home/me/one.md kkt:/tmp/run.Abc123/one.md",
            "tool compose exec --user root kkt chmod -R a+rX /tmp/run.Abc123",
            "tool compose exec kkt kkt lanes /tmp/run.Abc123",
            "tool compose exec --user root kkt rm -rf /tmp/run.Abc123",
        ]
    );
}

#[test]
fn a_run_in_the_server_carries_files_out_after_it() {
    let service = service(in_server(|_| None));
    let carried = Carry::Out(PathBuf::from("/home/me/export"));
    assert!(service.run_in_server(&folder(), &command(carried)).is_ok());
    assert!(service.projects.has_folder(Path::new("/home/me/export")));
    assert_eq!(
        service.projects.log()[2..4],
        [
            "mkdir /home/me/export",
            "tool compose cp kkt:/tmp/run.Abc123/. /home/me/export",
        ]
    );
}

#[test]
fn a_run_refuses_a_folder_mktemp_did_not_make_as_asked() {
    for said in ["/tmp/other\n", "/tmp/run.\n", "/tmp/run.a/b\n"] {
        let service = service(in_server(move |request| match request {
            Request::Exec { args, .. } if args[0] == "mktemp" => {
                Some((Some(0), said.into(), String::new()))
            }
            _ => None,
        }));
        assert_eq!(
            service
                .run_in_server(&folder(), &command(Carry::In(vec![])))
                .map_err(|e| e.to_string()),
            Err(format!(
                "unexpected output from the folder mktemp made in the container: {said}"
            ))
        );
    }
}

#[test]
fn a_failed_run_whose_folder_cannot_be_removed_says_so_and_gives_the_runs_error() {
    let (service, said) = working_in(
        in_server(|request| match request {
            Request::Exec { args, .. } if args[0] == "kkt" => {
                Some((Some(1), String::new(), "no title".into()))
            }
            Request::Exec { args, .. } if args[0] == "rm" => {
                Some((Some(1), String::new(), "busy".into()))
            }
            _ => None,
        }),
        Ok(PathBuf::from("/home/me")),
    );
    assert_eq!(
        service
            .run_in_server(&folder(), &command(Carry::In(vec![])))
            .map_err(|e| e.to_string()),
        Err("`tool compose exec kkt kkt lanes /tmp/run.Abc123` failed (exit 1: no title)".into())
    );
    assert_eq!(
        *said.0.borrow(),
        ["Could not remove /tmp/run.Abc123 in the container: \
          `tool compose exec --user root kkt rm -rf /tmp/run.Abc123` failed (exit 1: busy)"]
    );
}

#[test]
fn a_config_home_file_is_read_in_a_new_container_and_only_a_plain_path_is_taken() {
    let service = service(
        local(LOCAL_ENV)
            .answering(|_| Ok((Some(0), "name_pack = \"birds\"\n".into(), String::new()))),
    );
    assert_eq!(
        service.config_home_file(&folder(), Path::new("kkt/config.toml")),
        Ok("name_pack = \"birds\"\n".into())
    );
    assert!(service.projects.log()[0].starts_with("tool compose run --rm --entrypoint sh kkt -c "));
    for refused in ["/etc/passwd", "kkt/../x", "a b", "", "kkt//x"] {
        assert_eq!(
            service
                .config_home_file(&folder(), Path::new(refused))
                .map_err(|e| e.to_string()),
            Err(format!(
                "{refused} is not a plain path under the config home"
            ))
        );
    }
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_refused_as_not_plain_or_as_an_unreadable_config_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let odd = Path::new(OsStr::from_bytes(b"kkt/\xff.toml"));
    assert_eq!(
        crate::in_server::config_script(odd).map_err(|e| e.to_string()),
        Err(format!(
            "{} is not a plain path under the config home",
            odd.display()
        ))
    );
    assert_eq!(
        crate::status::config_path_text(odd),
        Err("unreadable config path".to_string())
    );
}

// ----------------------------------------------------------------- backups

fn backing_up(said: &'static str) -> MemoryProjects {
    local(LOCAL_ENV).answering(move |_| Ok((Some(0), said.into(), String::new())))
}

#[test]
fn a_backup_stays_in_the_managed_folder_or_is_copied_where_the_person_asks() {
    let said = "Backed up to /backups/kkt-backup-1\n";
    let managed = service(backing_up(said));
    assert_eq!(
        managed.backup(&folder(), None),
        Ok(at("backups/kkt-backup-1"))
    );
    let copied = service(backing_up(said));
    assert_eq!(
        copied.backup(&folder(), Some(Path::new("saved"))),
        Ok(PathBuf::from("/home/me/saved/kkt-backup-1"))
    );
    assert_eq!(
        copied.projects.log(),
        [
            "tool compose exec kkt kkt backup /backups",
            "mkdir /home/me/saved",
            "tool compose cp kkt:/backups/kkt-backup-1 /home/me/saved",
        ]
    );
}

#[test]
fn a_backup_says_what_went_wrong() {
    let (lost, _) = working_in(
        backing_up("Backed up to /backups/x\n"),
        Err("no such folder".into()),
    );
    assert_eq!(
        lost.backup(&folder(), Some(Path::new("saved")))
            .map_err(|e| e.to_string()),
        Err("could not find the current folder: no such folder".into())
    );
    let odd = service(backing_up("Done.\n"));
    assert_eq!(
        odd.backup(&folder(), None).map_err(|e| e.to_string()),
        Err("unexpected output from `kkt backup`: Done.".into())
    );
    let nameless = service(backing_up("Backed up to /\n"));
    assert_eq!(
        nameless.backup(&folder(), None).map_err(|e| e.to_string()),
        Err("unexpected output from `kkt backup`: /".into())
    );
}

// ----------------------------------------------------------------- the web key

#[test]
fn the_web_keys_copy_is_compared_without_showing_either_key() {
    let with = |server: &'static str, status: Option<i32>| {
        service(
            local(LOCAL_ENV)
                .with_file("/home/me/web-key", "web-key-1\n")
                .answering(move |_| Ok((status, server.into(), String::new()))),
        )
    };
    let key = Path::new("/home/me/web-key");
    assert_eq!(
        with("web-key-1\n", Some(0)).compare_web_key(&folder(), key),
        Ok(KeyCopy::Same)
    );
    assert_eq!(
        with("web-key-2\n", Some(0)).compare_web_key(&folder(), key),
        Ok(KeyCopy::Differs)
    );
    assert_eq!(
        with("web-key-2\n", Some(1))
            .compare_web_key(&folder(), key)
            .map_err(|e| e.to_string()),
        Err("`tool compose exec kkt kkt key show --web` failed (exit 1, no key)".into())
    );
    assert_eq!(
        with("web-key-2\n", None)
            .compare_web_key(&folder(), key)
            .map_err(|e| e.to_string()),
        Err("`tool compose exec kkt kkt key show --web` failed (ended by a signal)".into())
    );
}

// ----------------------------------------------------------------- status

struct Packs;

impl ConfigReader for Packs {
    fn settings(&self, text: &str, _: &Path, _: Option<&Path>) -> Result<ContainerConfig, String> {
        Ok(ContainerConfig {
            name_pack: text.strip_prefix("pack=").map(|p| p.trim().to_string()),
            max_lane_chars: None,
        })
    }
}

fn serving(args: &[&str]) -> ServiceConfig {
    ServiceConfig {
        command: Some(args.iter().map(|a| a.to_string()).collect()),
        entrypoint: Some(vec!["/usr/local/bin/kkt".into()]),
        env: Some(vec!["HOME=/home/kkt".into()]),
        working_dir: Some("/data".into()),
    }
}

fn status_of(projects: MemoryProjects) -> State {
    service(projects)
        .status(
            &folder(),
            &Packs,
            "default-pack",
            Path::new("kkt/config.toml"),
        )
        .unwrap()
}

#[test]
fn status_says_what_is_there_and_running_and_the_name_pack_serve_uses() {
    let none = status_of(MemoryProjects::new(&["kkt"]));
    assert!(!none.files_present);
    let stopped = status_of(local(LOCAL_ENV));
    assert!(stopped.files_present && !stopped.running);
    assert_eq!(stopped.version.as_deref(), Some("0.4.0"));
    assert_eq!(stopped.name_pack, Err("container is not running".into()));
    let flagged = local(LOCAL_ENV).with_config("kkt", serving(&["serve", "--name-pack=birds"]));
    flagged
        .up(
            &Project {
                folder: &folder(),
                shielded: &[],
            },
            &[],
            Output::Captured,
        )
        .unwrap();
    assert_eq!(status_of(flagged).name_pack, Ok("birds".into()));
    let configured = local(LOCAL_ENV)
        .with_config("kkt", serving(&["serve"]))
        .answering(|request| match request {
            Request::Exec { args, .. } if args[0] == "cat" => {
                Ok((Some(0), "pack=stones\n".into(), String::new()))
            }
            _ => Ok((Some(0), String::new(), String::new())),
        });
    configured
        .up(
            &Project {
                folder: &folder(),
                shielded: &[],
            },
            &[],
            Output::Captured,
        )
        .unwrap();
    assert_eq!(status_of(configured).name_pack, Ok("stones".into()));
}

#[test]
fn a_name_pack_with_no_container_is_unidentified() {
    let project_folder = folder();
    let project = Project {
        folder: &project_folder,
        shielded: &[],
    };
    assert_eq!(
        crate::status::name_pack(
            &local(LOCAL_ENV),
            &project,
            &Packs,
            "default-pack",
            Path::new("kkt/config.toml"),
        ),
        Err("could not identify the kkt container".into())
    );
}

/// A sharing node whose status answer is its login in a word: `in`,
/// `out` or any other state; an empty answer cannot be read.
struct Node;

impl SharingNode for Node {
    fn status_command(&self) -> Vec<String> {
        ["tailscale", "status", "--json"].map(String::from).to_vec()
    }

    fn login(&self, answer: &str) -> Option<NodeLogin> {
        match answer {
            "" => None,
            "in" => Some(NodeLogin::LoggedIn),
            "out" => Some(NodeLogin::NeedsLogin),
            other => Some(NodeLogin::Other(other.to_string())),
        }
    }

    /// `in <name> <https|plain>`, `out`, `out <link>`, or another word, a
    /// state; anything empty cannot be read.
    fn status(&self, answer: &str) -> Result<NodeStatus, String> {
        let words: Vec<&str> = answer.split_whitespace().collect();
        let node = |login, link: Option<&str>, name: Option<&str>, https| NodeStatus {
            login,
            link: link.map(str::to_string),
            name: name.map(str::to_string),
            https,
        };
        match words.as_slice() {
            [] => Err("no BackendState".into()),
            ["in", name, https] => Ok(node(
                NodeLogin::LoggedIn,
                None,
                Some(name).filter(|n| **n != "-").copied(),
                *https == "https",
            )),
            ["out"] => Ok(node(NodeLogin::NeedsLogin, None, None, false)),
            ["out", link] => Ok(node(NodeLogin::NeedsLogin, Some(link), None, false)),
            [other, ..] => Ok(node(NodeLogin::Other(other.to_string()), None, None, false)),
        }
    }
}

#[test]
fn a_shared_installs_status_says_whether_tailscale_is_logged_in() {
    for (said, login) in [
        ("in", Login::LoggedIn),
        ("out", Login::NeedsLogin),
        ("Starting", Login::Other("Starting".into())),
        ("", Login::Other("an unreadable answer".into())),
    ] {
        let projects =
            tailscale(TAILSCALE_ENV).answering(move |_| Ok((Some(0), said.into(), String::new())));
        assert_eq!(status_of(projects).login, Some(login));
    }
    let unreachable =
        tailscale(TAILSCALE_ENV).answering(|_| Ok((Some(1), String::new(), "not running".into())));
    assert_eq!(status_of(unreachable).login, Some(Login::Unreachable));
}

/// A sharing node asked with a command of its own, read as [`Node`] reads.
struct OwnCommand;

impl SharingNode for OwnCommand {
    fn status_command(&self) -> Vec<String> {
        vec!["share-node".into(), "state".into()]
    }
    fn login(&self, answer: &str) -> Option<NodeLogin> {
        Node.login(answer)
    }
    fn status(&self, answer: &str) -> Result<NodeStatus, String> {
        Node.status(answer)
    }
}

#[test]
fn the_sharing_node_is_asked_with_the_command_it_gives() {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let seen = asked.clone();
    let projects = tailscale(TAILSCALE_ENV).answering(move |request| {
        if let Request::Exec { service, args, .. } = request
            && service == "tailscale"
        {
            seen.borrow_mut().push(args.clone());
        }
        Ok((Some(0), "in".into(), String::new()))
    });
    let state = Containerized::new(
        projects,
        Box::new(Said::default()),
        Box::new(OwnCommand),
        Box::new(NoBrowser),
        Box::new(KeysIn(Rc::new(kkt_keys::testing::Memory::new()))),
        Ok(PathBuf::from("/home/me")),
    )
    .status(
        &folder(),
        &Packs,
        "default-pack",
        Path::new("kkt/config.toml"),
    )
    .unwrap();
    assert_eq!(state.login, Some(Login::LoggedIn));
    assert_eq!(
        *asked.borrow(),
        vec![vec!["share-node".to_string(), "state".to_string()]]
    );
}

#[test]
fn the_service_files_are_the_repositorys_own() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for tailscale in [false, true] {
        for (name, text) in crate::files::embedded(tailscale) {
            let file = std::fs::read_to_string(root.join(name)).unwrap();
            assert_eq!(text, file, "{name}");
        }
    }
}

// ---------------------------------------------------------------- setup

/// A browser that opens nothing and records each link asked for.
#[derive(Clone, Default)]
struct NoBrowser;

impl crate::Browser for NoBrowser {
    fn open(&self, _: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::NotFound, "no opener"))
    }
    fn open_failure(&self) -> Option<String> {
        None
    }
}

/// kkt-keys' key files in memory, shared so a test can look after.
struct KeysIn(Rc<kkt_keys::testing::Memory>);

impl kkt_keys::KeyFiles for KeysIn {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.0.read(path)
    }
    fn try_exists(&self, path: &Path) -> io::Result<bool> {
        self.0.try_exists(path)
    }
    fn is_file(&self, path: &Path) -> bool {
        self.0.is_file(path)
    }
    fn mode(&self, path: &Path) -> io::Result<Option<u32>> {
        self.0.mode(path)
    }
    fn create_dir_all(&self, dir: &Path) -> Result<(), String> {
        self.0.create_dir_all(dir)
    }
    fn keep_staged(&self, staged: &Path, path: &Path) -> Result<(), String> {
        self.0.keep_staged(staged, path)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        self.0.remove(path)
    }
    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String> {
        self.0.write_key(path, key, force)
    }
    fn write_admin_name(&self, admin_key: &Path, bound: Option<&str>) -> io::Result<()> {
        self.0.write_admin_name(admin_key, bound)
    }
}

/// A login link that records what it was shown.
#[derive(Default)]
struct Shown(RefCell<Vec<String>>);

impl crate::LoginLink for Shown {
    fn show(&self, url: &str) {
        self.0.borrow_mut().push(url.to_string());
    }
    fn open_requested(&self) -> bool {
        false
    }
}

const ACCESS: &str = "access-key-0123456789";
const WEB: &str = "web-key-0123456789";
const ADMIN: &str = "admin-key-0123456789";

fn key_copies() -> crate::KeyCopies {
    crate::KeyCopies {
        access: PathBuf::from("/home/me/.config/kkt/docker-access-key"),
        web: PathBuf::from("/home/me/.config/kkt/docker-web-key"),
        admin: PathBuf::from("/home/me/.config/kkt-admin/docker-admin-key"),
    }
}

fn inputs(sharing: bool) -> crate::Inputs {
    crate::Inputs {
        version: "0.4.0".into(),
        project: "kkt".into(),
        port: 7420,
        allow_hosts: vec![],
        sharing: sharing.then(|| crate::Sharing {
            hostname: "kkt".into(),
            auth_key: None,
        }),
    }
}

/// What one setup did: its result, what it said, the links it showed, the
/// key files, and the projects' log.
struct SetUp {
    result: Result<crate::Provisioned, String>,
    said: Vec<String>,
    shown: Vec<String>,
    keys: Rc<kkt_keys::testing::Memory>,
    log: Vec<String>,
}

/// Sets up over projects whose commands in a service `answer` answers;
/// each answer moves the clock on three seconds, so waits end.
fn set_up(
    sharing: bool,
    keys: kkt_keys::testing::Memory,
    answer: impl Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError> + 'static,
) -> SetUp {
    let clock = std::sync::Arc::new(FixedClock::new(
        Time::from_parts(2026, 11, 17, 12, 0, 0).unwrap(),
    ));
    let ticking = clock.clone();
    let projects = MemoryProjects::new(&["kkt", "tailscale"]).answering(move |request| {
        ticking.advance(kkt_clock::Span::seconds(3));
        answer(request)
    });
    let said = Said::default();
    let keys = Rc::new(keys);
    let service = Containerized::new(
        projects,
        Box::new(said.clone()),
        Box::new(Node),
        Box::new(NoBrowser),
        Box::new(KeysIn(keys.clone())),
        Ok(PathBuf::from("/home/me")),
    );
    let shown = Shown::default();
    let result =
        block_on(service.provision(&*clock, &shown, &folder(), &inputs(sharing), &key_copies()));
    let log = service.projects.log();
    let said = said.0.borrow().clone();
    SetUp {
        result: result.map_err(|e| e.to_string()),
        said,
        shown: shown.0.into_inner(),
        keys,
        log,
    }
}

/// The running server's answers: its keys, with `admin` for the admin key
/// show, and `tailscale` for the Tailscale container's status.
fn server(
    admin: (Option<i32>, &'static str, &'static str),
    tailscale: Vec<&'static str>,
) -> impl Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError> {
    let tailscale = RefCell::new(tailscale);
    move |request| {
        let Request::Exec { service, args, .. } = request else {
            return Ok((Some(0), String::new(), String::new()));
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(match (service.as_str(), args.as_slice()) {
            ("tailscale", _) => {
                let mut answers = tailscale.borrow_mut();
                let next = if answers.len() > 1 {
                    answers.remove(0)
                } else {
                    answers[0]
                };
                (Some(0), next.to_string(), String::new())
            }
            (_, [.., "--admin"]) => (admin.0, admin.1.to_string(), admin.2.to_string()),
            (_, [.., "--web"]) => (Some(0), format!("{WEB}\n"), String::new()),
            _ => (Some(0), format!("{ACCESS}\n"), String::new()),
        })
    }
}

fn no_admin() -> (Option<i32>, &'static str, &'static str) {
    (Some(4), "", "there is no admin key")
}

/// No key is in `text`.
fn keyless(text: &str) -> bool {
    [ACCESS, WEB, ADMIN].iter().all(|key| !text.contains(key))
}

#[test]
fn setup_writes_the_files_starts_kkt_and_replaces_each_key_copy() {
    let old = kkt_keys::testing::Memory::new()
        .with_file("/home/me/.config/kkt/docker-access-key", "old\n")
        .with_file("/home/me/.config/kkt/docker-web-key", "old\n")
        .with_file("/home/me/.config/kkt-admin/docker-admin-key", "old\n");
    let done = set_up(
        false,
        old,
        server((Some(0), "admin-key-0123456789\n", ""), vec![]),
    );
    assert_eq!(done.result, Ok(crate::Provisioned { node: None }));
    for (path, key) in [
        ("/home/me/.config/kkt/docker-access-key", ACCESS),
        ("/home/me/.config/kkt/docker-web-key", WEB),
        ("/home/me/.config/kkt-admin/docker-admin-key", ADMIN),
    ] {
        let file = done.keys.file(Path::new(path)).expect("a copy");
        assert_eq!(file.text, format!("{key}\n"), "{path}");
        assert_eq!(file.mode, Some(kkt_keys::ACCESS_KEY_FILE_MODE), "{path}");
    }
    let commands: Vec<&String> = done.log.iter().filter(|l| l.starts_with("tool ")).collect();
    assert_eq!(
        commands[..4],
        [
            "tool compose version",
            "tool compose run --rm --user root --entrypoint chown kkt 10001 /backups",
            "tool compose up -d",
            "tool compose exec kkt kkt key show",
        ]
    );
    assert!(done.said.iter().all(|line| keyless(line)));
}

#[test]
fn no_key_reaches_an_error_or_a_line_said_even_when_a_key_show_fails_after_printing_one() {
    for admin in [
        (
            Some(1),
            "admin-key-0123456789\n",
            "partial admin-key-0123456789",
        ),
        (None, "admin-key-0123456789\n", ""),
        (Some(0), "  \n", ""),
    ] {
        let done = set_up(
            false,
            kkt_keys::testing::Memory::new(),
            server(admin, vec![]),
        );
        let error = done.result.unwrap_err();
        assert!(keyless(&error), "{error}");
        assert!(
            error.contains("could not read the admin key") || error.contains("ended by a signal"),
            "{error}"
        );
        assert!(done.said.iter().all(|line| keyless(line)));
    }
    // The access key's show keeps failing after printing the key: setup
    // waits, then says so without it.
    let done = set_up(false, kkt_keys::testing::Memory::new(), |request| {
        Ok(match request {
            Request::Exec { .. } => (Some(1), format!("{ACCESS}\n"), ACCESS.to_string()),
            _ => (Some(0), String::new(), String::new()),
        })
    });
    let error = done.result.unwrap_err();
    assert_eq!(
        error,
        "kkt did not start or did not make its keys in time; `tool compose logs kkt` shows why"
    );
}

#[test]
fn a_shared_setup_shows_the_link_once_restarts_a_node_with_none_and_names_the_node() {
    let done = set_up(
        true,
        kkt_keys::testing::Memory::new(),
        server(
            no_admin(),
            vec![
                "out",
                "out",
                "out",
                "out",
                "out",
                "out",
                "out https://login/a",
                "out https://login/a",
                "in kkt-1.tail1.ts.net https",
            ],
        ),
    );
    assert_eq!(
        done.result,
        Ok(crate::Provisioned {
            node: Some(("kkt-1".into(), "tail1.ts.net".into()))
        })
    );
    assert_eq!(done.shown, ["https://login/a"]);
    assert_eq!(
        done.log
            .iter()
            .filter(|l| l.contains("restart tailscale"))
            .count(),
        1
    );
    assert!(
        done.said
            .contains(&"Logged in to Tailscale as kkt-1.tail1.ts.net.".to_string())
    );
}

#[test]
fn a_shared_setup_says_why_it_cannot_finish() {
    for (answer, error) in [
        (
            "in kkt-1.tail1.ts.net plain",
            "HTTPS certificates are not enabled",
        ),
        ("in - https", "logged in, but the node has no name"),
        (
            "in kkt-1 https",
            "the node name \"kkt-1\" has no tailnet part",
        ),
        (
            "",
            "unexpected output from `tailscale status --json`: no BackendState",
        ),
    ] {
        let done = set_up(
            true,
            kkt_keys::testing::Memory::new(),
            server(no_admin(), vec![answer]),
        );
        let said = done.result.unwrap_err();
        assert!(said.contains(error), "{answer}: {said}");
    }
}

#[test]
fn a_missing_tool_is_said_by_its_name_with_what_to_do() {
    let clock = FixedClock::new(Time::from_parts(2026, 11, 17, 12, 0, 0).unwrap());
    let service = service(MemoryProjects::new(&["kkt"]).unavailable());
    let error = block_on(service.provision(
        &clock,
        &Shown::default(),
        &folder(),
        &inputs(false),
        &key_copies(),
    ))
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("tool is not available: could not run `tool compose version`"),
        "{error}"
    );
    assert!(
        error.ends_with(". Install tool with Compose and run setup again"),
        "{error}"
    );
}

// ---------------------------------------------------------------- the admin key and upgrade

/// Over running projects whose commands in a service `answer` answers,
/// with `keys` as the key files and `env` as the settings file, the
/// service and the lines it says.
fn installed(
    keys: Rc<kkt_keys::testing::Memory>,
    env: &str,
    answer: impl Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError> + 'static,
) -> (
    Containerized<MemoryProjects>,
    Said,
    std::sync::Arc<FixedClock>,
) {
    let clock = std::sync::Arc::new(FixedClock::new(
        Time::from_parts(2026, 11, 17, 12, 0, 0).unwrap(),
    ));
    let ticking = clock.clone();
    let projects = MemoryProjects::new(&["kkt"])
        .with_file(at(".env"), env)
        .answering(move |request| {
            ticking.advance(kkt_clock::Span::seconds(3));
            answer(request)
        });
    let said = Said::default();
    let service = Containerized::new(
        projects,
        Box::new(said.clone()),
        Box::new(Node),
        Box::new(NoBrowser),
        Box::new(KeysIn(keys)),
        Ok(PathBuf::from("/home/me")),
    );
    service.up(&folder()).unwrap();
    (service, said, clock)
}

const LOCAL_SETTINGS: &str = "KKT_VERSION=0.4.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\n";

/// `answer`, with `kkt backup` answered as a backup that worked.
fn with_backup(
    answer: impl Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError> + 'static,
) -> impl Fn(&Request) -> Result<(Option<i32>, String, String), ProjectError> + 'static {
    move |request| match request {
        Request::Exec { args, .. } if args.iter().any(|a| a == "backup") => Ok((
            Some(0),
            "Backed up to /backups/kkt-backup-1\n".into(),
            String::new(),
        )),
        _ => answer(request),
    }
}

#[test]
fn the_admin_key_is_made_kkt_restarted_and_every_copy_replaced() {
    let keys = Rc::new(
        kkt_keys::testing::Memory::new()
            .with_file("/home/me/.config/kkt/docker-access-key", "old\n")
            .with_file("/home/me/.config/kkt-admin/docker-admin-key", "old\n"),
    );
    let made = Rc::new(RefCell::new(false));
    let making = made.clone();
    let (service, said, clock) = installed(keys.clone(), LOCAL_SETTINGS, move |request| {
        let Request::Exec { args, .. } = request else {
            return Ok((Some(0), String::new(), String::new()));
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(match args.as_slice() {
            [.., "create", "--admin", "--name", "kkt-admin"] => {
                *making.borrow_mut() = true;
                (Some(0), String::new(), String::new())
            }
            [.., "--admin"] if *making.borrow() => (Some(0), format!("{ADMIN}\n"), String::new()),
            [.., "--admin"] => (Some(4), String::new(), "no admin key".into()),
            [.., "--web"] => (Some(0), format!("{WEB}\n"), String::new()),
            _ => (Some(0), format!("{ACCESS}\n"), String::new()),
        })
    });
    let copy = block_on(service.add_admin_key(&*clock, &folder(), &key_copies(), "kkt-admin"));
    assert_eq!(
        copy,
        Ok(PathBuf::from("/home/me/.config/kkt-admin/docker-admin-key"))
    );
    for (path, key) in [
        ("/home/me/.config/kkt/docker-access-key", ACCESS),
        ("/home/me/.config/kkt/docker-web-key", WEB),
        ("/home/me/.config/kkt-admin/docker-admin-key", ADMIN),
    ] {
        let file = keys.file(Path::new(path)).expect("a copy");
        assert_eq!(file.text, format!("{key}\n"), "{path}");
        assert_eq!(file.mode, Some(kkt_keys::ACCESS_KEY_FILE_MODE), "{path}");
    }
    let log = service.projects.log();
    assert!(
        log.contains(&"tool compose restart kkt".to_string()),
        "{log:?}"
    );
    assert!(said.0.borrow().iter().all(|line| keyless(line)));
}

#[test]
fn no_admin_key_reaches_an_error_or_a_line_said_when_its_read_fails_after_printing_it() {
    for admin in [
        (Some(1), format!("{ADMIN}\n"), format!("partial {ADMIN}")),
        (None, format!("{ADMIN}\n"), String::new()),
        (Some(0), "  \n".to_string(), String::new()),
    ] {
        let (service, said, clock) = installed(
            Rc::new(kkt_keys::testing::Memory::new()),
            LOCAL_SETTINGS,
            move |request| {
                let Request::Exec { args, .. } = request else {
                    return Ok((Some(0), String::new(), String::new()));
                };
                Ok(match args.last().map(String::as_str) {
                    Some("--admin") => (admin.0, admin.1.clone(), admin.2.clone()),
                    Some("--web") => (Some(0), format!("{WEB}\n"), String::new()),
                    _ => (Some(0), format!("{ACCESS}\n"), String::new()),
                })
            },
        );
        let error = block_on(service.add_admin_key(&*clock, &folder(), &key_copies(), "kkt-admin"))
            .unwrap_err()
            .to_string();
        assert!(keyless(&error), "{error}");
        assert!(
            error.ends_with("(exit 1, could not read the admin key)")
                || error.ends_with("(ended by a signal)")
                || error.ends_with("(exit 0, could not read the admin key)"),
            "{error}"
        );
        assert!(said.0.borrow().iter().all(|line| keyless(line)));
    }
}

#[test]
fn an_upgrade_selects_the_version_and_keeps_every_other_setting() {
    let keys = Rc::new(kkt_keys::testing::Memory::new());
    let (service, said, clock) = installed(
        keys,
        "# mine\nKKT_PORT=8000\nKKT_VERSION=0.3.9\nEXTRA=1\n",
        with_backup(server(no_admin(), vec![])),
    );
    block_on(service.upgrade(&*clock, &folder(), "0.4.0", Some(&key_copies()))).unwrap();
    assert_eq!(
        service.projects.file_text(&at(".env")),
        Some((
            "# mine\nKKT_PORT=8000\nKKT_VERSION=0.4.0\nEXTRA=1\n".to_string(),
            true
        ))
    );
    let log = service.projects.log();
    let pull = log.iter().position(|l| l == "tool compose pull");
    let backup = log
        .iter()
        .position(|l| l.contains("kkt backup /backups"))
        .expect("a backup");
    assert!(pull.is_some_and(|pull| backup < pull), "{log:?}");
    assert!(said.0.borrow().iter().all(|line| keyless(line)));
}

#[test]
fn no_key_reaches_a_line_said_when_an_upgrade_cannot_save_the_key_copies() {
    // Each key's show fails after printing its key; the upgrade stands and
    // says how to save them, without any key.
    for failing in ["", "--web", "--admin"] {
        let (service, said, clock) = installed(
            Rc::new(kkt_keys::testing::Memory::new()),
            LOCAL_SETTINGS,
            move |request| {
                let Request::Exec { args, .. } = request else {
                    return Ok((Some(0), String::new(), String::new()));
                };
                let last = args.last().map(String::as_str).unwrap_or("");
                let which = match last {
                    "--web" | "--admin" => last,
                    "/backups" => {
                        return Ok((
                            Some(0),
                            "Backed up to /backups/kkt-backup-1\n".into(),
                            String::new(),
                        ));
                    }
                    _ => "",
                };
                let key = match which {
                    "--web" => WEB,
                    "--admin" => ADMIN,
                    _ => ACCESS,
                };
                Ok(if which == failing {
                    (Some(1), format!("{key}\n"), key.to_string())
                } else if which == "--admin" {
                    (Some(4), String::new(), "no admin key".into())
                } else {
                    (Some(0), format!("{key}\n"), String::new())
                })
            },
        );
        block_on(service.upgrade(&*clock, &folder(), "0.5.0", Some(&key_copies()))).unwrap();
        let said = said.0.borrow();
        let line = said
            .iter()
            .find(|l| l.starts_with("kkt is running, but its keys could not be saved"))
            .unwrap_or_else(|| panic!("{failing}: {said:?}"));
        assert!(
            line.contains("run `tool compose exec kkt kkt key show --web` in /data/docker"),
            "{line}"
        );
        assert!(said.iter().all(|l| keyless(l)), "{failing}: {said:?}");
    }
}

#[test]
fn set_version_replaces_or_adds_the_version_line_and_keeps_the_rest() {
    assert_eq!(
        crate::files::set_version("A=1\nKKT_VERSION=0.1\nB=2\n", "0.2"),
        "A=1\nKKT_VERSION=0.2\nB=2\n"
    );
    assert_eq!(
        crate::files::set_version("KKT_PORT=8000", "0.2"),
        "KKT_PORT=8000\nKKT_VERSION=0.2\n"
    );
    assert!(crate::files::is_shared(
        "KKT_VERSION=1\n COMPOSE_FILE=compose.tailscale.yaml \n"
    ));
    assert!(!crate::files::is_shared("KKT_VERSION=1\n"));
}

#[test]
fn a_tools_unexpected_answer_is_said_as_unexpected_output_from_its_command() {
    // No install path reaches this arm today (Docker answers `Unexpected`
    // only from `inspect`, which the name pack words itself), so it is
    // pinned here: the kind, and the text the old code gave.
    let error = crate::ContainerError::from(ProjectError::Unexpected {
        command: "tool compose ps".into(),
        detail: "no lines".into(),
    });
    assert!(matches!(error, crate::ContainerError::Unexpected { .. }));
    assert_eq!(
        error.to_string(),
        "unexpected output from `tool compose ps`: no lines"
    );
}
