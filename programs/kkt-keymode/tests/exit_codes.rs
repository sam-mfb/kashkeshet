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
