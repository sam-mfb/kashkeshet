//! Where tests put their scratch files: a folder inside Cargo's `target/`
//! folder, never the system temporary folder, so a test run writes only
//! inside the clone.
//!
//! The integration tests in `tests/` and the unit tests in `src/` both use
//! this file, so both find the folder the same way: from where Cargo put the
//! test program, `<target>/<profile>/deps/`. Each test makes its own folder
//! inside it with `Dir`, which removes the folder when dropped: when the test
//! returns, or while a failed test unwinds. A test program that is killed
//! leaves its folders behind, and a removal that fails is printed, not
//! retried.

// Each test program builds this module on its own, and not every one uses
// every item.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

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

/// A test's own scratch folder, `<root>/<name>-<process ID>`, removed with
/// everything in it when this value is dropped.
///
/// Declare it before anything that uses the folder, such as a running `kkt`
/// process, so it is dropped after them: locals drop in reverse order.
pub struct Dir(PathBuf);

impl Dir {
    /// An empty folder named for `name` and this process, as an absolute
    /// path free of symlinks. Whatever an earlier run left at that path is
    /// removed first. `name` must be unique among the tests in one program.
    pub fn new(name: &str) -> Dir {
        let path = root().join(format!("{name}-{}", std::process::id()));
        remove(&path);
        std::fs::create_dir_all(&path).expect("create scratch folder");
        Dir(path.canonicalize().expect("canonical scratch folder"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for Dir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Dir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        remove(&self.0);
    }
}

/// Removes `path` and everything in it. A folder that is already gone is
/// fine; any other failure is reported on standard error rather than by
/// panicking, since this runs while a failed test unwinds.
fn remove(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("could not remove scratch folder {}: {e}", path.display()),
    }
}
