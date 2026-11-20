//! The exit codes of `kkt` itself: wrong usage exits 2 (the parser's
//! convention, clap's), every other failure exits 1. `kkt agent` with
//! no subcommand prints help and exits 2.

#[path = "common/home.rs"]
mod home;
use home::OwnHome;
use kkt_scratch as scratch;
use std::path::Path;
use std::process::{Command, Stdio};

const KKT: &str = env!("CARGO_BIN_EXE_kkt");

fn run(home: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(KKT)
        .args(args)
        .own_home(home)
        .stdin(Stdio::null())
        .output()
        .expect("run kkt");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn wrong_usage_exits_2_and_other_failures_exit_1() {
    let home = scratch::Dir::new("exit-codes");
    // Wrong usage: the parser refuses before anything runs.
    for args in [
        &["setup", "--mode", "bogus"][..],
        &["connect"][..],
        &["reclaim"][..],
        &["--bogus"][..],
    ] {
        let (stdout, stderr, code) = run(&home, args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?}: {stdout}");
        assert!(stderr.contains("error:"), "{args:?}: {stderr}");
    }
    // No subcommand for `agent`: help on standard error, exit 2.
    let (stdout, stderr, code) = run(&home, &["agent"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("Usage:"), "{stderr}");
    // A flag's value checked after the parser: also wrong usage, exit 2.
    for args in [
        &[
            "switch",
            "https://example.invalid/mcp",
            "--clients",
            "bogus",
        ][..],
        &["setup", "--clients", "bogus"][..],
    ] {
        let (stdout, stderr, code) = run(&home, args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?}: {stdout}");
        assert!(stderr.contains("unknown client"), "{args:?}: {stderr}");
    }
    // A plain failure: exit 1, one `error: ` line.
    let (stdout, stderr, code) = run(&home, &["key", "show", "--web"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error: "), "{stderr}");
}
