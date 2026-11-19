//! `kkt service install --dry-run` with the real binary: what the service
//! would run, worked out from a temporary home, with nothing changed. Every
//! run here has only a fake `launchctl`, `systemctl` and `id` on its PATH,
//! answering as a machine with no kkt service, so no test asks the real
//! service manager anything: the answers are the same on every machine,
//! with or without a login session or an installed kkt. The other service
//! commands are tested against a fake system in each service manager's
//! wrapper (`crates/kkt-bare-service-launchd/src/tests.rs` and
//! `crates/kkt-bare-service-systemd/src/tests.rs`).
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "common/home.rs"]
mod home;
use home::OwnHome;
use kkt_scratch as scratch;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const TIMEOUT: Duration = Duration::from_secs(20);

/// An empty home, absolute and free of symlinks as the paths in the service
/// will be, removed when dropped.
fn temp_home(name: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-service-{name}"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn kkt(home: &Path, args: &[&str]) -> Output {
    let run = Command::new(KKT)
        .args(args)
        .own_home(home)
        .env("PATH", fake_service_manager(home))
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(TIMEOUT, run)
        .await
        .expect("kkt should finish")
        .expect("run kkt")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn service_file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents/com.sam-mfb.kashkeshet.plist")
    } else {
        home.join(".config/systemd/user/kashkeshet.service")
    }
}

/// What every test's config file starts with: an address no other program
/// has, since port 0 lets the system pick one. `install` checks that the
/// server can listen on its address, even on a dry run, so the
/// default address would fail these tests on a machine already running kkt.
const FREE_BIND: &str = "bind = \"127.0.0.1:0\"\n";

/// Writes the default config file under `home`: [`FREE_BIND`], then `text`.
fn write_config(home: &Path, text: &str) -> PathBuf {
    let config_dir = home.join(".config/kkt");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config = config_dir.join("config.toml");
    std::fs::write(&config, format!("{FREE_BIND}{text}")).expect("config");
    config
}

fn default_db(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/kashkeshet/kashkeshet.db")
    } else {
        home.join(".local/share/kashkeshet/kashkeshet.db")
    }
}

#[tokio::test]
async fn a_dry_run_shows_absolute_paths_and_changes_nothing() {
    let home = temp_home("dry");
    let config_dir = home.join(".config/kkt");
    write_config(&home, "allow_hosts = [\"host.docker.internal\"]\n");

    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(said.starts_with("Dry run: nothing was changed."), "{said}");
    assert!(
        said.contains(&format!("Would write {}:", service_file(&home).display())),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "Would create a new web key at {}",
            config_dir.join("web-key").display()
        )),
        "{said}"
    );
    assert!(!config_dir.join("web-key").exists());
    let exe = Path::new(KKT).canonicalize().expect("kkt path");
    for path in [exe, config_dir.join("config.toml"), default_db(&home)] {
        assert!(
            said.contains(&path.display().to_string()),
            "{} missing from: {said}",
            path.display()
        );
    }
    // The service is the server setup meant, so `kkt serve` lets it start.
    assert!(said.contains("--ignore-setup"), "{said}");
    // With no key set, the default one is made at install and passed.
    let key = config_dir.join("access-key");
    assert!(
        said.contains(&format!(
            "Would create a new access key at {}",
            key.display()
        )),
        "{said}"
    );
    assert!(
        said.contains(&format!("\"--access-key-file\" \"{}\"", key.display()))
            || said.contains(&format!("<string>{}</string>", key.display())),
        "{said}"
    );
    // Nothing was written: not the service file, not the database's folder,
    // not the key.
    assert!(!key.exists());
    assert!(!service_file(&home).exists());
    assert!(!default_db(&home).parent().expect("db dir").exists());
    // On macOS the dry run asks launchd whether the service is loaded: the
    // fake answered, so the real launchd, and any kkt installed on this
    // machine, played no part.
    if cfg!(target_os = "macos") {
        let calls = std::fs::read_to_string(home.join("service-calls.log")).unwrap_or_default();
        assert!(
            calls.contains("launchctl print gui/501/com.sam-mfb.kashkeshet"),
            "{calls}"
        );
        assert!(calls.contains("id -u"), "{calls}");
    }
}

#[tokio::test]
async fn a_database_named_in_the_config_file_is_left_to_it() {
    let home = temp_home("configdb");
    let config_dir = home.join(".config/kkt");
    write_config(&home, "db = \"nested/chat.db\"\n");

    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    // The service reads the database from the config file each time it
    // starts, so it isn't passed and the default isn't used.
    assert!(!said.contains("--db"), "{said}");
    assert!(
        !said.contains(&default_db(&home).display().to_string()),
        "{said}"
    );
    // serve won't make a configured database's folder, so install does.
    assert!(
        said.contains(&format!(
            "Would create {}",
            config_dir.join("nested").display()
        )),
        "{said}"
    );
}

