//! `kkt serve` on a machine `kkt setup` set up: run with the real binary in
//! its own home directory, with a setup record written by hand.

#[path = "common/home.rs"]
mod home;
#[path = "common/scratch.rs"]
mod scratch;
use home::OwnHome;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const TIMEOUT: Duration = Duration::from_secs(20);

/// A home with an empty kkt config folder, removed when dropped.
fn temp_home(name: &str) -> scratch::Dir {
    let dir = empty_home(name);
    std::fs::create_dir_all(dir.join(".config/kkt")).expect("temp home");
    dir
}

fn free_bind() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();
    format!("127.0.0.1:{port}")
}

fn write_record(home: &Path, text: &str) {
    std::fs::write(home.join(".config/kkt/setup.toml"), text).expect("record");
}

/// `kkt serve` with its own database and port, plus `extra`.
fn serve(home: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(KKT);
    command
        .arg("serve")
        .args(extra)
        .arg("--db")
        .arg(home.join("kkt.db"))
        .args(["--bind", &free_bind(), "--no-access-key"])
        .own_home(home)
        .kill_on_drop(true);
    command
}

/// Runs `command`, which should refuse to start; returns what it printed.
async fn refused(mut command: Command) -> String {
    let out = tokio::time::timeout(TIMEOUT, command.output())
        .await
        .expect("kkt should refuse, not run")
        .expect("run kkt");
    assert!(!out.status.success());
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Starts `command` and stops it once it is listening.
async fn starts(mut command: Command) {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start kkt");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    tokio::time::timeout(TIMEOUT, async {
        while let Some(line) = lines.next_line().await.expect("read stdout") {
            if line.contains("listening") {
                return;
            }
        }
        panic!("kkt exited before listening");
    })
    .await
    .expect("kkt did not start in time");
    child.kill().await.expect("stop kkt");
}

const CLIENT: &str = "format = 1\nmode = \"client\"\nwritten_by = \"0.4.0\"\n\
                      url = \"https://kkt.tail1234.ts.net/mcp\"\n";

#[tokio::test]
async fn a_client_machine_refuses_to_serve_unless_told_to_ignore_setup() {
    let home = temp_home("client");
    write_record(&home, CLIENT);
    let said = refused(serve(&home, &[])).await;
    assert!(
        said.contains("client of the kkt server at https://kkt.tail1234.ts.net/mcp"),
        "{said}"
    );
    assert!(
        !home.join("kkt.db").exists(),
        "refused before opening anything"
    );
    starts(serve(&home, &["--ignore-setup"])).await;
}

#[tokio::test]
async fn a_docker_machine_refuses_to_serve_naming_its_folder() {
    let home = temp_home("docker");
    write_record(
        &home,
        "format = 1\nmode = \"docker\"\nwritten_by = \"0.4.0\"\n\
         url = \"http://127.0.0.1:7420/mcp\"\n\n[docker]\n\
         folder = \"/srv/kkt-docker\"\ntailscale = false\n",
    );
    let said = refused(serve(&home, &[])).await;
    assert!(said.contains("Docker, from /srv/kkt-docker"), "{said}");
}

#[tokio::test]
async fn a_broken_record_is_named_and_can_be_ignored() {
    let home = temp_home("broken");
    write_record(&home, "mode = ");
    let said = refused(serve(&home, &[])).await;
    assert!(said.contains("setup.toml"), "{said}");
    assert!(said.contains("is not valid"), "{said}");
    starts(serve(&home, &["--ignore-setup"])).await;
}

#[tokio::test]
async fn with_no_record_serve_runs_as_before() {
    let home = temp_home("none");
    starts(serve(&home, &[])).await;
}

/// `kkt` with `args`, `home` as its only home directory, no terminal, and
/// an empty PATH so no real client is found.
async fn kkt(home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(KKT)
        .args(args)
        .own_home(home)
        .env("PATH", "")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out)
        .await
        .expect("kkt should finish")
        .expect("run kkt");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// A server with an access key, in its own home; returns the home, the
/// server, its MCP URL and its key file. The home comes first so that, bound
/// in that order, it is dropped (and removed) after the server.
async fn keyed_server(name: &str) -> (scratch::Dir, tokio::process::Child, String, PathBuf) {
    let (home, key) = server_key(name).await;
    let bind = free_bind();
    let child = serve_with_key(&home, &key, &bind).await;
    (home, child, format!("http://{bind}/mcp"), key)
}

/// A home for a server, and a new access key in it.
async fn server_key(name: &str) -> (scratch::Dir, PathBuf) {
    let home = temp_home(name);
    let key = home.join("server-key");
    let (made, said) = kkt(
        &home,
        &["key", "create", "--path", key.to_str().expect("utf-8")],
    )
    .await;
    assert!(made, "{said}");
    (home, key)
}

/// Starts `kkt serve` with `key` on `bind`, returning once it listens.
async fn serve_with_key(home: &Path, key: &Path, bind: &str) -> tokio::process::Child {
    let mut command = Command::new(KKT);
    command
        .args(["serve", "--bind", bind, "--access-key-file"])
        .arg(key)
        .arg("--db")
        .arg(home.join("kkt.db"))
        .own_home(home)
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().expect("start kkt");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    tokio::time::timeout(TIMEOUT, async {
        while let Some(line) = lines.next_line().await.expect("read stdout") {
            if line.contains("listening") {
                return;
            }
        }
        panic!("kkt exited before listening");
    })
    .await
    .expect("kkt did not start in time");
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    child
}

