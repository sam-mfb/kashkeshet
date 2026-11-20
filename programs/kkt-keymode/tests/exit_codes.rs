//! The exit codes of `kkt-keymode`: wrong usage exits 2, every other
//! failure exits 1.

use std::process::{Command, Stdio};

const PROGRAM: &str = env!("CARGO_BIN_EXE_kkt-keymode");

fn run(args: &[&str]) -> (String, Option<i32>) {
    let out = Command::new(PROGRAM)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run kkt-keymode");
    (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn run_os(args: &[&std::ffi::OsStr]) -> (String, Option<i32>) {
    let out = Command::new(PROGRAM)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run kkt-keymode");
    (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn wrong_usage_exits_2_and_other_failures_exit_1() {
    for args in [&[][..], &["--bogus", "x"][..], &["--bind", "nope"][..]] {
        let (stderr, code) = run(args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(
            stderr.contains("usage:") || stderr.contains("--bind"),
            "{args:?}: {stderr}"
        );
    }
    // A config file that is not there is a failure, not wrong usage.
    let (stderr, code) = run(&["--config", "/nope/kkt-keymode.toml"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(!stderr.is_empty(), "no message");
}

/// A config path that is not valid UTF-8 keeps its native bytes: it is
/// read as a path (and not found, here), never a panic.
#[cfg(unix)]
#[test]
fn a_non_utf8_config_path_is_a_failure_not_a_panic() {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::OsStr::from_bytes(b"/nope/cfg-\xff.toml");
    let (stderr, code) = run_os(&["--config".as_ref(), path]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(!stderr.is_empty(), "no message");
}
