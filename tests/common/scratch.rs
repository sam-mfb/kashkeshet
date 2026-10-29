//! Where tests put their scratch files: a folder inside Cargo's `target/`
//! folder, never the system temporary folder, so a test run writes only
//! inside the clone.
//!
//! The integration tests in `tests/` and the unit tests in `src/` both use
//! this file, so both find the folder the same way: from where Cargo put the
//! test program, `<target>/<profile>/deps/`. Each test still names its own
//! folder inside it (with the process ID, and more when tests in one program
//! could collide) and removes it when done.

use std::path::PathBuf;

/// `<target>/tmp`, created if missing; the same folder Cargo names in
/// `CARGO_TARGET_TMPDIR` for integration tests.
pub fn root() -> PathBuf {
    let program = std::env::current_exe().expect("path of the test program");
    let target = program
        .ancestors()
        .nth(3)
        .expect("test program inside <target>/<profile>/deps");
    let root = target.join("tmp");
    std::fs::create_dir_all(&root).expect("scratch folder under target");
    root
}