#[tokio::test]
async fn a_client_machine_is_set_up_from_flags_alone() {
    let (_server_home, mut server, url, key) = keyed_server("client-server").await;
    let home = temp_home("client-setup");
    let key_arg = key.to_str().expect("utf-8");
    let (ok, said) = kkt(
        &home,
        &[
            "setup",
            "--mode",
            "client",
            "--url",
            &url,
            "--key-file",
            key_arg,
            "--clients",
            "codex",
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(said.contains("set up as client"), "{said}");
    assert!(said.contains("ok: key accepted"), "doctor ran: {said}");

    let record = std::fs::read_to_string(home.join(".config/kkt/setup.toml")).expect("record");
    assert!(record.contains("mode = \"client\""), "{record}");
    assert!(record.contains(&format!("url = \"{url}\"")), "{record}");
    let codex = std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config");
    assert!(codex.contains(&url), "{codex}");

    // Doctor, with nothing given, checks the recorded server with the
    // recorded key. (Its last line, about this machine's service, depends on
    // the machine, so only the connection checks are asserted.)
    let (_, said) = kkt(&home, &["doctor"]).await;
    assert!(
        said.contains(&format!("ok: server: reachable at {url}")),
        "{said}"
    );
    assert!(said.contains("ok: key accepted"), "{said}");
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn with_no_terminal_a_missing_answer_names_its_flag() {
    let home = temp_home("no-terminal");
    let (ok, said) = kkt(&home, &["setup"]).await;
    assert!(!ok);
    assert!(said.contains("pass --mode"), "{said}");
    let (ok, said) = kkt(&home, &["setup", "--mode", "client"]).await;
    assert!(!ok);
    assert!(said.contains("pass --url"), "{said}");
    for mode in ["docker", "service"] {
        let (ok, said) = kkt(&home, &["setup", "--mode", mode, "--yes"]).await;
        assert!(!ok);
        assert!(
            said.contains("pass --tailscale or --no-tailscale"),
            "{said}"
        );
        let (ok, said) = kkt(&home, &["setup", "--mode", mode, "--no-tailscale", "--yes"]).await;
        assert!(!ok);
        assert!(
            said.contains("pass --allow-host host.docker.internal, or --allow-host none"),
            "{said}"
        );
    }
    assert!(!home.join(".config/kkt/setup.toml").exists());
}

/// The record of an earlier setup, which a failed setup must leave alone.
const EARLIER: &str = "format = 1\nmode = \"client\"\nwritten_by = \"0.4.0\"\n\
                       url = \"https://old.example/mcp\"\n";

fn record_text(home: &Path) -> String {
    std::fs::read_to_string(home.join(".config/kkt/setup.toml")).expect("record")
}

/// `kkt setup` for a client of `url` with the key in `key`, plus `extra`.
async fn setup_client(home: &Path, url: &str, key: &Path, extra: &[&str]) -> (bool, String) {
    let key = key.to_str().expect("utf-8");
    let mut args = vec!["setup", "--mode", "client", "--url", url, "--key-file", key];
    args.extend(extra);
    kkt(home, &args).await
}

/// The key is checked as soon as it is given, so setup stops before any
/// other step, naming the flag.
#[tokio::test]
async fn a_rejected_key_leaves_the_earlier_setup() {
    let (_server_home, mut server, url, _) = keyed_server("wrong-key-server").await;
    let home = temp_home("wrong-key");
    write_record(&home, EARLIER);
    let wrong = home.join("wrong-key");
    let (made, said) = kkt(
        &home,
        &["key", "create", "--path", wrong.to_str().expect("utf-8")],
    )
    .await;
    assert!(made, "{said}");
    let (ok, said) = setup_client(&home, &url, &wrong, &["--clients", "none"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("--key-file"), "{said}");
    assert!(
        said.contains("problem: key accepted: server returned 401"),
        "{said}"
    );
    assert!(
        !said.contains("ok: "),
        "only the problems are shown: {said}"
    );
    assert_eq!(record_text(&home), EARLIER);
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn an_unreachable_server_leaves_the_earlier_setup() {
    let home = temp_home("unreachable");
    write_record(&home, EARLIER);
    let key = home.join("key");
    let (made, said) = kkt(
        &home,
        &["key", "create", "--path", key.to_str().expect("utf-8")],
    )
    .await;
    assert!(made, "{said}");
    // Nothing listens on a port just freed.
    let url = format!("http://{}/mcp", free_bind());
    let (ok, said) = setup_client(&home, &url, &key, &["--clients", "codex"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("--url"), "{said}");
    assert!(said.contains("problem: server: "), "{said}");
    assert!(
        said.contains("check the address, and that the server is running on its machine"),
        "{said}"
    );
    assert_eq!(record_text(&home), EARLIER);
    assert!(
        !home.join(".codex/config.toml").exists(),
        "no client was touched"
    );
}

/// A server that answers, but at another path, fails the address check,
/// so the problem is put on `--url`, not on the key.
#[tokio::test]
async fn a_wrong_path_on_a_running_server_is_put_on_the_url() {
    let (_server_home, mut server, url, key) = keyed_server("wrong-path-server").await;
    let home = temp_home("wrong-path");
    write_record(&home, EARLIER);
    let wrong = format!("{url}x");
    let (ok, said) = setup_client(&home, &wrong, &key, &["--clients", "none"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("given with --url"), "{said}");
    assert!(!said.contains("given with --key-file"), "{said}");
    assert!(said.contains("problem: MCP: "), "{said}");
    assert_eq!(record_text(&home), EARLIER);
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn a_missing_answer_leaves_the_earlier_setup() {
    let (_server_home, mut server, url, key) = keyed_server("missing-answer-server").await;
    let home = temp_home("missing-answer");
    write_record(&home, EARLIER);
    // A Codex config, so Codex is found and setup would ask about it.
    std::fs::create_dir_all(home.join(".codex")).expect("codex dir");
    std::fs::write(home.join(".codex/config.toml"), "# mine\n").expect("codex config");
    let (ok, said) = setup_client(&home, &url, &key, &[]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("pass --clients"), "{said}");
    assert_eq!(record_text(&home), EARLIER);
    assert_eq!(
        std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex"),
        "# mine\n"
    );
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn a_client_that_cannot_be_configured_leaves_the_earlier_setup() {
    let (_server_home, mut server, url, key) = keyed_server("bad-client-server").await;
    let home = temp_home("bad-client");
    write_record(&home, EARLIER);
    // A folder where Codex's config file should be, so it can't be written.
    std::fs::create_dir_all(home.join(".codex/config.toml")).expect("in the way");
    let (ok, said) = setup_client(&home, &url, &key, &["--clients", "codex"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("could not connect Codex"), "{said}");
    assert_eq!(record_text(&home), EARLIER);
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn url_and_key_file_each_replace_only_their_part_of_the_record() {
    let (_server_home, mut server, url, key) = keyed_server("override-server").await;
    let home = temp_home("override");
    let unreachable = format!("http://{}/mcp", free_bind());
    write_record(
        &home,
        &format!(
            "format = 1\nmode = \"client\"\nwritten_by = \"0.4.0\"\nurl = \"{unreachable}\"\n\
             key_file = \"{}\"\n",
            key.display()
        ),
    );
    // --url alone: the given server, with the recorded key.
    let (_, said) = kkt(&home, &["doctor", "--url", &url]).await;
    assert!(
        said.contains(&format!("ok: server: reachable at {url}")),
        "{said}"
    );
    assert!(said.contains("ok: key accepted"), "{said}");
    // --key-file alone: the recorded server, with the given key.
    let other = home.join("other-key");
    let (made, made_said) = kkt(
        &home,
        &["key", "create", "--path", other.to_str().expect("utf-8")],
    )
    .await;
    assert!(made, "{made_said}");
    let (_, said) = kkt(
        &home,
        &["doctor", "--key-file", other.to_str().expect("utf-8")],
    )
    .await;
    assert!(
        said.contains(&format!("key file: {}", other.display())),
        "{said}"
    );
    assert!(said.contains(&unreachable), "{said}");
    server.kill().await.expect("stop kkt");
}

/// A home folder with nothing in it at all, as on a new machine, removed
/// when dropped.
fn empty_home(name: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-setup-{name}"))
}

/// The admin key the fake `docker`'s kkt container has once
/// `kkt key create --admin` has run in it.
const FAKE_ADMIN_KEY: &str = "admin-key-0123456789abcdef0123456789abcdef";

/// A `docker` that does nothing and succeeds, except that asking the kkt
/// container for its key prints `key`, it has an admin key
/// ([`FAKE_ADMIN_KEY`]) only once `kkt key create --admin` has run, listing
/// running services lists kkt when `FAKE_KKT_RUNNING` is set, `kkt backup`
/// in the container says where it backed up to, starting the Tailscale
/// container fails, and each call is added to the file `FAKE_DOCKER_LOG`
/// names, if set. Put in its own folder for PATH.
fn fake_docker(dir: &Path, key: &str) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let docker = bin.join("docker");
    let admin_made = dir.join("admin-made");
    let admin_made = admin_made.display();
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\n[ -n \"$FAKE_DOCKER_LOG\" ] && echo \"$*\" >> \"$FAKE_DOCKER_LOG\"\n\
             case \"$*\" in\n\
             *\"kkt key create --admin\"*) : > '{admin_made}' ;;\n\
             *\"kkt key show --admin\"*) [ -f '{admin_made}' ] || exit 4; echo '{FAKE_ADMIN_KEY}' ;;\n\
             *\"kkt key show --web\"*) echo 'web-key-0123456789abcdef0123456789abcdef' ;;\n\
             *\"kkt key show\"*) echo '{key}' ;;\n\
             *\"kkt backup\"*) echo 'Backed up to /backups/kkt-20261029T120000.000Z-v0.4.2' ;;\n\
             *\"ps --status running --services\"*) [ -n \"$FAKE_KKT_RUNNING\" ] && echo kkt ;;\n\
             *\"up -d tailscale\"*) exit 1 ;;\n\
             esac\nexit 0\n"
        ),
    )
    .expect("fake docker");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
    }
    bin
}

#[tokio::test]
async fn docker_setup_works_from_a_home_with_nothing_in_it() {
    // A real server stands in for the container, on the port setup is told
    // Docker publishes, so the connection check is real.
    let (_server_home, mut server, url, key) = keyed_server("docker-server").await;
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .to_string();
    let key_text = std::fs::read_to_string(&key)
        .expect("key")
        .trim()
        .to_string();
    let scratch = empty_home("docker-scratch");
    let bin = fake_docker(&scratch, &key_text);
    let home = empty_home("docker-home");

    let out = Command::new(KKT)
        .args([
            "setup",
            "--mode",
            "docker",
            "--no-tailscale",
            "--clients",
            "codex",
            "--allow-host",
            "host.docker.internal",
        ])
        .args(["--docker-project", "kkt-test", "--docker-port", &port])
        .own_home(&home)
        .env("PATH", &bin)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out)
        .await
        .expect("setup should finish")
        .expect("run kkt");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{said}");
    let web_saved = home.join(".config/kkt/docker-web-key");
    let web_text = std::fs::read_to_string(&web_saved).expect("saved web key");
    assert_eq!(web_text.trim(), "web-key-0123456789abcdef0123456789abcdef");
    assert!(!said.contains(web_text.trim()));
    let shown = Command::new(KKT)
        .args(["key", "show", "--web"])
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .output()
        .await
        .expect("show web key");
    assert!(shown.status.success());
    assert_eq!(String::from_utf8_lossy(&shown.stdout), web_text);
    let saved = home.join(".config/kkt/docker-access-key");
    assert_eq!(
        std::fs::read_to_string(&saved).expect("saved key").trim(),
        key_text
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&saved)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(
            std::fs::metadata(&web_saved).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let record = record_text(&home);
    assert!(record.contains("mode = \"docker\""), "{record}");
    assert!(record.contains(&format!("url = \"{url}\"")), "{record}");
    let client_config =
        std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config");
    assert!(!client_config.contains(web_text.trim()));
    assert!(!client_config.contains("web-key"));
    assert!(client_config.contains("docker-access-key"));
    // The extra host name reaches the managed settings file.
    let data = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/kashkeshet")
    } else {
        home.join(".local/share/kashkeshet")
    };
    let env = std::fs::read_to_string(data.join("docker/.env")).expect("managed .env");
    assert!(
        env.contains("KKT_ALLOW_HOSTS=host.docker.internal\n"),
        "{env}"
    );
    assert!(
        said.contains(&format!("the web page is at http://127.0.0.1:{port}/")),
        "{said}"
    );
    // Not asked for, so no admin key is made or saved.
    assert!(!home.join(".config/kkt/docker-admin-key").exists());
    assert!(!said.contains("admin key"), "{said}");
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn docker_setup_with_an_admin_key_makes_it_in_the_container_and_saves_a_private_copy() {
    let (_server_home, mut server, url, key) = keyed_server("docker-admin-server").await;
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .to_string();
    let key_text = std::fs::read_to_string(&key)
        .expect("key")
        .trim()
        .to_string();
    let scratch = empty_home("docker-admin-scratch");
    let bin = fake_docker(&scratch, &key_text);
    let home = empty_home("docker-admin-home");
    let log = scratch.join("docker.log");

    let out = Command::new(KKT)
        .args([
            "setup",
            "--mode",
            "docker",
            "--no-tailscale",
            "--clients",
            "codex",
            "--allow-host",
            "none",
            "--admin-key",
        ])
        .args(["--docker-project", "kkt-test", "--docker-port", &port])
        .own_home(&home)
        .env("PATH", &bin)
        .env("FAKE_DOCKER_LOG", &log)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out)
        .await
        .expect("setup should finish")
        .expect("run kkt");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{said}");
    let admin_saved = home.join(".config/kkt/docker-admin-key");
    assert_eq!(
        std::fs::read_to_string(&admin_saved).expect("saved admin key"),
        format!("{FAKE_ADMIN_KEY}\n")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&admin_saved)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert!(!said.contains(FAKE_ADMIN_KEY), "{said}");
    assert!(said.contains("docker-admin-key"), "{said}");
    // Made in the container, then kkt restarted to read it.
    let ran = std::fs::read_to_string(&log).expect("docker log");
    let create = ran.find("kkt key create --admin").expect("made the key");
    let restart = ran.find("restart kkt").expect("restarted kkt");
    assert!(create < restart, "{ran}");
    assert!(!ran.contains(FAKE_ADMIN_KEY), "{ran}");
    let shown = Command::new(KKT)
        .args(["key", "show", "--admin"])
        .own_home(&home)
        .output()
        .await
        .expect("show admin key");
    assert!(shown.status.success());
    assert_eq!(
        String::from_utf8_lossy(&shown.stdout),
        format!("{FAKE_ADMIN_KEY}\n")
    );
    let status = Command::new(KKT)
        .arg("status")
        .own_home(&home)
        .env("PATH", &bin)
        .output()
        .await
        .expect("status");
    let status = String::from_utf8_lossy(&status.stdout).to_string();
    assert!(
        status.contains(&format!("Admin key: {}", admin_saved.display())),
        "{status}"
    );
    let client_config =
        std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config");
    assert!(
        !client_config.contains(FAKE_ADMIN_KEY) && !client_config.contains("docker-admin-key"),
        "{client_config}"
    );
    server.kill().await.expect("stop kkt");
}

/// `kkt doctor` with a Docker record for `folder`, `bin` as PATH, and the
/// fake kkt container running or not. Returns what it printed.
async fn doctor_for_docker(home: &Path, bin: &Path, folder: &Path, running: bool) -> String {
    write_record(
        home,
        &format!(
            "format = 1\nmode = \"docker\"\nwritten_by = \"0.4.0\"\n\
             url = \"http://{}/mcp\"\n\n[docker]\nfolder = \"{}\"\ntailscale = false\n",
            free_bind(),
            folder.display()
        ),
    );
    let mut command = Command::new(KKT);
    command
        .arg("doctor")
        .own_home(home)
        .env("PATH", bin)
        .env_remove("FAKE_KKT_RUNNING")
        .kill_on_drop(true);
    if running {
        command.env("FAKE_KKT_RUNNING", "1");
    }
    let out = tokio::time::timeout(TIMEOUT, command.output())
        .await
        .expect("doctor should finish")
        .expect("run kkt");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test]
async fn on_a_docker_machine_doctor_reports_the_container_not_the_service() {
    let home = temp_home("doctor-docker");
    let bin = fake_docker(&home, "unused");
    let folder = home.join("docker files");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(folder.join(".env"), "KKT_VERSION=0.4.0\n").expect(".env");

    let said = doctor_for_docker(&home, &bin, &folder, true).await;
    assert!(
        said.contains("ok: this machine's kkt in Docker: running kkt 0.4.0"),
        "{said}"
    );
    assert!(!said.contains("this machine's kkt service"), "{said}");

    let said = doctor_for_docker(&home, &bin, &folder, false).await;
    assert!(
        said.contains("problem: this machine's kkt in Docker: not running"),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "cd '{}' && docker compose up -d",
            folder.display()
        )),
        "{said}"
    );

    std::fs::remove_file(folder.join(".env")).expect("remove .env");
    let said = doctor_for_docker(&home, &bin, &folder, true).await;
    assert!(said.contains("its files are missing from"), "{said}");
    assert!(said.contains("run `kkt setup` again"), "{said}");

    // No docker at all.
    std::fs::write(folder.join(".env"), "KKT_VERSION=0.4.0\n").expect(".env");
    let said = doctor_for_docker(&home, &home.join("no-bin"), &folder, true).await;
    assert!(
        said.contains("problem: this machine's kkt in Docker: "),
        "{said}"
    );
    assert!(said.contains("check that Docker is running"), "{said}");
}

#[tokio::test]
async fn on_a_client_machine_doctor_has_no_line_about_a_server_here() {
    let home = temp_home("doctor-client");
    write_record(&home, CLIENT);
    let (_, said) = kkt(&home, &["doctor"]).await;
    assert!(said.contains("https://kkt.tail1234.ts.net/mcp"), "{said}");
    assert!(!said.contains("this machine's kkt"), "{said}");
}

#[tokio::test]
async fn setup_waits_for_a_server_it_set_up_that_answers_late() {
    // As with a new Tailscale node, whose HTTPS certificate comes on first
    // use: the server only answers a few seconds after setup's check starts.
    let (server_home, key) = server_key("late-server").await;
    let bind = free_bind();
    let port = bind.trim_start_matches("127.0.0.1:").to_string();
    let key_text = std::fs::read_to_string(&key)
        .expect("key")
        .trim()
        .to_string();
    let scratch = empty_home("late-scratch");
    let bin = fake_docker(&scratch, &key_text);
    let home = empty_home("late-home");

    let setup = Command::new(KKT)
        .args([
            "setup",
            "--mode",
            "docker",
            "--no-tailscale",
            "--clients",
            "codex",
            "--allow-host",
            "none",
        ])
        .args(["--docker-project", "kkt-test", "--docker-port", &port])
        .own_home(&home)
        .env("PATH", &bin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("start setup");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut server = serve_with_key(&server_home, &key, &bind).await;
    let out = tokio::time::timeout(TIMEOUT, setup.wait_with_output())
        .await
        .expect("setup should finish once the server answers")
        .expect("run setup");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{said}");
    assert!(said.contains("Waiting for the server to answer"), "{said}");
    // Set up with --no-tailscale, so the wait says nothing of it.
    assert!(!said.contains("Tailscale node"), "{said}");
    assert!(said.contains("ok: key accepted"), "{said}");
    assert!(record_text(&home).contains("mode = \"docker\""));
    server.kill().await.expect("stop kkt");
}

const LOGGED_IN: &str = r#"{"BackendState":"Running","Self":{"DNSName":"kkt.tail1.ts.net."},"CertDomains":["kkt.tail1.ts.net"]}"#;
const LOGIN_NEEDED: &str = r#"{"BackendState":"NeedsLogin","AuthURL":"","Self":{"DNSName":""}}"#;

/// Runs `kkt <command>` on a machine set up as Docker with Tailscale, where
/// the Tailscale container says `tailscale_status`, or does not answer if
/// that is `None`. Returns what it printed.
async fn tailscale_docker_output(
    name: &str,
    command: &str,
    tailscale_status: Option<&str>,
) -> String {
    let home = temp_home(name);
    let folder = home.join("docker");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(
        folder.join(".env"),
        "KKT_VERSION=0.4.0\nCOMPOSE_FILE=compose.tailscale.yaml\n",
    )
    .expect("env");
    write_record(
        &home,
        &format!(
            "format = 1\nmode = \"docker\"\nwritten_by = \"0.4.0\"\n\
             url = \"https://kkt.tail1.ts.net/mcp\"\n\n[docker]\nfolder = \"{}\"\n\
             tailscale = true\nhostname = \"kkt\"\ntailnet = \"tail1.ts.net\"\n",
            folder.display()
        ),
    );
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let docker = bin.join("docker");
    std::fs::write(
        &docker,
        "#!/bin/sh\ncase \"$*\" in\n\
         *\"tailscale status --json\"*) [ -n \"$FAKE_TS_STATUS\" ] && { echo \"$FAKE_TS_STATUS\"; exit 0; } || exit 1 ;;\n\
         *\"ps --status running --services\"*) echo kkt ;;\n\
         esac\nexit 0\n",
    )
    .expect("fake docker");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
    }
    let output = || async {
        let mut run = Command::new(KKT);
        run.arg(command)
            .own_home(&home)
            .env("PATH", &bin)
            .env_remove("FAKE_TS_STATUS")
            .kill_on_drop(true);
        if let Some(status) = tailscale_status {
            run.env("FAKE_TS_STATUS", status);
        }
        let out = tokio::time::timeout(TIMEOUT, run.output())
            .await
            .expect("kkt should finish")
            .expect("run kkt");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    // Another test's process starting can briefly hold the script open for
    // writing, so that running it fails with "Text file busy": try again.
    let mut printed = output().await;
    for _ in 0..20 {
        if !printed.contains("Text file busy") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        printed = output().await;
    }
    printed
}

#[tokio::test]
async fn status_and_doctor_say_when_the_tailscale_machine_needs_a_login() {
    let said = tailscale_docker_output("ts-status-login", "status", Some(LOGIN_NEEDED)).await;
    assert!(said.contains("Tailscale login: needed"), "{said}");
    assert!(said.contains("kkt setup"), "{said}");
    let said = tailscale_docker_output("ts-doctor-login", "doctor", Some(LOGIN_NEEDED)).await;
    assert!(
        said.contains("problem: this machine's Tailscale login: not logged in"),
        "{said}"
    );
    assert!(said.contains("fix: run `kkt setup` again"), "{said}");
}

#[tokio::test]
async fn status_and_doctor_are_quiet_about_tailscale_when_it_is_logged_in() {
    let said = tailscale_docker_output("ts-status-ok", "status", Some(LOGGED_IN)).await;
    assert!(said.contains("Tailscale login: logged in"), "{said}");
    let said = tailscale_docker_output("ts-doctor-ok", "doctor", Some(LOGGED_IN)).await;
    assert!(
        said.contains("ok: this machine's Tailscale login: logged in"),
        "{said}"
    );
}

#[tokio::test]
async fn a_silent_tailscale_container_is_not_called_logged_out() {
    let said = tailscale_docker_output("ts-status-down", "status", None).await;
    assert!(said.contains("Tailscale login: unknown"), "{said}");
    assert!(!said.contains("needed"), "{said}");
    let said = tailscale_docker_output("ts-doctor-down", "doctor", None).await;
    assert!(
        said.contains(
            "problem: this machine's Tailscale login: the Tailscale container does not answer"
        ),
        "{said}"
    );
}

#[tokio::test]
async fn doctor_does_not_tell_a_docker_or_client_machine_to_start_the_service() {
    // The server is not answering, as when the Tailscale machine is off the
    // tailnet. The old fix, `kkt service start`, is wrong on these machines.
    let said = tailscale_docker_output("ts-doctor-hint", "doctor", Some(LOGIN_NEEDED)).await;
    assert!(said.contains("problem: server"), "{said}");
    assert!(!said.contains("kkt service start"), "{said}");
    assert!(
        said.contains("this machine runs kkt in Docker: run `kkt status`, or start it with `cd "),
        "{said}"
    );
    // A client machine's server is on another machine.
    let home = temp_home("doctor-hint-client");
    write_record(
        &home,
        &format!(
            "format = 1\nmode = \"client\"\nwritten_by = \"0.4.0\"\nurl = \"http://{}/mcp\"\n",
            free_bind()
        ),
    );
    let out = tokio::time::timeout(
        TIMEOUT,
        Command::new(KKT)
            .arg("doctor")
            .own_home(&home)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("doctor should finish")
    .expect("run kkt");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!said.contains("kkt service start"), "{said}");
    assert!(
        said.contains("the server runs on another machine"),
        "{said}"
    );
}

/// `kkt <args>` with `home`, `bin` as PATH, no terminal, and the fake
/// docker's calls added to `log`.
async fn kkt_with_docker(home: &Path, bin: &Path, log: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(KKT)
        .args(args)
        .env("HOME", home)
        .env("PATH", bin)
        .env("FAKE_DOCKER_LOG", log)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out)
        .await
        .expect("kkt should finish")
        .expect("run kkt");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[tokio::test]
async fn upgrading_a_docker_install_from_before_the_web_key_saves_its_host_copy() {
    // As `kkt setup` 0.4.2 left it: a record, the managed folder and the
    // access key's host copy, with no web key on the host.
    let scratch = empty_home("upgrade-scratch");
    let bin = fake_docker(&scratch, "0123456789abcdefghijklmnopqrstuvwxyz");
    let log = scratch.join("docker.log");
    let home = temp_home("upgrade-docker");
    let folder = home.join("docker files");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(
        folder.join(".env"),
        "COMPOSE_PROJECT_NAME=kkt\nKKT_VERSION=0.4.2\n",
    )
    .expect(".env");
    let key_file = home.join(".config/kkt/docker-access-key");
    std::fs::write(&key_file, "0123456789abcdefghijklmnopqrstuvwxyz\n").expect("access key");
    write_record(
        &home,
        &format!(
            "format = 1\nmode = \"docker\"\nwritten_by = \"0.4.2\"\n\
             url = \"http://127.0.0.1:7420/mcp\"\nkey_file = \"{}\"\n\n\
             [docker]\nfolder = \"{}\"\ntailscale = false\n",
            key_file.display(),
            folder.display()
        ),
    );

    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["upgrade"]).await;
    assert!(ok, "{said}");
    let web_saved = home.join(".config/kkt/docker-web-key");
    assert_eq!(
        std::fs::read_to_string(&web_saved).expect("saved web key"),
        "web-key-0123456789abcdef0123456789abcdef\n"
    );
    assert!(said.contains("The web key is saved in"), "{said}");
    assert!(said.contains("Upgraded the server to kkt"), "{said}");
    assert!(!said.contains("web-key-0123"), "{said}");
    assert!(!said.contains("0123456789abcdefghij"), "{said}");
    let docker_calls = std::fs::read_to_string(&log).expect("docker log");
    let up = docker_calls.find("up -d").expect("started");
    let web = docker_calls
        .find("key show --web")
        .expect("asked for the web key");
    assert!(up < web, "{docker_calls}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for saved in [&web_saved, &key_file] {
            assert_eq!(
                std::fs::metadata(saved).unwrap().permissions().mode() & 0o777,
                0o600,
                "{}",
                saved.display()
            );
        }
    }
    let (_, shown) = kkt_with_docker(&home, &bin, &log, &["key", "show", "--web"]).await;
    assert_eq!(shown, "web-key-0123456789abcdef0123456789abcdef\n");
}

/// A home with a Docker record whose managed folder runs kkt 0.5.0 and whose
/// access key copy is saved, plus the fake `docker` and its log. The web
/// key's host copy holds `web_copy`, or is missing if `None`.
fn docker_install(
    name: &str,
    web_copy: Option<&str>,
) -> (scratch::Dir, scratch::Dir, PathBuf, PathBuf) {
    let scratch = empty_home(&format!("{name}-scratch"));
    let bin = fake_docker(&scratch, "0123456789abcdefghijklmnopqrstuvwxyz");
    let log = scratch.join("docker.log");
    let home = temp_home(name);
    let folder = home.join("docker files");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(folder.join(".env"), "KKT_VERSION=0.5.0\n").expect(".env");
    let key_file = home.join(".config/kkt/docker-access-key");
    std::fs::write(&key_file, "0123456789abcdefghijklmnopqrstuvwxyz\n").expect("access key");
    if let Some(copy) = web_copy {
        std::fs::write(home.join(".config/kkt/docker-web-key"), copy).expect("web key");
    }
    write_record(
        &home,
        &format!(
            "format = 1\nmode = \"docker\"\nwritten_by = \"0.5.0\"\n\
             url = \"http://{}/mcp\"\nkey_file = \"{}\"\n\n\
             [docker]\nfolder = \"{}\"\ntailscale = false\n",
            free_bind(),
            key_file.display(),
            folder.display()
        ),
    );
    (home, scratch, bin, log)
}

/// `kkt doctor` on the fake Docker install in `home`, with kkt running.
async fn doctor_with_kkt_running(home: &Path, bin: &Path) -> String {
    let out = Command::new(KKT)
        .arg("doctor")
        .own_home(home)
        .env("PATH", bin)
        .env("FAKE_KKT_RUNNING", "1")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out)
        .await
        .expect("doctor should finish")
        .expect("run kkt");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test]
async fn doctor_says_when_the_docker_web_keys_host_copy_is_not_the_servers() {
    // The fake container's web key is this one.
    let (home, _scratch, bin, _) = docker_install(
        "doctor-web-same",
        Some("web-key-0123456789abcdef0123456789abcdef\n"),
    );
    let said = doctor_with_kkt_running(&home, &bin).await;
    assert!(
        said.contains("ok: web key host copy: the same as the server's"),
        "{said}"
    );

    // Rotated inside the container after setup saved the copy.
    let (home, _scratch, bin, _) = docker_install(
        "doctor-web-stale",
        Some("web-key-stale-6789abcdef0123456789abcdef\n"),
    );
    let said = doctor_with_kkt_running(&home, &bin).await;
    assert!(
        said.contains("problem: web key host copy: differs from the server's web key"),
        "{said}"
    );
    assert!(said.contains("`kkt setup` (run again"), "{said}");
    assert!(
        said.contains("docker compose exec kkt kkt key show --web"),
        "{said}"
    );
    assert!(!said.contains("web-key-0123"), "{said}");
    assert!(!said.contains("web-key-stale"), "{said}");
}

#[tokio::test]
async fn doctor_tells_a_docker_machine_missing_its_web_key_copy_how_to_get_it() {
    let (home, _scratch, bin, _) = docker_install("doctor-web-missing", None);
    let said = doctor_with_kkt_running(&home, &bin).await;
    assert!(said.contains("problem: web key file:"), "{said}");
    assert!(!said.contains("kkt key create --web"), "{said}");
    assert!(said.contains("`kkt upgrade`"), "{said}");
    assert!(
        said.contains("docker compose exec kkt kkt key show --web"),
        "{said}"
    );
    // Nothing to compare: the missing file has its own line.
    assert!(!said.contains("web key host copy"), "{said}");
}

#[tokio::test]
async fn while_docker_setup_is_unfinished_the_web_key_is_its_host_copy() {
    let scratch = empty_home("unfinished-web-scratch");
    let bin = fake_docker(&scratch, "unused");
    let log = scratch.join("docker.log");
    let home = temp_home("unfinished-web");
    std::fs::write(
        home.join(".config/kkt/setup-unfinished.toml"),
        "mode = \"docker\"\ntailscale = false\nallow_hosts = []\nclients = []\n",
    )
    .expect("unfinished note");
    let web_copy = home.join(".config/kkt/docker-web-key");

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(
        said.contains(&format!("Web key: {} ", web_copy.display())),
        "{said}"
    );
    assert!(!said.contains(".config/kkt/web-key"), "{said}");

    std::fs::write(&web_copy, "web-key-0123456789abcdef0123456789abcdef\n").expect("web key");
    let (ok, shown) = kkt_with_docker(&home, &bin, &log, &["key", "show", "--web"]).await;
    assert!(ok, "{shown}");
    assert_eq!(shown, "web-key-0123456789abcdef0123456789abcdef\n");
}

#[tokio::test]
async fn setup_after_a_failed_check_continues_without_installing_again() {
    // The container hands out a key the server refuses, so the connection
    // check after the install fails.
    let (_server_home, mut server, url, key) = keyed_server("resume-server").await;
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .to_string();
    let scratch = empty_home("resume-scratch");
    let bin = fake_docker(&scratch, "0123456789abcdefghijklmnopqrstuvwxyz");
    let log = scratch.join("docker.log");
    let home = empty_home("resume-home");
    // Codex is found, but the first setup chose no clients.
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().expect("dir")).expect("codex dir");
    std::fs::write(&codex, "model = \"o3\"\n").expect("codex config");
    let flags = [
        "setup",
        "--mode",
        "docker",
        "--no-tailscale",
        "--clients",
        "none",
        "--allow-host",
        "none",
        "--docker-project",
        "kkt-test",
        "--docker-port",
        &port,
    ];

    let (ok, said) = kkt_with_docker(&home, &bin, &log, &flags).await;
    assert!(!ok, "{said}");
    assert!(said.contains("nothing was recorded"), "{said}");
    assert!(said.contains("installed a server here in Docker"), "{said}");
    assert!(said.contains("Run `kkt setup` again to finish"), "{said}");
    assert!(!home.join(".config/kkt/setup.toml").exists());
    let docker_calls = std::fs::read_to_string(&log).expect("docker log");
    assert!(docker_calls.contains("up -d"), "{docker_calls}");

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(
        said.contains("Unfinished setup: An earlier `kkt setup` installed"),
        "{said}"
    );
    assert!(said.contains("Mode: docker"), "{said}");
    let (_, said) = kkt_with_docker(&home, &bin, &log, &["doctor"]).await;
    assert!(said.contains(&format!("reachable at {url}")), "{said}");
    assert!(
        said.contains("problem: this machine's setup: An earlier `kkt setup` installed"),
        "{said}"
    );

    // Fixed by hand: the right key in the file setup saved it to. Setup
    // again, with no flags at all and no terminal, continues from the check
    // without asking about Codex again.
    std::fs::copy(&key, home.join(".config/kkt/docker-access-key")).expect("fix the key");
    std::fs::write(&log, "").expect("clear the log");
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["setup"]).await;
    assert!(ok, "{said}");
    assert!(
        said.contains("Continuing from the connection check."),
        "{said}"
    );
    assert!(said.contains("ok: key accepted"), "{said}");
    assert_eq!(std::fs::read_to_string(&log).expect("docker log"), "");
    assert_eq!(
        std::fs::read_to_string(&codex).expect("codex config"),
        "model = \"o3\"\n"
    );
    let record = record_text(&home);
    assert!(record.contains("mode = \"docker\""), "{record}");
    assert!(!home.join(".config/kkt/setup-unfinished.toml").exists());

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(!said.contains("Unfinished"), "{said}");
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn an_install_that_fails_after_starting_the_server_is_found_and_continued() {
    let (server_home, key) = server_key("key-fail-server").await;
    let bind = free_bind();
    let port = bind.trim_start_matches("127.0.0.1:").to_string();
    let mut server = serve_with_key(&server_home, &key, &bind).await;
    let key_text = std::fs::read_to_string(&key)
        .expect("key")
        .trim()
        .to_string();
    let scratch = empty_home("key-fail-scratch");
    let bin = fake_docker(&scratch, &key_text);
    let log = scratch.join("docker.log");
    let home = empty_home("key-fail-home");
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().expect("dir")).expect("codex dir");
    std::fs::write(&codex, "").expect("codex config");
    // The containers start, but the access key can't be saved.
    let key_file = home.join(".config/kkt/docker-access-key");
    std::fs::create_dir_all(&key_file).expect("a folder where the key goes");

    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &[
            "setup",
            "--mode",
            "docker",
            "--no-tailscale",
            "--clients",
            "codex",
            "--allow-host",
            "none",
            "--docker-project",
            "kkt-test",
            "--docker-port",
            &port,
        ],
    )
    .await;
    assert!(!ok, "{said}");
    assert!(
        said.contains("began to install a server here in Docker"),
        "{said}"
    );
    assert!(said.contains("continues from the install"), "{said}");
    let docker_calls = std::fs::read_to_string(&log).expect("docker log");
    assert!(docker_calls.contains("up -d"), "{docker_calls}");
    assert!(home.join(".config/kkt/setup-unfinished.toml").exists());
    assert!(!home.join(".config/kkt/setup.toml").exists());

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(
        said.contains("Unfinished setup: An earlier `kkt setup` began to install"),
        "{said}"
    );
    assert!(said.contains("Mode: docker"), "{said}");
    assert!(said.contains("Docker folder:"), "{said}");
    let (_, said) = kkt_with_docker(&home, &bin, &log, &["doctor"]).await;
    assert!(
        said.contains("problem: this machine's setup: An earlier `kkt setup` began to install"),
        "{said}"
    );
    assert!(said.contains("this machine's kkt in Docker"), "{said}");

    // Fixed by hand. Setup again, with none of the answers and no terminal,
    // installs again with the same answers and connects the client chosen
    // before. (The hidden test flags are given again: they are not answers.)
    std::fs::remove_dir(&key_file).expect("remove the folder");
    std::fs::write(&log, "").expect("clear the log");
    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &[
            "setup",
            "--docker-project",
            "kkt-test",
            "--docker-port",
            &port,
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(said.contains("Continuing from the install."), "{said}");
    let docker_calls = std::fs::read_to_string(&log).expect("docker log");
    assert!(docker_calls.contains("up -d"), "{docker_calls}");
    assert!(said.contains("ok: key accepted"), "{said}");
    assert!(
        std::fs::read_to_string(&codex)
            .expect("codex config")
            .contains("kkt"),
        "Codex is connected"
    );
    assert!(record_text(&home).contains("mode = \"docker\""));
    assert!(!home.join(".config/kkt/setup-unfinished.toml").exists());
    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn an_install_continued_after_failing_keeps_its_tailscale_name_and_auth_key() {
    // The Tailscale container fails to start, before any login.
    let scratch = empty_home("ts-resume-scratch");
    let bin = fake_docker(&scratch, "unused");
    let log = scratch.join("docker.log");
    let home = empty_home("ts-resume-home");
    let auth_key_file = scratch.join("tailscale-key");
    std::fs::write(&auth_key_file, "tskey-auth-secret123\n").expect("auth key");
    let auth_key_path = auth_key_file.display().to_string();
    let test_flags = ["--docker-project", "kkt-test", "--docker-port", "1"];
    let data = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/kashkeshet")
    } else {
        home.join(".local/share/kashkeshet")
    };
    let env_file = data.join("docker/.env");
    let asks_for_both = |env: &str| {
        assert!(env.contains("TS_HOSTNAME=kkt-work\n"), "{env}");
        assert!(env.contains("TS_AUTHKEY=tskey-auth-secret123\n"), "{env}");
    };

    let mut first = vec![
        "setup",
        "--mode",
        "docker",
        "--tailscale",
        "--ts-hostname",
        "kkt-work",
        "--ts-authkey-file",
        &auth_key_path,
        "--clients",
        "none",
        "--allow-host",
        "none",
    ];
    first.extend(test_flags);
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &first).await;
    assert!(!ok, "{said}");
    assert!(said.contains("continues from the install"), "{said}");
    asks_for_both(&std::fs::read_to_string(&env_file).expect("managed .env"));
    // The note names the key's file, never the key.
    let note = std::fs::read_to_string(home.join(".config/kkt/setup-unfinished.toml"))
        .expect("unfinished note");
    assert!(note.contains("ts_hostname = \"kkt-work\""), "{note}");
    assert!(note.contains(&auth_key_path), "{note}");
    assert!(!note.contains("tskey-auth-secret123"), "{note}");

    // Setup again with no answers installs again with the same name and key.
    std::fs::remove_file(&env_file).expect("remove .env");
    let mut again = vec!["setup"];
    again.extend(test_flags);
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &again).await;
    assert!(!ok, "{said}");
    assert!(said.contains("Continuing from the install."), "{said}");
    asks_for_both(&std::fs::read_to_string(&env_file).expect("managed .env"));
    assert!(!said.contains("tskey-auth-secret123"), "{said}");
}