#[tokio::test]
async fn relative_flags_become_absolute() {
    let home = temp_home("relative");
    write_config(&home, "");
    let out = Command::new(KKT)
        .args(["service", "install", "--dry-run", "--db", "data/chat.db"])
        .current_dir(&home)
        .own_home(&home)
        .env("PATH", fake_service_manager(&home))
        .output()
        .await
        .expect("run kkt");
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains(&home.join("data/chat.db").display().to_string()),
        "{said}"
    );
}

#[tokio::test]
async fn bad_settings_fail_at_install_not_in_a_restart_loop() {
    let home = temp_home("bad");
    write_config(&home, "default_limit = 0\n");
    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    let said = text(&out);
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("invalid setting: default_limit"), "{said}");

    write_config(&home, "");
    let missing = home.join("no-such-key");
    let out = kkt(
        &home,
        &[
            "service",
            "install",
            "--dry-run",
            "--access-key-file",
            missing.to_str().expect("utf-8"),
        ],
    )
    .await;
    let said = text(&out);
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("access_key_file"), "{said}");
    assert!(!service_file(&home).exists());
}

#[tokio::test]
async fn log_output_has_no_color_codes_when_not_a_terminal() {
    // A service's log is a file or the journal, not a terminal; color codes
    // would show up there as raw escape characters.
    let home = temp_home("ansi");
    let out = kkt(&home, &["serve", "--default-limit", "0"]).await;
    let said = text(&out);
    assert!(said.contains("invalid setting"), "{said}");
    assert!(!said.contains('\u{1b}'), "{said:?}");
}

