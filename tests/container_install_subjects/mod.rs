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
use std::io;

/// The subjects, by name, in the order they run.
pub const SUBJECTS: &[&str] = &["Docker", "Containerized"];

/// The subject named `name`, over the scripted host `host`, opening a
/// login link in `browser`.
pub fn subject(name: &str, browser: Box<dyn Browser>, host: Scripted) -> Box<dyn Container> {
    match name {
        "Docker" => Box::new(Docker::scripted(browser, host)),
        // kkt's containerized service over Docker as the general
        // `Projects`, with Docker on the same host doing the rest.
        "Containerized" => Box::new(Containerized::new(
            Docker::scripted(Box::new(NoBrowser), host.clone()),
            Docker::scripted(browser, host.clone()),
            Box::new(Transcript(host.clone())),
            Box::new(TailscaleNode),
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

/// The browser of the Docker that serves only as `Projects`, which opens
/// no login link.
struct NoBrowser;

impl Browser for NoBrowser {
    fn open(&self, _: &str) -> io::Result<()> {
        unreachable!("only the fallback opens a login link")
    }
    fn open_failure(&self) -> Option<String> {
        None
    }
}
