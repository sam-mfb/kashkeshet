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
    let dir = std::env::temp_dir().join(format!("kkt-fake-product-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = dir.join("taken.db");
    std::fs::write(&db, "").expect("existing file");
    let (stderr, code) = run(&[db.as_os_str()]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("exists; give a new file"), "{stderr}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
