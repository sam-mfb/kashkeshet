//! The `kkt` command: on its own it explains itself, and `kkt serve` starts
//! and serves the web page.

#[path = "common/home.rs"]
mod home;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/serving.rs"]
mod serving;
use home::OwnHome;

use std::process::Stdio;
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");

#[tokio::test]
async fn version_names_the_app_and_matches_the_package_version() {
    let home = scratch::Dir::new("kkt-version");
    let out = Command::new(KKT)
        .arg("--version")
        .own_home(&home)
        .output()
        .await
        .expect("run kkt");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).expect("version text"),
        format!("kashkeshet {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[cfg(unix)]
#[test]
fn a_closed_standard_output_ends_a_command_quietly() {
    // As with `kkt status | head`: the reader has gone before kkt writes.
    use std::os::unix::process::ExitStatusExt;
    let home = scratch::Dir::new("kkt-closed-out");
    for args in [&["status"][..], &["--version"]] {
        let (reader, writer) = std::io::pipe().expect("pipe");
        drop(reader);
        let out = std::process::Command::new(KKT)
            .args(args)
            .own_home(&home)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .expect("run kkt");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("panicked"), "{args:?}: {stderr}");
        // Ended by SIGPIPE, as other Unix commands are, or exited cleanly.
        assert!(
            out.status.signal() == Some(13) || out.status.success(),
            "{args:?}: {:?} {stderr}",
            out.status
        );
    }
}

#[tokio::test]
async fn kkt_alone_prints_help() {
    let home = scratch::Dir::new("kkt-alone");
    let out = Command::new(KKT)
        .own_home(&home)
        .output()
        .await
        .expect("run kkt");
    assert!(!out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("Usage: kkt <COMMAND>"), "{text}");
    assert!(text.contains("kkt serve"), "{text}");
}

/// Asked for port 0, `kkt serve` listens on a port the system chooses and
/// says which in a line of its own, after the "listening" line it has
/// always printed.
#[tokio::test]
async fn kkt_serve_starts_and_serves() {
    let dir = scratch::Dir::new("kashkeshet-kkt");
    let mut command = Command::new(KKT);
    command
        .arg("serve")
        .arg("--db")
        .arg(dir.join("kkt.db"))
        .arg("--no-access-key")
        .own_home(&dir);
    let mut server = serving::start(command).await;

    let lines: Vec<&str> = server.log.lines().collect();
    let [.., listening, bound] = lines[..] else {
        panic!("too few lines: {}", server.log);
    };
    assert!(
        listening.contains("listening: web page http://127.0.0.1:0/ ,"),
        "{listening}"
    );
    assert!(
        bound.ends_with(&format!("bound to {}", server.bind())),
        "{bound}"
    );
    assert_ne!(server.addr.port(), 0);

    let page = reqwest::get(format!("{}/", server.base()))
        .await
        .expect("get /");
    assert!(page.status().is_success());
    assert!(
        page.text()
            .await
            .expect("body")
            .contains("Kashkeshet (kkt)")
    );
    assert!(dir.join("kkt.db").exists());

    server.child.kill().await.expect("stop kkt");
}

/// A server that must be given its port, and finds another kkt server
/// already on it, is started again on the next port, and the server already
/// there is not taken for it.
#[tokio::test]
async fn a_server_whose_port_is_taken_is_started_on_another() {
    let dir = scratch::Dir::new("kashkeshet-kkt-taken");
    let serve = |db: &str| {
        let mut command = Command::new(KKT);
        command
            .arg("serve")
            .arg("--db")
            .arg(dir.join(db))
            .arg("--no-access-key")
            .own_home(&dir);
        command
    };
    let mut other = serving::start(serve("other.db")).await;
    let free = serving::free_port();

    let (mut child, bind) =
        serving::try_start_quiet(|_| serve("kkt.db"), [other.addr.port(), free])
            .await
            .expect("kkt should start on the second port");
    assert_eq!(bind, format!("127.0.0.1:{free}"));

    child.kill().await.expect("stop kkt");
    other.child.kill().await.expect("stop the other kkt");
}

/// When every port tried is held by another process, the failure keeps each
/// server's exit status and what it printed, not just the ports.
#[tokio::test]
async fn a_server_that_runs_out_of_ports_says_why() {
    let dir = scratch::Dir::new("kashkeshet-kkt-no-ports");
    let serve = |db: &str| {
        let mut command = Command::new(KKT);
        command
            .arg("serve")
            .arg("--db")
            .arg(dir.join(db))
            .arg("--no-access-key")
            .own_home(&dir);
        command
    };
    let mut other = serving::start(serve("other.db")).await;

    let Err(failure) = serving::try_start_quiet(|_| serve("kkt.db"), [other.addr.port()]).await
    else {
        panic!("kkt should not have started");
    };
    assert!(
        failure.contains("could not listen on any port it tried"),
        "{failure}"
    );
    assert!(failure.contains("exit status: 1"), "{failure}");
    assert!(
        failure.contains(&format!("could not listen on {}:", other.bind())),
        "{failure}"
    );

    other.child.kill().await.expect("stop the other kkt");
}

/// A server that fails for a reason other than its port is reported as it
/// is, even when another process holds the port, and no other port is
/// tried.
#[tokio::test]
async fn a_failure_on_a_taken_port_is_not_taken_for_the_port() {
    let dir = scratch::Dir::new("kashkeshet-kkt-taken-fails");
    let mut other = {
        let mut command = Command::new(KKT);
        command
            .arg("serve")
            .arg("--db")
            .arg(dir.join("other.db"))
            .arg("--no-access-key")
            .own_home(&dir);
        serving::start(command).await
    };
    let bad = || {
        let mut command = Command::new(KKT);
        command
            .arg("serve")
            .arg("--db")
            .arg(dir.join("kkt.db"))
            .args(["--no-access-key", "--allow-host", "http://bad.example"])
            .own_home(&dir);
        command
    };
    let free = serving::free_port();

    let Err(failure) = serving::try_start_quiet(|_| bad(), [other.addr.port(), free]).await else {
        panic!("kkt should not have started");
    };
    assert!(failure.contains("kkt exited before listening"), "{failure}");
    assert!(failure.contains("invalid setting: allow_host"), "{failure}");
    assert!(failure.contains(&other.bind()), "{failure}");
    assert!(!failure.contains(&format!("127.0.0.1:{free}")), "{failure}");

    other.child.kill().await.expect("stop the other kkt");
}

/// A server that exits before listening fails the test with its exit status
/// and what it printed to standard error, so the failure says why.
#[tokio::test]
async fn a_server_that_exits_before_listening_says_why() {
    let dir = scratch::Dir::new("kashkeshet-kkt-exits");
    let mut command = Command::new(KKT);
    command
        .arg("serve")
        .arg("--db")
        .arg(dir.join("kkt.db"))
        .args(["--no-access-key", "--allow-host", "http://bad.example"])
        .own_home(&dir);
    let Err(failure) = serving::try_start(command).await else {
        panic!("kkt should not have started");
    };
    assert!(failure.contains("kkt exited before listening"), "{failure}");
    assert!(failure.contains("exit status: 1"), "{failure}");
    assert!(failure.contains("invalid setting: allow_host"), "{failure}");
}
