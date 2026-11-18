//! Containerized over projects in memory: the managed folder's reads, the
//! service's start and stop, and what it hands to its fallback.

use crate::{Containerized, NodeLogin, SharingNode, Tell};
use futures::executor::block_on;
use kkt_clock::Time;
use kkt_clock::testing::FixedClock;
use kkt_container::testing::{Memory, MemoryProjects, Request};
use kkt_container::{
    Arg, Carry, ConfigReader, Container, ContainerConfig, Existing, FileAction, Folder, InServer,
    KeyCopy, Login, Output, Project, ProjectError, ProjectFile, Projects, Ran, ServiceConfig,
    State, User,
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

fn service(projects: MemoryProjects) -> Containerized<MemoryProjects, Memory> {
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
) -> (Containerized<MemoryProjects, Memory>, Said) {
    let said = Said::default();
    (over(projects, said.clone(), working_dir), said)
}

fn over<P: Projects>(
    projects: P,
    said: Said,
    working_dir: Result<PathBuf, String>,
) -> Containerized<P, Memory> {
    Containerized::new(
        projects,
        Memory::new(None, String::new()),
        Box::new(said),
        Box::new(Node),
        working_dir,
    )
}

fn failing(error: ProjectError) -> Containerized<Failing, Memory> {
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
        service(local("KKT_PORT=7420\n")).version(&folder()),
        Err("/data/docker/.env has no KKT_VERSION".into())
    );
    assert_eq!(
        service(MemoryProjects::new(&["kkt"])).version(&folder()),
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
        service.sharing_key(Path::new("/home/me/none")),
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
        assert_eq!(service(projects).existing(&folder()), Err(refusal.into()));
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
    fn available(&self) -> Result<(), ProjectError> {
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
        missing.up(&folder()),
        Err("Tool is not available: could not run `tool compose up -d`: no such file".into())
    );
    let failed = failing(ProjectError::Failed {
        command: "tool compose down".into(),
        status: Some(1),
        stdout: String::new(),
        stderr: "no network\n".into(),
    });
    assert_eq!(
        failed.down(&folder(), false),
        Err("`tool compose down` failed (exit 1: no network)".into())
    );
    let file = failing(ProjectError::file(
        FileAction::Read,
        Path::new("/x"),
        &io::Error::other("gone"),
    ));
    assert_eq!(file.up(&folder()), Err("could not read /x: gone".into()));
}

// ----------------------------------------------------------------- fallback

#[test]
fn what_it_does_not_do_itself_goes_to_the_fallback() {
    let service = service(local(LOCAL_ENV));
    let clock = FixedClock::new(Time::from_parts(2026, 11, 17, 12, 0, 0).unwrap());
    // The fallback has no install there, so it refuses; the refusal is the
    // fallback's own.
    assert!(block_on(service.upgrade(&clock, &folder(), "0.5.0", None)).is_err());
    assert!(service.projects.log().is_empty());
    assert_eq!(service.fallback.calls(), ["upgrade"]);
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
            service.run_in_server(&folder(), &command(Carry::In(vec![]))),
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
        service.run_in_server(&folder(), &command(Carry::In(vec![]))),
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
            service.config_home_file(&folder(), Path::new(refused)),
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
        crate::in_server::config_script(odd),
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
        lost.backup(&folder(), Some(Path::new("saved"))),
        Err("could not find the current folder: no such folder".into())
    );
    let odd = service(backing_up("Done.\n"));
    assert_eq!(
        odd.backup(&folder(), None),
        Err("unexpected output from `kkt backup`: Done.".into())
    );
    let nameless = service(backing_up("Backed up to /\n"));
    assert_eq!(
        nameless.backup(&folder(), None),
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
        with("web-key-2\n", Some(1)).compare_web_key(&folder(), key),
        Err("`tool compose exec kkt kkt key show --web` failed (exit 1, no key)".into())
    );
    assert_eq!(
        with("web-key-2\n", None).compare_web_key(&folder(), key),
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
        working_dir: "/data".into(),
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
    fn login(&self, answer: &str) -> Option<NodeLogin> {
        match answer {
            "" => None,
            "in" => Some(NodeLogin::LoggedIn),
            "out" => Some(NodeLogin::NeedsLogin),
            other => Some(NodeLogin::Other(other.to_string())),
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
