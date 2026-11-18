//! The implementations of kkt-container's `Container` that the container
//! install's pins run (`tests/container_install/main.rs`). Every scenario
//! runs once for each subject here, and each subject's transcript must
//! match the same file in `tests/container_install/`.
//!
//! This folder sits outside `tests/container_install/` on purpose. While the
//! container split is under way, a step may add a subject here. Until its
//! last step, no step removes or alters a subject. No step changes the
//! scenarios, what the scripted host answers, or their files, which all
//! live in that folder.

use kashkeshet::machine::sharing_node::TailscaleNode;
use kkt_container::Container;
use kkt_container_docker::testing::Scripted;
use kkt_container_docker::{Browser, Docker};
use kkt_containerized_service::{Containerized, Tell};
use kkt_keys::KeyFiles;
use std::io;
use std::path::Path;

/// The subjects, by name, in the order they run.
pub const SUBJECTS: &[&str] = &["Docker", "Containerized"];

/// The subject named `name`, over the scripted host `host`, opening a
/// login link in `browser`.
pub fn subject(name: &str, browser: Box<dyn Browser>, host: Scripted) -> Box<dyn Container> {
    match name {
        "Docker" => Box::new(Docker::scripted(browser, host)),
        // kkt's containerized service over Docker as the general
        // `Projects`, with Docker on the same host doing the rest. It opens
        // the login link itself, and writes the key copies on the same host.
        "Containerized" => Box::new(Containerized::new(
            Docker::scripted(Box::new(NoBrowser), host.clone()),
            Docker::scripted(Box::new(NoBrowser), host.clone()),
            Box::new(Transcript(host.clone())),
            Box::new(TailscaleNode),
            browser,
            Box::new(KeyCopiesOn(host.clone())),
            host.current_folder().map_err(|e| e.to_string()),
        )),
        other => panic!("no subject {other}"),
    }
}

/// The containerized service's lines, written into the transcript as
/// Docker's host writes its own.
struct Transcript(Scripted);

impl Tell for Transcript {
    fn say(&self, line: &str) {
        self.0.note(&format!("{:<8} {line}", "say"));
    }
}

/// The browser of the Dockers inside `Containerized`, which open no login
/// link: `Containerized` opens it.
struct NoBrowser;

impl Browser for NoBrowser {
    fn open(&self, _: &str) -> io::Result<()> {
        unreachable!("only Containerized opens a login link")
    }
    fn open_failure(&self) -> Option<String> {
        None
    }
}

/// The key copies on the scripted host, written as kkt-keys-os writes them
/// (a private file, the key and a line's end, refused over a file there
/// unless forced, worded as it words them), into the same transcript as
/// Docker's own writes.
struct KeyCopiesOn(Scripted);

impl KeyFiles for KeyCopiesOn {
    fn create_dir_all(&self, dir: &Path) -> Result<(), String> {
        self.0
            .make_folder(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))
    }
    fn write_key(&self, path: &Path, key: &str, force: bool) -> Result<(), String> {
        if !force && self.0.has(path) {
            return Err(format!(
                "{} already exists; pass --force to replace it",
                path.display()
            ));
        }
        self.0
            .write(path, &format!("{key}\n"), true)
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    }
    fn read(&self, _: &Path) -> io::Result<String> {
        unreachable!("setup only writes key copies")
    }
    fn exists(&self, path: &Path) -> bool {
        self.0.has(path)
    }
    fn try_exists(&self, path: &Path) -> io::Result<bool> {
        Ok(self.0.has(path))
    }
    fn is_file(&self, _: &Path) -> bool {
        unreachable!("setup only writes key copies")
    }
    fn mode(&self, _: &Path) -> io::Result<Option<u32>> {
        unreachable!("setup only writes key copies")
    }
    fn keep_staged(&self, _: &Path, _: &Path) -> Result<(), String> {
        unreachable!("setup only writes key copies")
    }
    fn remove(&self, _: &Path) -> io::Result<()> {
        unreachable!("setup only writes key copies")
    }
    fn write_admin_name(&self, _: &Path, _: Option<&str>) -> io::Result<()> {
        unreachable!("setup only writes key copies")
    }
}