#[tokio::test]
async fn an_existing_key_is_used_and_a_configured_one_left_to_the_config() {
    let home = temp_home("keys");
    let out = kkt(&home, &["key", "create"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let key = home.join(".config/kkt/access-key");
    write_config(&home, "");

    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(!said.contains("Would create a new access key"), "{said}");
    assert!(said.contains(&key.display().to_string()), "{said}");

    // A key file named in the config is read from it at each start.
    write_config(&home, "access_key_file = \"access-key\"\n");
    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(!said.contains("--access-key-file"), "{said}");
}

#[tokio::test]
async fn no_access_key_installs_without_a_key() {
    let home = temp_home("no-key");
    let key = home.join(".config/kkt/access-key");
    write_config(&home, "");

    // As a flag: passed on to serve, and no key made.
    let out = kkt(
        &home,
        &["service", "install", "--dry-run", "--no-access-key"],
    )
    .await;
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(!said.contains("Would create a new access key"), "{said}");
    assert!(!said.contains("--access-key-file"), "{said}");
    assert!(said.contains("--no-access-key"), "{said}");
    assert!(!key.exists());

    // From the config file: left to the config, which serve reads at start.
    write_config(&home, "no_access_key = true\n");
    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(!said.contains("Would create a new access key"), "{said}");
    assert!(!said.contains("--access-key-file"), "{said}");
    assert!(!said.contains("--no-access-key"), "{said}");

    // With a key file as well, install is refused, as serve would be.
    let out = kkt(
        &home,
        &[
            "service",
            "install",
            "--dry-run",
            "--access-key-file",
            "/keys/k",
        ],
    )
    .await;
    assert!(!out.status.success());
    assert!(
        text(&out).contains("invalid setting: no_access_key cannot be set together"),
        "{}",
        text(&out)
    );
    assert!(!key.exists());
}

#[tokio::test]
async fn a_bad_existing_web_key_is_checked_even_when_the_access_key_is_new() {
    let home = temp_home("bad-web");
    let keys = home.join(".config/kkt");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::write(keys.join("web-key"), "too-short").unwrap();
    let out = kkt(&home, &["service", "install", "--dry-run"]).await;
    assert!(!out.status.success());
    assert!(text(&out).contains("web_key_file"));
    assert!(!keys.join("access-key").exists());
}

/// The fake service manager of [`service_manager`] in `home`, logging to
/// `service-calls.log` there: the PATH a test's `kkt` runs with.
fn fake_service_manager(home: &Path) -> PathBuf {
    service_manager(home, &home.join("service-calls.log"))
}

/// A `bin` folder in `scratch` with a fake launchd and systemd that log
/// each call to `calls` and answer as a machine with no kkt service:
/// `launchctl print` finds no service (exit 113, as launchd says it),
/// `systemctl is-active` says inactive (exit 3), `id -u` gives a user ID,
/// and every other call succeeds. So no test asks the real service
/// manager anything, or starts a real service.
fn service_manager(scratch: &Path, calls: &Path) -> PathBuf {
    fake_bin(
        &scratch.join("bin"),
        calls,
        "if [ \"$1\" = print ]; then echo \"Could not find service \\\"$2\\\"\" >&2; exit 113; fi",
    )
}

/// A fake service manager in `bin` as [`service_manager`] makes it, but
/// whose `launchctl` answers each question as it does for a user with no
/// desktop login session: `launchctl print` exits 125.
#[cfg(target_os = "macos")]
fn service_manager_without_a_login_session(scratch: &Path, calls: &Path) -> PathBuf {
    fake_bin(
        &scratch.join("bin-no-session"),
        calls,
        "if [ \"$1\" = print ]; then echo 'Could not print domain: 125: Domain does not support specified action' >&2; exit 125; fi",
    )
}

/// The fake `launchctl` (answering as `launchctl` says), `systemctl` and
/// `id` in `bin`, each logging its calls to `calls`.
fn fake_bin(bin: &Path, calls: &Path, launchctl: &str) -> PathBuf {
    std::fs::create_dir_all(bin).expect("bin");
    let answers = [
        ("launchctl", launchctl),
        (
            "systemctl",
            "case \" $* \" in *\" is-active \"*) echo inactive; exit 3;; esac",
        ),
        ("id", "if [ \"$1\" = -u ]; then echo 501; fi"),
    ];
    for (name, answer) in answers {
        let path = bin.join(name);
        let log = calls.display();
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"{name} $*\" >> '{log}'\n{answer}\nexit 0\n"),
        )
        .expect("script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
    }
    bin.to_path_buf()
}

/// On macOS, for a user not logged in at the desktop, `kkt service
/// install` says so in plain words, with or without --dry-run.
#[cfg(target_os = "macos")]
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn without_a_login_session_install_says_so_plainly() {
    let home = temp_home("no-session");
    write_config(&home, "no_access_key = true\n");
    let bin = service_manager_without_a_login_session(&home, &home.join("calls.log"));
    for args in [
        &["service", "install", "--dry-run"][..],
        &["service", "install"][..],
    ] {
        let run = Command::new(KKT)
            .args(args)
            .own_home(&home)
            .env("PATH", &bin)
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(TIMEOUT, run)
            .await
            .expect("kkt should finish")
            .expect("run kkt");
        let said = text(&out);
        assert!(!out.status.success(), "{args:?}: {said}");
        assert!(
            said.contains("logged in at this Mac's desktop"),
            "{args:?}: {said}"
        );
        assert!(said.contains("`kkt serve`"), "{args:?}: {said}");
        assert!(!said.contains("Could not print domain"), "{args:?}: {said}");
        assert!(
            !service_file(&home).exists(),
            "{args:?}: no service file is left"
        );
    }
}

/// With another program on the server's address, `kkt service
/// install` fails, with or without --dry-run, and changes nothing: no
/// service file, no key, no call to the service manager.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn an_address_in_use_fails_the_install_and_changes_nothing() {
    let home = temp_home("in-use");
    let other = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
    let bind = other.local_addr().expect("address");
    let config = home.join(".config/kkt/config.toml");
    std::fs::create_dir_all(config.parent().expect("folder")).expect("config dir");
    std::fs::write(&config, format!("bind = \"{bind}\"\n")).expect("config");
    let calls = home.join("calls.log");
    let bin = service_manager(&home, &calls);

    for args in [
        &["service", "install", "--dry-run"][..],
        &["service", "install"][..],
    ] {
        let run = Command::new(KKT)
            .args(args)
            .own_home(&home)
            .env("PATH", &bin)
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(TIMEOUT, run)
            .await
            .expect("kkt should finish")
            .expect("run kkt");
        let said = text(&out);
        assert!(!out.status.success(), "{args:?}: {said}");
        assert!(
            said.contains(&format!(
                "{bind} is already in use, so the server could not listen there."
            )),
            "{args:?}: {said}"
        );
        assert!(
            said.contains(&config.display().to_string()),
            "{args:?}: {said}"
        );
        assert!(!service_file(&home).exists(), "{args:?}");
        assert!(!home.join(".config/kkt/access-key").exists(), "{args:?}");
        assert!(!home.join(".config/kkt/web-key").exists(), "{args:?}");
        assert!(!calls.exists(), "{args:?}: the service manager was called");
    }
}
