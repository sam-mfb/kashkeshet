//! Containerized over projects in memory: the managed folder's reads, the
//! service's start and stop, and what it hands to its fallback.

use crate::Containerized;
use kkt_container::testing::{Memory, MemoryProjects};
use kkt_container::{
    Container, Existing, FileAction, Folder, Output, Project, ProjectError, ProjectFile, Projects,
    Ran, ServiceConfig, User,
};
use std::io;
use std::path::{Path, PathBuf};

const FOLDER: &str = "/data/docker";

fn folder() -> Folder {
    Folder(PathBuf::from(FOLDER))
}

fn at(name: &str) -> PathBuf {
    Path::new(FOLDER).join(name)
}

fn service(projects: MemoryProjects) -> Containerized<MemoryProjects, Memory> {
    Containerized::new(projects, Memory::new(None, String::new()))
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
    let missing = Containerized::new(
        Failing(ProjectError::Missing {
            command: "tool compose up -d".into(),
            cause: "no such file".into(),
        }),
        Memory::new(None, String::new()),
    );
    assert_eq!(
        missing.up(&folder()),
        Err("Tool is not available: could not run `tool compose up -d`: no such file".into())
    );
    let failed = Containerized::new(
        Failing(ProjectError::Failed {
            command: "tool compose down".into(),
            status: Some(1),
            stdout: String::new(),
            stderr: "no network\n".into(),
        }),
        Memory::new(None, String::new()),
    );
    assert_eq!(
        failed.down(&folder(), false),
        Err("`tool compose down` failed (exit 1: no network)".into())
    );
    let file = Containerized::new(
        Failing(ProjectError::file(
            FileAction::Read,
            Path::new("/x"),
            &io::Error::other("gone"),
        )),
        Memory::new(None, String::new()),
    );
    assert_eq!(file.up(&folder()), Err("could not read /x: gone".into()));
}

// ----------------------------------------------------------------- fallback

#[test]
fn what_it_does_not_do_itself_goes_to_the_fallback() {
    let service = service(local(LOCAL_ENV));
    // The fallback has no install there, so it refuses; the refusal is the
    // fallback's own.
    assert_eq!(
        service.backup(&folder(), None),
        Err("container is not installed".into())
    );
    assert!(service.projects.log().is_empty());
    assert!(!service.fallback.calls().is_empty());
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
