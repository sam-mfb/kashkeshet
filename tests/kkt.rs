//! The `kkt` command: on its own it explains itself, and `kkt serve` starts
//! and serves the web page.

#[path = "common/home.rs"]
mod home;
#[path = "common/scratch.rs"]
mod scratch;
use home::OwnHome;

use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
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

#[tokio::test]
async fn kkt_serve_starts_and_serves() {
    let dir = scratch::Dir::new("kashkeshet-kkt");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();
    let bind = format!("127.0.0.1:{port}");
    let mut child = Command::new(KKT)
        .arg("serve")
        .arg("--db")
        .arg(dir.join("kkt.db"))
        .args(["--bind", &bind, "--no-access-key"])
        .own_home(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("start kkt");

    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(line) = lines.next_line().await.expect("read stdout") {
            if line.contains("listening") {
                return;
            }
        }
        panic!("kkt exited before listening");
    })
    .await
    .expect("kkt did not start in time");

    let page = reqwest::get(format!("http://{bind}/"))
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

    child.kill().await.expect("stop kkt");
}
