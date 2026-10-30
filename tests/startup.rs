//! Starting the binary with settings it must refuse.

#[path = "common/home.rs"]
mod home;
#[path = "common/scratch.rs"]
mod scratch;
use home::OwnHome;

use std::time::Duration;
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");

/// Long enough for a refusal; a server that wrongly starts is killed then.
const REFUSAL_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn a_bad_allowed_host_stops_the_server_naming_the_setting() {
    // Its own database and a free port, so a server that wrongly starts
    // touches nothing shared and is caught by the timeout.
    let dir = scratch::Dir::new("kashkeshet-startup");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();
    let run = Command::new(KKT)
        .arg("serve")
        .arg("--db")
        .arg(dir.join("startup.db"))
        .args(["--bind", &format!("127.0.0.1:{port}")])
        .args([
            "--allow-host",
            "http://host.docker.internal",
            "--no-access-key",
        ])
        .own_home(&dir)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(REFUSAL_TIMEOUT, run)
        .await
        .expect("the server should refuse to start, not run")
        .expect("run kkt");
    assert!(!out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("invalid setting: allow_host"), "{text}");
}

#[tokio::test]
async fn a_bad_or_reused_web_key_stops_the_server_without_printing_it() {
    let dir = scratch::Dir::new("kashkeshet-startup-web");
    let keys = dir.join(".config/kkt");
    std::fs::create_dir_all(&keys).unwrap();
    let access = "access-0123456789abcdef0123456789abcdef";
    std::fs::write(keys.join("access-key"), access).unwrap();
    for web in ["short-secret", access] {
        std::fs::write(keys.join("web-key"), web).unwrap();
        let run = Command::new(KKT)
            .args(["serve", "--bind", "127.0.0.1:0"])
            .env("HOME", dir.path())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(REFUSAL_TIMEOUT, run)
            .await
            .unwrap()
            .unwrap();
        assert!(!out.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(text.contains("invalid setting: web_key_file"), "{text}");
        assert!(!text.contains(web));
        assert!(!text.contains(access));
    }
}

#[tokio::test]
async fn a_bad_or_reused_admin_key_stops_the_server_without_printing_it() {
    let dir = scratch::Dir::new("kashkeshet-startup-admin");
    let keys = dir.join(".config/kkt");
    std::fs::create_dir_all(&keys).unwrap();
    let access = "access-0123456789abcdef0123456789abcdef";
    let web = "web-0123456789abcdef0123456789abcdef";
    std::fs::write(keys.join("access-key"), access).unwrap();
    std::fs::write(keys.join("web-key"), web).unwrap();
    for admin in ["short-secret", access, web] {
        std::fs::write(keys.join("admin-key"), admin).unwrap();
        let run = Command::new(KKT)
            .args(["serve", "--bind", "127.0.0.1:0"])
            .arg("--db")
            .arg(dir.join("startup.db"))
            .own_home(&dir)
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(REFUSAL_TIMEOUT, run)
            .await
            .unwrap()
            .unwrap();
        assert!(!out.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(text.contains("invalid setting: admin_key_file"), "{text}");
        for key in [admin, access, web] {
            assert!(!text.contains(key), "{text}");
        }
        assert!(!dir.join("startup.db").exists());
    }
}
