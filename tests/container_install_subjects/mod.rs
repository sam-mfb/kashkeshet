//! The implementations of kkt-container's `Container` that the container
//! install's pins run (`tests/container_install/main.rs`). Every scenario
//! runs once for each subject here, and each subject's transcript must
//! match the same file in `tests/container_install/`.
//!
//! This folder sits outside `tests/container_install/` on purpose. While the
//! container split is under way, a step may add a subject here. It may not
//! change the scenarios, what the scripted host answers, or their files,
//! which all live in that folder.

use kkt_container::Container;
use kkt_container_docker::testing::Scripted;
use kkt_container_docker::{Browser, Docker};

/// The subjects, by name, in the order they run.
pub const SUBJECTS: &[&str] = &["Docker"];

/// The subject named `name`, over the scripted host `host`, opening a
/// login link in `browser`.
pub fn subject(name: &str, browser: Box<dyn Browser>, host: Scripted) -> Box<dyn Container> {
    match name {
        "Docker" => Box::new(Docker::scripted(browser, host)),
        other => panic!("no subject {other}"),
    }
}
