//! The exit codes of `kkt-webapp-preview`: wrong usage exits 2 (the
//! parser's convention), every other failure exits 1.

use std::process::{Command, Stdio};

const PROGRAM: &str = env!("CARGO_BIN_EXE_kkt-webapp-preview");

fn run(args: &[String]) -> (String, Option<i32>) {
    let out = Command::new(PROGRAM)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run kkt-webapp-preview");
    (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn wrong_usage_exits_2_and_other_failures_exit_1() {
    let (stderr, code) = run(&["--bogus".to_string()]);
    assert_eq!(code, Some(2), "{stderr}");
    // An address already in use is a failure, not wrong usage.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("hold a port");
    let address = listener.local_addr().expect("address").to_string();
    let (stderr, code) = run(&["--bind".to_string(), address]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(!stderr.is_empty(), "no message");
}
