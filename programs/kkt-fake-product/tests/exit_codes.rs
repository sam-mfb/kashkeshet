//! The exit codes of `kkt-fake-product`: wrong usage exits 2, every
//! other failure exits 1.

use std::process::{Command, Stdio};

const PROGRAM: &str = env!("CARGO_BIN_EXE_kkt-fake-product");

fn run(args: &[&std::ffi::OsStr]) -> (String, Option<i32>) {
    let out = Command::new(PROGRAM)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run kkt-fake-product");
    (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn wrong_usage_exits_2_and_other_failures_exit_1() {
    for args in [
        Vec::<&std::ffi::OsStr>::new(),
        vec!["a.db".as_ref(), "b.db".as_ref()],
    ] {
        let (stderr, code) = run(&args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains("usage:"), "{args:?}: {stderr}");
    }
    // A database file that already exists is a failure, not wrong usage.
    // The fixture is under the workspace's build directory: CI closes
    // the system temporary folder to tests, and this package adds no
    // dependency for a scratch folder of its own.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp")
        .join(format!("kkt-fake-product-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("build dir");
    let db = dir.join("taken.db");
    std::fs::write(&db, "").expect("existing file");
    let (stderr, code) = run(&[db.as_os_str()]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("exists; give a new file"), "{stderr}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A database path that is not valid UTF-8 keeps its native bytes: it
/// is used as a path (and cannot be made under /nope, here), never a
/// panic.
#[cfg(unix)]
#[test]
fn a_non_utf8_database_path_is_a_failure_not_a_panic() {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::OsStr::from_bytes(b"/nope/db-\xff.db");
    let (stderr, code) = run(&[path]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(!stderr.is_empty(), "no message");
}
