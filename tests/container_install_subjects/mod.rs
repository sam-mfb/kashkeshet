//! The implementations of kkt-containerized-service's `ServiceContainer` that the container
//! install's pins run (`tests/container_install/main.rs`). Every scenario
//! runs once for each subject here, and each subject's transcript must
//! match the same file in `tests/container_install/`.
//!
//! This folder sits outside `tests/container_install/` on purpose. During
//! the container split a step could add a subject here, and change the
//! `Containerized` subject's construction to hand it the parts it takes
//! (msg 4045). The split's last step removed the Docker subject, with the
//! old code it ran (msg 4071); `Containerized` is the only one left. No
//! step changes the scenarios, what the scripted host answers, or their
//! files, which all live in that folder. The harness there may gain a
//! check that can only catch more, such as a further check on every
//! transcript; that is within the rule (msg 4060).

use kashkeshet::machine::sharing_node::TailscaleNode;
use kkt_container_docker::Docker;
use kkt_container_docker::testing::Scripted;
use kkt_containerized_service::ServiceContainer;
use kkt_containerized_service::{Browser, Containerized, Tell};
use kkt_keys::KeyFiles;
use std::io;
use std::path::Path;

/// The subjects, by name, in the order they run.
pub const SUBJECTS: &[&str] = &["Containerized"];

/// The subject named `name`, over the scripted host `host`, opening a
/// login link in `browser`.
pub fn subject(name: &str, browser: Box<dyn Browser>, host: Scripted) -> Box<dyn ServiceContainer> {
    match name {
        // kkt's containerized service over Docker as the general
        // `Projects` on the scripted host. It opens the login link itself,
        // and writes the key copies on the same host.
        "Containerized" => Box::new(Containerized::new(
            Docker::scripted(host.clone()),
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
