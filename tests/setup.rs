//! `kkt serve` on a machine `kkt setup` set up: run with the real binary in
//! its own home directory, with a setup record written by hand.

#[path = "common/home.rs"]
mod home;
use kkt_scratch as scratch;
#[path = "common/serving.rs"]
mod serving;
use home::OwnHome;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const TIMEOUT: Duration = Duration::from_secs(20);

/// A home with an empty kkt config folder, removed when dropped.
fn temp_home(name: &str) -> scratch::Dir {
    let dir = empty_home(name);
    std::fs::create_dir_all(dir.join(".config/kkt")).expect("temp home");
    dir
}

/// An address nothing listens on, for a server that is not there.
fn free_bind() -> String {
    format!("127.0.0.1:{}", serving::free_port())
}

fn write_record(home: &Path, text: &str) {
    std::fs::write(home.join(".config/kkt/setup.toml"), text).expect("record");
}

/// `kkt serve` with its own database, plus `extra`; `refused` and `starts`
/// add the address.
fn serve(home: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(KKT);
    command
        .arg("serve")
        .args(extra)
        .arg("--db")
        .arg(home.join("kkt.db"))
        .arg("--no-access-key")
        .own_home(home)
        .kill_on_drop(true);
    command
}

/// Runs `command`, which should refuse to start; returns what it printed.
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn refused(mut command: Command) -> String {
    // Port 0, so a server that wrongly starts takes no port another needs.
    command.args(["--bind", "127.0.0.1:0"]);
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
async fn starts(command: Command) {
    let mut server = serving::start(command).await;
    server.child.kill().await.expect("stop kkt");
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
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
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

#[tokio::test]
async fn a_client_can_back_up_and_import_lanes_on_an_explicit_local_database() {
    let home = temp_home("client-local-target");
    starts(serve(&home, &[])).await;
    write_record(&home, CLIENT);
    let config = home.join("other.toml");
    std::fs::write(&config, "db = \"kkt.db\"\nno_access_key = true\n").expect("config");
    let lanes = home.join("lanes");
    std::fs::create_dir(&lanes).expect("lanes");
    let db = home.join("kkt.db");
    for (flag, path, result) in [
        ("--db", db.as_path(), "created work (version 1)"),
        ("--config", config.as_path(), "updated work (version 2)"),
    ] {
        let target = path.to_str().expect("target");
        let backup = home.join(flag);
        let (ok, said) = kkt(&home, &["backup", backup.to_str().unwrap(), flag, target]).await;
        assert!(ok && said.contains("Backed up to"), "{said}");
        std::fs::write(
            lanes.join("work.md"),
            format!("---\nname: work\ndescription: local lane\n---\n\nSelected by {flag}\n"),
        )
        .expect("lane");
        let (ok, said) = kkt(
            &home,
            &["lanes", "import", lanes.to_str().unwrap(), flag, target],
        )
        .await;
        assert!(ok && said.contains(result), "{said}");
    }
    for args in [
        vec!["backup", "out"],
        vec!["lanes", "import", "lanes"],
        vec!["lanes", "export", "lanes"],
    ] {
        let (ok, said) = kkt(&home, &args).await;
        assert!(!ok && said.contains("this machine is a client"), "{said}");
    }
    let exported = home.join("exported");
    let (ok, said) = kkt(
        &home,
        &[
            "lanes",
            "export",
            exported.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(exported.join("work.md").exists());
    assert_eq!(record_text(&home), CLIENT);
}

/// A server with an access key, in its own home; returns the home, the
/// server, its MCP URL and its key file. The home comes first so that, bound
/// in that order, it is dropped (and removed) after the server.
async fn keyed_server(name: &str) -> (scratch::Dir, tokio::process::Child, String, PathBuf) {
    let (home, key) = server_key(name).await;
    let (child, bind) = serve_with_key(&home, &key).await;
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

/// `kkt serve` with `key`, without an address.
fn serve_with_key_command(home: &Path, key: &Path) -> Command {
    let mut command = Command::new(KKT);
    command
        .args(["serve", "--access-key-file"])
        .arg(key)
        .arg("--db")
        .arg(home.join("kkt.db"))
        .own_home(home);
    command
}

/// Starts `kkt serve` with `key` on a free port, returning it and its
/// address once it listens.
async fn serve_with_key(home: &Path, key: &Path) -> (tokio::process::Child, String) {
    let server = serving::start(serve_with_key_command(home, key)).await;
    let bind = server.bind();
    (server.child, bind)
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

    // Status on a client machine says nothing about the server's name pack,
    // which can't be seen or changed from here.
    let (ok, said) = kkt(&home, &["status"]).await;
    assert!(ok, "{said}");
    assert!(
        said.contains("Client only; no server is managed on this machine."),
        "{said}"
    );
    assert!(!said.contains("Name pack"), "{said}");
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

fn codex_config(home: &Path) -> String {
    std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config")
}

fn arg(path: &Path) -> &str {
    path.to_str().expect("utf-8")
}

/// A client machine set up for one server moves to another with `kkt
/// switch`: its connected client is rewritten (the old config kept as a
/// backup), and doctor and register use the new server without flags.
#[tokio::test]
async fn kkt_switch_moves_a_client_machine_to_another_server() {
    let (_old_home, mut old, old_url, old_key) = keyed_server("switch-old").await;
    let (_new_home, mut new, new_url, new_key) = keyed_server("switch-new").await;
    let home = temp_home("switch-client");
    let (ok, said) = setup_client(&home, &old_url, &old_key, &["--clients", "codex"]).await;
    assert!(ok, "{said}");
    let before = codex_config(&home);

    let (ok, said) = kkt(&home, &["switch", &new_url, "--key-file", arg(&new_key)]).await;
    assert!(ok, "{said}");
    assert!(said.contains(&format!("(before: {old_url})")), "{said}");
    assert!(
        said.contains(&format!("clients now use {new_url}.")),
        "{said}"
    );
    assert!(codex_config(&home).contains(&new_url));
    assert_eq!(
        std::fs::read_to_string(home.join(".codex/config.toml.bak")).expect("backup"),
        before
    );
    let record = record_text(&home);
    assert!(record.contains("mode = \"client\""), "{record}");
    assert!(record.contains(&format!("url = \"{new_url}\"")), "{record}");
    assert!(!record.contains("[clients]"), "{record}");

    let (_, said) = kkt(&home, &["doctor"]).await;
    assert!(
        said.contains(&format!("ok: server: reachable at {new_url}")),
        "{said}"
    );
    assert!(said.contains("ok: key accepted"), "{said}");
    let token = home.join("token");
    let (ok, said) = kkt(&home, &["register", "--token-file", arg(&token)]).await;
    assert!(ok, "registered with the new server: {said}");
    old.kill().await.expect("stop kkt");
    new.kill().await.expect("stop kkt");
}

/// A record for a service at `url` with `key`, as setup writes one.
fn service_record(url: &str, key: &Path) -> String {
    format!(
        "format = 1\nmode = \"service\"\nwritten_by = \"0.6.0\"\nurl = \"{url}\"\n\
         key_file = \"{}\"\n\n[service]\ntailscale = false\n",
        key.display()
    )
}

/// On a machine that runs a server, switching moves only the clients: the
/// server stays recorded as it was (nothing can run here, as PATH is
/// empty), `--local` brings the clients back, and `kkt setup --mode client`
/// does the same as `kkt switch`.
#[tokio::test]
async fn switching_a_server_machine_moves_only_its_clients() {
    let (_local_home, mut local, local_url, local_key) = keyed_server("switch-local").await;
    let (_other_home, mut other, other_url, other_key) = keyed_server("switch-other").await;
    let home = temp_home("switch-server");
    let original = service_record(&local_url, &local_key);
    write_record(&home, &original);
    let (ok, said) = kkt(&home, &["connect", "codex", "--write"]).await;
    assert!(ok, "{said}");
    assert!(codex_config(&home).contains(&local_url));

    let (ok, said) = kkt(
        &home,
        &["switch", &other_url, "--key-file", arg(&other_key)],
    )
    .await;
    assert!(ok, "{said}");
    assert!(
        said.contains("kkt runs here as a service, and stays as it is"),
        "{said}"
    );
    let switched = format!(
        "{original}\n[clients]\nurl = \"{other_url}\"\nkey_file = \"{}\"\n",
        other_key.display()
    );
    assert_eq!(record_text(&home), switched);
    assert!(codex_config(&home).contains(&other_url));
    let (_, said) = kkt(&home, &["doctor"]).await;
    assert!(
        said.contains(&format!("ok: server: reachable at {other_url}")),
        "{said}"
    );

    let (ok, said) = kkt(&home, &["switch", "--local"]).await;
    assert!(ok, "{said}");
    assert_eq!(record_text(&home), original);
    assert!(codex_config(&home).contains(&local_url));

    let (ok, said) = setup_client(&home, &other_url, &other_key, &["--clients", "codex"]).await;
    assert!(ok, "{said}");
    assert!(said.contains("stays as it is"), "{said}");
    assert_eq!(record_text(&home), switched);
    local.kill().await.expect("stop kkt");
    other.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn a_switch_whose_check_fails_changes_nothing() {
    let (_server_home, mut server, url, key) = keyed_server("switch-fail-server").await;
    let home = temp_home("switch-fail");
    let (ok, said) = setup_client(&home, &url, &key, &["--clients", "codex"]).await;
    assert!(ok, "{said}");
    let (record, codex) = (record_text(&home), codex_config(&home));
    let unreachable = format!("http://{}/mcp", free_bind());
    let (ok, said) = kkt(&home, &["switch", &unreachable]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("nothing was changed"), "{said}");
    assert_eq!(record_text(&home), record);
    assert_eq!(codex_config(&home), codex);
    assert!(!home.join(".codex/config.toml.bak").exists());
    server.kill().await.expect("stop kkt");
}

/// Setting up a Docker server where a service is recorded would leave the
/// service running unmanaged, so it is refused before anything changes.
#[tokio::test]
async fn setup_never_drops_a_recorded_server_for_the_other_kind() {
    let home = temp_home("switch-kind");
    let record = service_record("http://127.0.0.1:7420/mcp", &home.join("access-key"));
    write_record(&home, &record);
    let (ok, said) = kkt(
        &home,
        &[
            "setup",
            "--mode",
            "docker",
            "--no-tailscale",
            "--allow-host",
            "none",
            "--clients",
            "none",
        ],
    )
    .await;
    assert!(!ok, "{said}");
    assert!(said.contains("kkt runs here as a service"), "{said}");
    assert!(said.contains("kkt uninstall"), "{said}");
    assert_eq!(record_text(&home), record);
    assert!(!home.join(".config/kkt/setup-unfinished.toml").exists());
}

/// A home folder with nothing in it at all, as on a new machine, removed
/// when dropped.
fn empty_home(name: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-setup-{name}"))
}

/// The admin key the fake `docker`'s kkt container has once
/// `kkt key create --admin` has run in it.
const FAKE_ADMIN_KEY: &str = "admin-key-0123456789abcdef0123456789abcdef";

/// The file, in the fake `docker`'s folder, that stands for the config file
/// in the kkt container's data volume.
const FAKE_CONTAINER_CONFIG: &str = "container-config.toml";

/// The file, in the fake `docker`'s folder, whose presence makes `docker
/// compose version` fail, as when Compose is not installed.
const FAKE_NO_COMPOSE: &str = "no-compose";

/// A `docker` that does nothing and succeeds, except that reading the kkt
/// container's config file prints [`FAKE_CONTAINER_CONFIG`] in `dir`, if it
/// is there, `compose version` fails when [`FAKE_NO_COMPOSE`] is there,
/// asking the kkt container for its key prints `key`, it has an admin key
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
    let container_config = dir.join(FAKE_CONTAINER_CONFIG);
    let container_config = container_config.display();
    let no_compose = dir.join(FAKE_NO_COMPOSE);
    let no_compose = no_compose.display();
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\n[ -n \"$FAKE_DOCKER_LOG\" ] && echo \"$*\" >> \"$FAKE_DOCKER_LOG\"\n\
             case \"$*\" in\n\
             *\"--entrypoint sh kkt -c\"*) [ -f '{container_config}' ] && \
             while IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{container_config}' ;;\n\
             \"compose version\") [ -f '{no_compose}' ] && exit 1 ;;\n\
             *\"kkt key create --admin\"*) : > '{admin_made}' ;;\n\
             *\"kkt key show --admin\"*) [ -f '{admin_made}' ] || exit 4; echo '{FAKE_ADMIN_KEY}' ;;\n\
             *\"kkt key show --web\"*) echo 'web-key-0123456789abcdef0123456789abcdef' ;;\n\
             *\"kkt key show\"*) echo '{key}' ;;\n\
             *\"kkt backup\"*) echo 'Backed up to /backups/kkt-20261029T120000.000Z-v0.4.2' ;;\n\
             *\"ps --all --quiet kkt\"*) echo abc123 ;;\n\
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
async fn docker_setup_announces_existing_storage_before_provisioning() {
    for (case, has_folder, has_volume) in [
        ("fresh", false, false),
        ("folder", true, false),
        ("volume", false, true),
        ("both", true, true),
        ("unavailable", false, false),
    ] {
        let home = temp_home(&format!("reuse-{case}"));
        write_record(&home, CLIENT);
        let folder = if cfg!(target_os = "macos") {
            home.join("Library/Application Support/kashkeshet/docker")
        } else {
            home.join(".local/share/kashkeshet/docker")
        };
        if has_folder {
            std::fs::create_dir_all(&folder).expect("managed folder");
        }
        let bin = home.join("bin");
        // Check for a volume before the first setup write. Stop at Compose
        // so this test never needs Docker or a running server.
        let reply = if case == "unavailable" {
            "exit 1"
        } else if has_volume {
            "echo kkt_kkt-data"
        } else {
            "echo kkt_kkt-data-old"
        };
        script(
            &bin,
            "docker",
            &format!(
                "case \"$*\" in\n'volume ls --quiet --filter name=kkt_kkt-data') [ ! -e \"$HOME/.config/kkt/setup-unfinished.toml\" ] || exit 2; {reply};;\n*) exit 1;;\nesac"
            ),
        );
        let (ok, said) = kkt_with_path(
            &home,
            &bin,
            &[
                "setup",
                "--mode",
                "docker",
                "--no-tailscale",
                "--allow-host",
                "none",
                "--clients",
                "none",
            ],
        )
        .await;
        assert!(
            !ok && said.contains("Docker is not available"),
            "{case}: {said}"
        );
        assert_eq!(
            said.contains("Docker could not check for volume"),
            case == "unavailable",
            "{said}"
        );
        let notice = "Docker setup will reuse existing storage.";
        assert_eq!(
            said.contains(notice),
            has_folder || has_volume,
            "{case}: {said}"
        );
        if has_folder || has_volume {
            assert!(
                said.contains("kkt_kkt-data") && said.contains(folder.to_str().unwrap()),
                "{said}"
            );
            assert!(
                said.find(notice).unwrap() < said.find("Docker is not available").unwrap(),
                "{said}"
            );
        }
    }
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
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

/// `kkt <args>` with `home` as its only home directory, `bin` as PATH, and
/// no terminal.
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn kkt_with_path(home: &Path, bin: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(KKT)
        .args(args)
        .own_home(home)
        .env("PATH", bin)
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

/// A `systemctl` that adds each call to `log`, says the service is running,
/// and fails to stop or start it until the file `works` exists. Put in its
/// own folder for PATH.
#[cfg(target_os = "linux")]
fn fake_systemctl(dir: &Path, log: &Path, works: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let systemctl = bin.join("systemctl");
    std::fs::write(
        &systemctl,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\n\
             case \"$*\" in\n\
             *\" stop \"*|*\" start \"*) [ -f '{}' ] || {{ echo 'Job failed.' >&2; exit 1; }} ;;\n\
             esac\nexit 0\n",
            log.display(),
            works.display()
        ),
    )
    .expect("fake systemctl");
    std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o755))
        .expect("make it runnable");
    bin
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn setup_refuses_to_add_an_admin_key_over_an_access_key_named_admin_key() {
    use kkt_setup::{Mode, Purpose, Record, Unfinished};
    let scratch = empty_home("service-admin-collision-scratch");
    let bin = fake_systemctl(
        &scratch,
        &scratch.join("systemctl.log"),
        &scratch.join("systemctl-works"),
    );
    let home = temp_home("service-admin-collision");
    let unit = home.join(".config/systemd/user/kashkeshet.service");
    std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
    std::fs::write(&unit, "[Service]\n").unwrap();
    let access = home.join(".config/kkt/admin-key");
    let key = "a".repeat(40);
    std::fs::write(&access, &key).unwrap();
    let unfinished = Unfinished {
        purpose: Purpose::Service,
        tailscale: false,
        allow_hosts: vec![],
        clients: vec![],
        tailscale_login: None,
        record: Some(Record {
            mode: Mode::Service { tailscale: false },
            written_by: "0.5.0".into(),
            url: "http://localhost:7420/mcp".into(),
            key_file: Some(access.clone()),
            clients: None,
        }),
    };
    let note = home.join(".config/kkt/setup-unfinished.toml");
    let before = unfinished.render().unwrap();
    std::fs::write(&note, &before).unwrap();
    let (ok, said) = kkt_with_path(&home, &bin, &["setup", "--admin-key"]).await;
    assert!(!ok, "{said}");
    assert!(
        said.contains("access_key_file is named admin-key"),
        "{said}"
    );
    assert!(said.contains("move one of the files"), "{said}");
    assert_eq!(std::fs::read_to_string(&access).unwrap(), key);
    assert_eq!(std::fs::read_to_string(&note).unwrap(), before);
    assert!(!home.join(".config/kkt/setup.toml").exists());
    assert!(!home.join(".config/kkt/admin-key.name").exists());
}

/// A service install the admin key step failed on: the key was made, but
/// the service could not be restarted to read it. Continuing the setup with
/// `--admin-key` tries the restart again with the same key, and records the
/// setup only once the restart works.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn continuing_a_service_setup_whose_restart_failed_restarts_with_the_same_admin_key() {
    use kkt_setup::{Mode, Purpose, Record, Unfinished};
    let (_server_home, mut server, url, key) = keyed_server("service-admin-server").await;
    let scratch = empty_home("service-admin-scratch");
    let log = scratch.join("systemctl.log");
    let works = scratch.join("systemctl-works");
    let bin = fake_systemctl(&scratch, &log, &works);
    let home = temp_home("service-admin-home");
    let unit = home.join(".config/systemd/user/kashkeshet.service");
    std::fs::create_dir_all(unit.parent().expect("dir")).expect("unit dir");
    std::fs::write(&unit, "[Service]\n").expect("unit");
    // The server's own access key, where a service install puts it.
    let access = home.join(".config/kkt/access-key");
    std::fs::copy(&key, &access).expect("access key");
    let admin = home.join(".config/kkt/admin-key");
    // The install finished; the record waits for the remaining steps.
    let unfinished = Unfinished {
        purpose: Purpose::Service,
        tailscale: false,
        allow_hosts: vec![],
        clients: vec![],
        tailscale_login: None,
        record: Some(Record {
            mode: Mode::Service { tailscale: false },
            written_by: "0.0.0".into(),
            url: url.clone(),
            key_file: Some(access.clone()),
            clients: None,
        }),
    };
    std::fs::write(
        home.join(".config/kkt/setup-unfinished.toml"),
        unfinished.render().expect("note"),
    )
    .expect("unfinished note");
    let setup = || kkt_with_path(&home, &bin, &["setup", "--admin-key"]);
    let tried_restart = || {
        std::fs::read_to_string(&log)
            .expect("systemctl log")
            .contains("--user stop kashkeshet.service")
    };

    // The key is made, then the restart fails: nothing is recorded.
    let (ok, said) = setup().await;
    assert!(!ok, "{said}");
    assert!(said.contains("Job failed."), "{said}");
    assert!(said.contains("Run `kkt setup` again to finish"), "{said}");
    assert!(!home.join(".config/kkt/setup.toml").exists());
    let made = std::fs::read_to_string(&admin).expect("admin key made");
    assert!(tried_restart());

    // Continued while the service still cannot restart: the restart is
    // tried again, nothing is recorded, and the key stays.
    std::fs::write(&log, "").expect("clear the log");
    let (ok, said) = setup().await;
    assert!(!ok, "{said}");
    assert!(tried_restart(), "{said}");
    assert!(!home.join(".config/kkt/setup.toml").exists());
    assert_eq!(std::fs::read_to_string(&admin).expect("admin key"), made);

    // Continued once the service restarts: restarted with the same key, and
    // only now is the setup recorded.
    std::fs::write(&works, "").expect("service works");
    std::fs::write(&log, "").expect("clear the log");
    let (ok, said) = setup().await;
    assert!(ok, "{said}");
    let ran = std::fs::read_to_string(&log).expect("systemctl log");
    let stop = ran.find("--user stop kashkeshet.service").expect("stopped");
    let start = ran
        .find("--user start kashkeshet.service")
        .expect("started");
    assert!(stop < start, "{ran}");
    assert!(
        said.contains("The admin key for /mcp-admin is in"),
        "{said}"
    );
    assert_eq!(std::fs::read_to_string(&admin).expect("admin key"), made);
    let record = record_text(&home);
    assert!(record.contains("mode = \"service\""), "{record}");
    assert!(!home.join(".config/kkt/setup-unfinished.toml").exists());
    server.kill().await.expect("stop kkt");
}

/// `kkt doctor` with a Docker record for `folder`, `bin` as PATH, and the
/// fake kkt container running or not. Returns what it printed.
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; a real pause makes the server answer late, and a bound ends a hang"
)]
async fn setup_waits_for_a_server_it_set_up_that_answers_late() {
    // As with a new Tailscale node, whose HTTPS certificate comes on first
    // use: the server only answers a few seconds after setup's check starts.
    let (server_home, key) = server_key("late-server").await;
    // The test holds the port setup is told until the server starts, so no
    // other program, such as a neighbouring test's server, can take it in
    // the meantime. Each connection is accepted and dropped at once, which
    // setup counts as a server that does not answer yet.
    let holder = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("hold a port");
    let port = holder
        .local_addr()
        .expect("held address")
        .port()
        .to_string();
    let holding = tokio::spawn(async move {
        loop {
            let (connection, _) = holder.accept().await.expect("accept");
            drop(connection);
        }
    });
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
    // Let the port go, and wait until it has gone, then start the server on
    // it at once. Setup was told the port before the server started, so the
    // server cannot move to another.
    holding.abort();
    assert!(
        holding
            .await
            .expect_err("the holder runs until stopped")
            .is_cancelled(),
        "the holder stopped by itself"
    );
    let (mut server, _) = serving::try_start_quiet(
        |_| serve_with_key_command(&server_home, &key),
        [port.parse::<u16>().expect("port")],
    )
    .await
    .unwrap_or_else(|failure| panic!("{failure}"));
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
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this polls it in real time and bounds the wait"
)]
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
         *\"ps --quiet kkt\"*) echo abcdef1234 ;;\n\
         *\"inspect --format\"*) echo '{\"Entrypoint\":[\"kkt\"],\"Cmd\":[\"serve\",\"--name-pack\",\"garden\"]}' ;;\n\
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
    assert!(said.contains("Name pack: garden"), "{said}");
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
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn doctor_does_not_tell_a_docker_or_client_machine_to_start_the_service() {
    // The server is not answering, as when the Tailscale machine is off the
    // tailnet. `kkt service start` is the wrong fix on these machines.
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

/// `kkt setup --lanes` checks each lane as `kkt lanes import` would, with
/// the real check, before it installs anything: a folder whose files are
/// well formed but hold a lane the import would refuse stops setup with no
/// container started and no record written.
#[tokio::test]
async fn setup_refuses_a_lanes_folder_the_import_would_refuse_before_installing() {
    let scratch = empty_home("lanes-check-scratch");
    let bin = fake_docker(&scratch, "unused");
    let lane = |name: &str, description: &str, text: &str| {
        format!("---\nname: {name}\ndescription: {description}\n---\n\n{text}\n")
    };
    let cases = [
        (
            "bad-name",
            "Bad_Name",
            lane("Bad_Name", "Reviews.", "Read."),
            "Bad_Name",
        ),
        (
            "no-description",
            "reviewer",
            lane("reviewer", "  ", "Read."),
            "needs a description",
        ),
        (
            "no-instructions",
            "reviewer",
            lane("reviewer", "Reviews.", " \n "),
            "needs instructions",
        ),
    ];
    for (case, file, contents, says) in cases {
        let home = empty_home(&format!("lanes-check-{case}"));
        let lanes = home.join("lanes");
        std::fs::create_dir_all(&lanes).expect("lanes");
        std::fs::write(
            lanes.join("orchestrator.md"),
            lane("orchestrator", "Plans.", "Plan."),
        )
        .expect("good lane");
        std::fs::write(lanes.join(format!("{file}.md")), contents).expect("bad lane");
        let log = home.join("docker.log");

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
                "18999",
                "--lanes",
                arg(&lanes),
            ],
        )
        .await;
        assert!(!ok, "{case}: {said}");
        assert!(said.contains("--lanes"), "{case}: {said}");
        assert!(said.contains(says), "{case}: {said}");
        let ran = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(!ran.contains("up -d"), "{case}: {ran}");
        assert!(!home.join(".config/kkt/setup.toml").exists(), "{case}");
        assert!(
            !managed_folder(&home).join("compose.yaml").exists(),
            "{case}"
        );
    }
}

/// `kkt setup --mode docker --lanes` checks the lanes against the lane text
/// limit of kkt in Docker, never the host's config file, which the
/// container does not read: the default for a new install, and the
/// container's own setting for an existing one. A lane over that limit
/// stops setup before anything is installed, however high the host's is,
/// and a lane under it gets past the check, however low the host's is. A
/// README beside it is skipped with a warning.
#[tokio::test]
async fn docker_setup_checks_lanes_against_the_containers_limit_not_the_hosts() {
    let lane = |chars: usize| {
        format!(
            "---\nname: reviewer\ndescription: Reviews.\n---\n\n{}\n",
            "x".repeat(chars)
        )
    };
    let default = 8000;
    // (case, host's limit, container's config file, lane length, refusal)
    let cases = [
        (
            "new",
            100_000,
            None,
            default + 1,
            Some(format!("at most {default} characters")),
        ),
        (
            "existing",
            100_000,
            Some("max_lane_chars = 20\n"),
            21,
            Some("at most 20 characters".to_string()),
        ),
        ("new-low-host", 20, None, 21, None),
        (
            "existing-low-host",
            20,
            Some("max_lane_chars = 100\n"),
            21,
            None,
        ),
    ];
    for (case, host_limit, container_config, chars, refusal) in cases {
        let scratch = empty_home(&format!("lanes-limit-scratch-{case}"));
        let bin = fake_docker(&scratch, "unused");
        // A lane let through is not imported: setup then stops at once, with
        // no Docker, instead of waiting for a container that never starts.
        std::fs::write(scratch.join(FAKE_NO_COMPOSE), "").expect("no compose");
        let home = empty_home(&format!("lanes-limit-{case}"));
        let config_dir = home.join(".config/kkt");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::write(
            config_dir.join("config.toml"),
            format!("max_lane_chars = {host_limit}\n"),
        )
        .expect("host config");
        let managed = managed_folder(&home);
        if let Some(config) = container_config {
            // An install's settings file, and the container's config.
            std::fs::create_dir_all(&managed).expect("managed folder");
            std::fs::write(
                managed.join(".env"),
                "COMPOSE_PROJECT_NAME=kkt-test\nKKT_VERSION=0.7.0\nKKT_PORT=18999\n",
            )
            .expect("settings file");
            std::fs::write(scratch.join(FAKE_CONTAINER_CONFIG), config).expect("container config");
        }
        let lanes = home.join("lanes");
        std::fs::create_dir_all(&lanes).expect("lanes");
        std::fs::write(lanes.join("reviewer.md"), lane(chars)).expect("lane");
        // Skipped with a warning, as `kkt lanes import` skips it.
        std::fs::write(lanes.join("README.md"), "# Our lanes\n").expect("readme");
        let log = home.join("docker.log");

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
                "18999",
                "--lanes",
                arg(&lanes),
            ],
        )
        .await;
        assert!(!ok, "{case}: {said}");
        let ran = std::fs::read_to_string(&log).unwrap_or_default();
        // Only an existing install's container is asked.
        assert_eq!(
            ran.contains("--entrypoint sh kkt -c"),
            container_config.is_some(),
            "{case}: {ran}"
        );
        match refusal {
            Some(says) => {
                assert!(said.contains("--lanes"), "{case}: {said}");
                assert!(said.contains(&says), "{case}: {said}");
                // Refused before the install began.
                assert!(!ran.contains("compose version"), "{case}: {ran}");
                assert!(
                    !home.join(".config/kkt/setup-unfinished.toml").exists(),
                    "{case}"
                );
            }
            None => {
                assert!(said.contains("Found 1 lanes in"), "{case}: {said}");
                assert!(
                    said.contains(&format!(
                        "warning: skipped {}: a README is not a lane",
                        lanes.join("README.md").display()
                    )),
                    "{case}: {said}"
                );
                assert!(said.contains("Docker is not available"), "{case}: {said}");
            }
        }
        assert!(!ran.contains("up -d"), "{case}: {ran}");
        assert!(!home.join(".config/kkt/setup.toml").exists(), "{case}");
        assert!(!managed.join("compose.yaml").exists(), "{case}");
    }
}

/// `kkt setup --mode service --lanes` checks the lanes against the config
/// file the service runs on, this machine's own, before installing it.
#[tokio::test]
async fn service_setup_checks_lanes_against_this_machines_config_file() {
    let scratch = empty_home("lanes-service-scratch");
    let bin = fake_docker(&scratch, "unused");
    let home = empty_home("lanes-service");
    let config_dir = home.join(".config/kkt");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(config_dir.join("config.toml"), "max_lane_chars = 20\n").expect("config");
    let lanes = home.join("lanes");
    std::fs::create_dir_all(&lanes).expect("lanes");
    std::fs::write(
        lanes.join("reviewer.md"),
        format!(
            "---\nname: reviewer\ndescription: Reviews.\n---\n\n{}\n",
            "x".repeat(21)
        ),
    )
    .expect("lane");
    let log = home.join("docker.log");

    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &[
            "setup",
            "--mode",
            "service",
            "--no-tailscale",
            "--clients",
            "codex",
            "--allow-host",
            "none",
            "--no-admin-key",
            "--lanes",
            arg(&lanes),
        ],
    )
    .await;
    assert!(!ok, "{said}");
    assert!(said.contains("--lanes"), "{said}");
    assert!(said.contains("at most 20 characters"), "{said}");
    // Docker plays no part, and nothing was begun.
    assert!(std::fs::read_to_string(&log).unwrap_or_default().is_empty());
    assert!(!home.join(".config/kkt/setup.toml").exists());
    assert!(!home.join(".config/kkt/setup-unfinished.toml").exists());
}

/// `kkt <args>` with `home`, `bin` as PATH, no terminal, and the fake
/// docker's calls added to `log`.
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
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
    assert!(said.contains("Name pack: unavailable"), "{said}");
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
    let (mut server, bind) = serve_with_key(&server_home, &key).await;
    let port = bind.trim_start_matches("127.0.0.1:").to_string();
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

/// The folder `kkt setup` keeps kkt's Docker files in, under `home`.
fn managed_folder(home: &Path) -> PathBuf {
    let data = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/kashkeshet")
    } else {
        home.join(".local/share/kashkeshet")
    };
    data.join("docker")
}

/// What `kkt setup` 0.4.1 left for Docker, when it stopped after starting
/// the server and before writing the record: the managed folder with `env`
/// and its Compose files, and the access key's host copy holding `key`. No
/// web key copy: 0.4.1 made none.
fn left_by_0_4_1(home: &Path, env: &str, key: &str) -> PathBuf {
    let folder = managed_folder(home);
    std::fs::create_dir_all(folder.join("tailscale")).expect("folder");
    std::fs::write(folder.join(".env"), env).expect(".env");
    for name in [
        "compose.yaml",
        "compose.tailscale.yaml",
        "tailscale/serve.json",
    ] {
        std::fs::write(folder.join(name), "# as 0.4.1 wrote it\n").expect("compose file");
    }
    let key_file = home.join(".config/kkt/docker-access-key");
    std::fs::write(&key_file, format!("{key}\n")).expect("access key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600))
            .expect("private key");
    }
    folder
}

const KEY_0_4_1: &str = "0123456789abcdefghijklmnopqrstuvwxyz";

#[tokio::test]
async fn a_tailscale_docker_install_with_no_record_is_found_and_never_set_up_without_asking() {
    let scratch = empty_home("found-ts-scratch");
    let bin = fake_docker(&scratch, KEY_0_4_1);
    let log = scratch.join("docker.log");
    let home = temp_home("found-ts");
    let folder = left_by_0_4_1(
        &home,
        "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
         KKT_VERSION=0.4.1\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\n\
         KKT_ALLOW_HOSTS=host.docker.internal\nCOMPOSE_FILE=compose.tailscale.yaml\n\
         TS_HOSTNAME=kkt\nTS_TAILNET=tail1234.ts.net\n",
        KEY_0_4_1,
    );
    let found = format!("Found a kkt server in Docker from {}", folder.display());

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(
        said.contains(&format!("Unrecorded install: {found}")),
        "{said}"
    );
    assert!(said.contains("Mode: docker"), "{said}");
    assert!(
        said.contains("Server: https://kkt.tail1234.ts.net/mcp"),
        "{said}"
    );
    assert!(said.contains("Tailscale: true"), "{said}");
    // 0.4.1 made no admin key, and finding the install makes none.
    assert!(!said.contains("Admin key"), "{said}");
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["doctor"]).await;
    assert!(!ok, "{said}");
    assert!(
        said.contains(&format!("problem: this machine's setup: {found}")),
        "{said}"
    );
    assert!(!said.contains("admin key"), "{said}");
    assert!(
        said.contains("`kkt setup --adopt` with no terminal"),
        "{said}"
    );
    assert!(said.contains("this machine's kkt in Docker"), "{said}");

    // Upgrade explains, and changes nothing.
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["upgrade"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("no setup record"), "{said}");
    assert!(said.contains(&found), "{said}");
    assert!(said.contains("installs nothing"), "{said}");

    // No terminal and no --adopt: named, not adopted, nothing changed.
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["setup"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("kkt setup --adopt"), "{said}");
    assert!(said.contains("--mode"), "{said}");
    assert!(!home.join(".config/kkt/setup.toml").exists());

    let docker_calls = std::fs::read_to_string(&log).unwrap_or_default();
    for change in ["up -d", "pull", "down", "backup", "key show"] {
        assert!(!docker_calls.contains(change), "{docker_calls}");
    }
    assert!(!home.join(".config/kkt/docker-admin-key").exists());
}

#[tokio::test]
async fn an_unrecorded_docker_install_shows_its_admin_key_copy() {
    let scratch = empty_home("found-admin-scratch");
    let bin = fake_docker(&scratch, KEY_0_4_1);
    let log = scratch.join("docker.log");
    let home = temp_home("found-admin");
    left_by_0_4_1(
        &home,
        "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
         KKT_VERSION=0.5.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\n",
        KEY_0_4_1,
    );
    let admin_copy = home.join(".config/kkt/docker-admin-key");
    std::fs::write(&admin_copy, format!("{FAKE_ADMIN_KEY}\n")).expect("admin key copy");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&admin_copy, std::fs::Permissions::from_mode(0o600))
            .expect("private key");
    }

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(said.contains("Unrecorded install"), "{said}");
    assert!(
        said.contains(&format!("Admin key: {}", admin_copy.display())),
        "{said}"
    );
    assert!(said.contains("read with `kkt key show --admin`"), "{said}");
    assert!(said.contains("read with `kkt key show --web`"), "{said}");
    assert!(!said.contains(FAKE_ADMIN_KEY), "{said}");
    let (_, said) = kkt_with_docker(&home, &bin, &log, &["doctor"]).await;
    assert!(said.contains("docker-admin-key"), "{said}");
    assert!(!said.contains(FAKE_ADMIN_KEY), "{said}");

    // The commands status advertises read the host copies.
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["key", "show", "--admin"]).await;
    assert!(ok, "{said}");
    assert_eq!(said.trim(), FAKE_ADMIN_KEY);
    let web_copy = home.join(".config/kkt/docker-web-key");
    assert!(!web_copy.exists());
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["key", "show", "--web"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains(&web_copy.display().to_string()), "{said}");
    std::fs::write(&web_copy, "web-key-copy\n").expect("web key copy");
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["key", "show", "--web"]).await;
    assert!(ok, "{said}");
    assert_eq!(said.trim(), "web-key-copy");

    // Making a key there is refused: the server reads its keys in the
    // container. Nothing on the host changes.
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["key", "create", "--admin"]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("inside the container"), "{said}");
    assert_eq!(
        std::fs::read_to_string(&admin_copy).expect("admin copy"),
        format!("{FAKE_ADMIN_KEY}\n")
    );
    assert!(!home.join(".config/kkt/admin-key").exists());
    let docker_calls = std::fs::read_to_string(&log).unwrap_or_default();
    // A `key` argument, not the word inside a folder name.
    assert!(
        !docker_calls.split_whitespace().any(|word| word == "key"),
        "{docker_calls}"
    );
}

#[tokio::test]
async fn setup_adopts_a_docker_install_with_no_record_and_upgrade_then_works() {
    // A real server stands in for the container, on the port the managed
    // settings file says Docker publishes it on.
    let (_server_home, mut server, url, key) = keyed_server("adopt-server").await;
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .to_string();
    let key_text = std::fs::read_to_string(&key)
        .expect("key")
        .trim()
        .to_string();
    let scratch = empty_home("adopt-scratch");
    let bin = fake_docker(&scratch, &key_text);
    let log = scratch.join("docker.log");
    let home = temp_home("adopt-docker");
    let folder = left_by_0_4_1(
        &home,
        &format!(
            "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
             KKT_VERSION=0.4.1\nCOMPOSE_PROJECT_NAME=kkt-test\nKKT_PORT={port}\n"
        ),
        &key_text,
    );
    let key_file = home.join(".config/kkt/docker-access-key");
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().expect("dir")).expect("codex dir");
    std::fs::write(&codex, "").expect("codex config");

    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &["setup", "--adopt", "--clients", "codex"],
    )
    .await;
    assert!(ok, "{said}");
    assert!(said.contains("Found a kkt server in Docker from"), "{said}");
    assert!(said.contains("ok: key accepted"), "{said}");
    assert!(!said.contains(&key_text), "{said}");
    // Nothing installed or started again, and no Tailscale login.
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        format!(
            "compose --project-directory {} ps --all --quiet kkt",
            folder.display()
        )
    );
    // What setup 0.4.2 would have written, by this version.
    assert_eq!(
        record_text(&home),
        format!(
            "format = 1\nmode = \"docker\"\nwritten_by = \"{}\"\nurl = \"{url}\"\n\
             key_file = \"{}\"\n\n[docker]\nfolder = \"{}\"\ntailscale = false\n",
            env!("CARGO_PKG_VERSION"),
            key_file.display(),
            folder.display()
        )
    );
    let client_config = std::fs::read_to_string(&codex).expect("codex config");
    assert!(
        client_config.contains("docker-access-key"),
        "{client_config}"
    );
    assert!(!client_config.contains(&key_text));

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(!said.contains("Unrecorded"), "{said}");
    // Adopting makes no admin key.
    assert!(!home.join(".config/kkt/docker-admin-key").exists());

    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["upgrade"]).await;
    assert!(ok, "{said}");
    assert!(said.contains("Upgraded the server to kkt"), "{said}");
    assert!(!said.contains(&key_text), "{said}");
    assert!(!said.contains("web-key-0123"), "{said}");
    let docker_calls = std::fs::read_to_string(&log).expect("docker log");
    let backup = docker_calls.find("kkt backup").expect("backed up");
    let up = docker_calls.find("up -d").expect("started");
    assert!(backup < up, "{docker_calls}");
    let env = std::fs::read_to_string(folder.join(".env")).expect(".env");
    assert!(
        env.contains(&format!("KKT_VERSION={}\n", env!("CARGO_PKG_VERSION"))),
        "{env}"
    );
    assert!(
        std::fs::read_to_string(folder.join("compose.yaml"))
            .expect("compose file")
            .contains("services:"),
        "the Compose file is this version's"
    );
    assert!(home.join(".config/kkt/docker-web-key").is_file());
    server.kill().await.expect("stop kkt");
}

/// Makes `name` in `bin` a shell script running `body`.
fn script(bin: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(bin).expect("bin");
    let path = bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
    }
}

#[tokio::test]
async fn setup_adopts_an_installed_service_with_no_record() {
    // The service as `kkt service install` writes it, with a service
    // manager that says it runs. It is installed before its stand-in
    // server below starts, since install refuses an address another
    // program has; the service file names the config file, not the
    // address, so the address set afterwards is the one it would use.
    let home = temp_home("adopt-service");
    let config = home.join(".config/kkt/config.toml");
    std::fs::write(&config, "bind = \"127.0.0.1:0\"\n").expect("config");
    let scratch = empty_home("adopt-service-scratch");
    let bin = service_manager(&scratch);
    let log = scratch.join("docker.log");
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["service", "install"]).await;
    assert!(ok, "{said}");
    let service_file = installed_service_file(&home);
    let service_text = std::fs::read_to_string(&service_file).expect("service file");

    // The server the service runs: `kkt serve` with the config file's
    // address and the default key, as `kkt service install` has it.
    let (mut server, bind) = serving::start_quiet(|bind| {
        std::fs::write(&config, format!("bind = \"{bind}\"\n")).expect("config");
        let mut server = Command::new(KKT);
        server.arg("serve").own_home(&home);
        server
    })
    .await;

    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(
        said.contains("Unrecorded install: Found a kkt server as a service"),
        "{said}"
    );
    let (_, said) = kkt_with_docker(&home, &bin, &log, &["doctor"]).await;
    assert!(
        said.contains("ok: this machine's setup: Found a kkt server as a service"),
        "{said}"
    );

    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &["setup", "--adopt", "--clients", "none"],
    )
    .await;
    assert!(ok, "{said}");
    assert!(said.contains("ok: key accepted"), "{said}");
    let key = std::fs::read_to_string(home.join(".config/kkt/access-key")).expect("key");
    assert!(!said.contains(key.trim()), "{said}");
    let record = record_text(&home);
    assert!(record.contains("mode = \"service\""), "{record}");
    assert!(record.contains("written_by = \"unknown\""), "{record}");
    assert!(
        record.contains(&format!("url = \"http://{bind}/mcp\"")),
        "{record}"
    );
    assert!(record.contains("[service]\ntailscale = false"), "{record}");
    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(
        said.contains(&format!("Version: {}", env!("CARGO_PKG_VERSION"))),
        "{said}"
    );
    assert!(said.contains("Mode: service"), "{said}");
    assert!(!said.contains("Unrecorded"), "{said}");

    // Its version is not known, so upgrade backs up and installs it again.
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["upgrade"]).await;
    assert!(ok, "{said}");
    assert!(said.contains("Upgraded the server to kkt"), "{said}");
    assert!(!said.contains(key.trim()), "{said}");
    assert!(home.join(".config/kkt/backups").is_dir());
    assert!(
        record_text(&home).contains(&format!("written_by = \"{}\"", env!("CARGO_PKG_VERSION")))
    );
    // Installed again with the settings it had.
    assert_eq!(
        std::fs::read_to_string(&service_file).expect("service file"),
        service_text,
    );
    let (ok, said) = kkt_with_docker(&home, &bin, &log, &["upgrade"]).await;
    assert!(ok, "{said}");
    assert!(said.contains("already at kkt"), "{said}");
    server.kill().await.expect("stop kkt");
    // A listener accepts connections but never answers MCP. Status still
    // succeeds and prints the manager's output, within its probe deadline.
    let stalled = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listen");
    std::fs::write(
        &config,
        format!("bind = \"{}\"\n", stalled.local_addr().expect("address")),
    )
    .expect("config");
    // The normal HTTP timeout alone would wait five seconds.
    #[expect(
        clippy::disallowed_methods,
        reason = "the real status process must finish before the HTTP client's timeout"
    )]
    let (ok, said) = tokio::time::timeout(
        Duration::from_secs(4),
        kkt_with_docker(&home, &bin, &log, &["status"]),
    )
    .await
    .expect("bounded status probe");
    assert!(
        ok && said.contains("Mode: service") && said.contains("Version: could not be read"),
        "{said}"
    );

    // Answer as a different build from both the CLI and its setup record.
    tokio::join!(
        async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            drop(stalled.accept().await.expect("previous request"));
            let (mut stream, _) = stalled.accept().await.expect("next request");
            let mut lines = BufReader::new(&mut stream).lines();
            while let Some(line) = lines.next_line().await.expect("HTTP headers") {
                if line.is_empty() {
                    break;
                }
            }
            let body = r#"{"result":{"serverInfo":{"name":"kkt","version":"0.10.0"}}}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .expect("reply");
        },
        async {
            let (ok, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
            assert!(ok && said.contains("Version: 0.10.0"), "{said}");
        }
    );
}

/// A `bin` folder in `scratch` with a fake launchd and systemd that do what
/// they are asked and say the service runs.
fn service_manager(scratch: &Path) -> PathBuf {
    let bin = scratch.join("bin");
    script(
        &bin,
        "launchctl",
        "case \"$1\" in print) echo 'state = running' ;; esac\nexit 0",
    );
    script(&bin, "systemctl", "exit 0");
    script(&bin, "id", "echo 501");
    bin
}

/// The service file `kkt service install` writes under `home` on this
/// system.
fn installed_service_file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents/com.sam-mfb.kashkeshet.plist")
    } else {
        home.join(".config/systemd/user/kashkeshet.service")
    }
}

#[tokio::test]
async fn a_service_installed_with_other_settings_is_not_adopted() {
    // The default config file says one address; the service was installed
    // with another config file, which says another. `install` checks that
    // its server could listen on its address, so that one is port 0, which
    // the system gives freely: a fixed port fails the test whenever another
    // program on the machine, such as a second test run, holds it. The
    // default config file's address is never listened on.
    let home = temp_home("adopt-service-custom");
    std::fs::write(
        home.join(".config/kkt/config.toml"),
        "bind = \"127.0.0.1:17420\"\n",
    )
    .expect("config");
    let custom = home.join("custom.toml");
    std::fs::write(&custom, "bind = \"127.0.0.1:0\"\n").expect("custom config");
    let scratch = empty_home("adopt-service-custom-scratch");
    let bin = service_manager(&scratch);
    let log = scratch.join("docker.log");
    let custom_arg = custom.to_str().expect("utf-8 path");
    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &["service", "install", "--config", custom_arg],
    )
    .await;
    assert!(ok, "{said}");

    // Neither status nor setup takes the default config file's address
    // for it.
    let (_, said) = kkt_with_docker(&home, &bin, &log, &["status"]).await;
    assert!(said.contains("Found a kkt service, not recorded"), "{said}");
    assert!(said.contains(&format!("--config {custom_arg}")), "{said}");
    assert!(!said.contains("17420"), "{said}");
    let (ok, said) = kkt_with_docker(
        &home,
        &bin,
        &log,
        &["setup", "--adopt", "--clients", "none"],
    )
    .await;
    assert!(!ok, "{said}");
    assert!(
        said.contains("settings other than the default config file's"),
        "{said}"
    );
    assert!(!said.contains("17420/mcp"), "{said}");
    assert!(!home.join(".config/kkt/setup.toml").exists());
}

/// `--allow-kkt` lets each client set up always use kkt: Codex's server
/// entry approves its tools and a rules file allows `kkt watch`, and OpenCode
/// lists kkt's tools. Running setup again adds nothing twice.
#[tokio::test]
async fn allow_kkt_writes_the_clients_permission_rules_once() {
    let (_server_home, mut server, url, key) = keyed_server("allow-kkt").await;
    let home = temp_home("allow-kkt-client");
    let extra = ["--clients", "codex,opencode,muse-code", "--allow-kkt"];
    let (ok, said) = setup_client(&home, &url, &key, &extra).await;
    assert!(ok, "{said}");
    assert!(
        said.contains("Allowed kkt's tools and `kkt watch`"),
        "{said}"
    );
    assert!(said.contains("Muse Code has no setting"), "{said}");
    assert!(
        codex_config(&home).contains("default_tools_approval_mode = \"approve\""),
        "{}",
        codex_config(&home)
    );
    let rules = home.join(".codex/rules/kkt.rules");
    let read = |path: &Path| std::fs::read_to_string(path).expect("read");
    assert_eq!(
        read(&rules),
        "prefix_rule(pattern = [\"kkt\", \"watch\"], decision = \"allow\")\n"
    );
    let opencode = home.join(".config/opencode/opencode.json");
    let text = read(&opencode);
    assert!(text.contains("\"kkt watch *\": \"allow\""), "{text}");
    assert!(
        text.contains("\"kkt_wait_for_messages\": \"allow\""),
        "{text}"
    );

    let before = (read(&rules), read(&opencode));
    let (ok, said) = setup_client(&home, &url, &key, &extra).await;
    assert!(ok, "{said}");
    assert!(said.contains("Already allowed: "), "{said}");
    assert_eq!((read(&rules), read(&opencode)), before);
    // Connecting Codex again may move its entry's lines, but the setting
    // is there once.
    let codex = codex_config(&home);
    assert_eq!(
        codex.matches("default_tools_approval_mode").count(),
        1,
        "{codex}"
    );
    server.kill().await.expect("stop kkt");
}

/// Exercise both printed recovery commands in a shell, with the same data
/// an operator has after uninstall. A different client's config stays intact.
#[cfg(unix)]
#[tokio::test]
async fn clients_can_follow_the_printed_commands_after_uninstall() {
    let (_server_home, mut server, url, key) = keyed_server("uninstall-destination").await;
    let home = temp_home("uninstall-clients");
    let key_path = home.join("destination's key");
    std::fs::copy(&key, &key_path).unwrap();
    let bin = fake_docker(&home, KEY_0_4_1);
    std::os::unix::fs::symlink(KKT, bin.join("kkt")).unwrap();
    let removed = "http://127.0.0.1:7420/mcp";
    let folder = managed_folder(&home);
    std::fs::create_dir_all(&folder).unwrap();
    write_record(
        &home,
        &format!(
            "format = 1\nmode = \"docker\"\nwritten_by = \"0.10.0\"\nurl = \"{removed}\"\n[docker]\nfolder = \"{}\"\ntailscale = false\n",
            folder.display()
        ),
    );
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(&codex, format!("[mcp_servers.kkt]\nurl = '{removed}'\n")).unwrap();
    let other = home.join(".claude.json");
    let remote = r#"{"mcpServers":{"kkt":{"url":"https://other.example/mcp"}}}"#;
    std::fs::write(&other, remote).unwrap();

    let (ok, said) = kkt_with_path(&home, &bin, &["uninstall"]).await;
    assert!(
        ok && said.contains("cannot reach a server until moved: codex."),
        "{said}"
    );
    let command = said
        .lines()
        .find(|line| line.starts_with("kkt setup "))
        .unwrap();
    let run = |command: &str| {
        std::process::Command::new("/bin/sh")
            .args(["-c", command])
            .own_home(&home)
            .env("PATH", &bin)
            .env("NEW_URL", &url)
            .env("NEW_KEY_FILE", &key_path)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let output = run(command);
    assert!(output.status.success(), "{output:?}");
    assert!(codex_config(&home).contains(&url));
    assert_eq!(std::fs::read_to_string(&other).unwrap(), remote);

    std::fs::remove_file(home.join(".config/kkt/setup.toml")).unwrap();
    let (ok, said) = kkt_with_path(
        &home,
        &bin,
        &[
            "switch",
            &url,
            "--key-file",
            arg(&key_path),
            "--clients",
            "codex",
        ],
    )
    .await;
    assert!(!ok, "{said}");
    let command = said
        .split_once("run `")
        .unwrap()
        .1
        .strip_suffix("`\n")
        .unwrap();
    let output = run(command);
    assert!(output.status.success(), "{output:?}");
    assert!(record_text(&home).contains(&url));
    assert_eq!(std::fs::read_to_string(&other).unwrap(), remote);
    server.kill().await.unwrap();
}

#[tokio::test]
async fn a_retained_project_and_an_unreachable_docker_daemon_are_distinguished() {
    let home = temp_home("retained-project");
    left_by_0_4_1(
        &home,
        "KKT_VERSION=0.10.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\n",
        KEY_0_4_1,
    );
    let bin = home.join("bin");
    script(
        &bin,
        "docker",
        r#"case "$*" in *"ps --all --quiet kkt") exit 0;; *) exit 1;; esac"#,
    );
    let (_, status) = kkt_with_path(&home, &bin, &["status"]).await;
    assert!(
        status.contains("No setup record") && !status.contains("Unrecorded install"),
        "{status}"
    );
    let flags = [
        "setup",
        "--mode",
        "docker",
        "--no-tailscale",
        "--allow-host",
        "none",
        "--clients",
        "none",
    ];
    let (_, setup) = kkt_with_path(&home, &bin, &flags).await;
    assert!(
        setup.contains("Docker setup will reuse existing storage") && !setup.contains("adopt"),
        "{setup}"
    );

    std::fs::remove_file(home.join(".config/kkt/setup-unfinished.toml")).ok();
    script(&bin, "docker", "echo 'daemon unavailable' >&2; exit 1");
    for args in [&["status"][..], &flags[..]] {
        let (_, said) = kkt_with_path(&home, &bin, args).await;
        assert!(
            said.contains("could not check whether the Docker server container exists")
                && said.contains("daemon unavailable"),
            "{said}"
        );
        assert!(!said.contains("Found a kkt server"), "{said}");
    }
}

#[tokio::test]
async fn client_backup_with_only_db_skips_default_config_and_keys_and_lanes_export_works() {
    let (_server_home, mut server, url, key) = keyed_server("client-db-only").await;
    let home = temp_home("client-db-only-home");
    let (ok, said) = setup_client(&home, &url, &key, &["--clients", "none"]).await;
    assert!(ok, "{said}");

    // Plant unrelated leftover default config.toml and access-key on the client.
    let default_config = home.join(".config/kkt/config.toml");
    let default_access_key = home.join(".config/kkt/access-key");
    std::fs::write(&default_config, "bind = \"127.0.0.1:17999\"\n").expect("default config");
    std::fs::write(
        &default_access_key,
        "leftover-default-key-0123456789abcdef\n",
    )
    .expect("default key");

    // Create a local test database (with one lane) and a separate config file.
    let init_home = temp_home("client-db-only-init");
    starts(serve(&init_home, &[])).await;
    let local_dir = home.join("local-server");
    std::fs::create_dir_all(&local_dir).expect("local dir");
    let local_db = init_home.join("kkt.db");
    let local_cfg = local_dir.join("config.toml");
    let local_key = local_dir.join("access-key");
    std::fs::write(&local_key, "local-server-key-0123456789abcdef0123\n").expect("local key");
    std::fs::write(
        &local_cfg,
        format!(
            "db = \"{}\"\naccess_key_file = \"{}\"\n",
            local_db.display(),
            local_key.display()
        ),
    )
    .expect("local cfg");

    // Seed the local database by importing a lane with --config.
    let lanes_in = home.join("lanes-in");
    std::fs::create_dir_all(&lanes_in).expect("lanes-in");
    std::fs::write(
        lanes_in.join("builder.md"),
        "---\nname: builder\ndescription: Builds things\n---\nBuild it.\n",
    )
    .expect("lane file");
    let (ok, said) = kkt(
        &home,
        &[
            "lanes",
            "import",
            arg(&lanes_in),
            "--config",
            arg(&local_cfg),
        ],
    )
    .await;
    assert!(ok, "{said}");

    // V2: `kkt lanes export` with --config and --db works on a client; bare export refuses.
    let lanes_out_cfg = home.join("lanes-out-cfg");
    let (ok, said) = kkt(
        &home,
        &[
            "lanes",
            "export",
            arg(&lanes_out_cfg),
            "--config",
            arg(&local_cfg),
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(lanes_out_cfg.join("builder.md").exists());

    let lanes_out_db = home.join("lanes-out-db");
    let (ok, said) = kkt(
        &home,
        &[
            "lanes",
            "export",
            arg(&lanes_out_db),
            "--db",
            arg(&local_db),
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(lanes_out_db.join("builder.md").exists());

    let (ok, said) = kkt(&home, &["lanes", "export", arg(&home.join("lanes-bare"))]).await;
    assert!(!ok, "{said}");
    assert!(said.contains("this machine is a client"), "{said}");

    // V1a: `kkt backup DIR --db X` on a client copies only the database, not default config/keys.
    let db_only_backups = home.join("backups-db-only");
    let (ok, said) = kkt(
        &home,
        &["backup", arg(&db_only_backups), "--db", arg(&local_db)],
    )
    .await;
    assert!(ok, "{said}");
    assert!(
        said.contains("Backed up only the database: no --config was given on a client machine"),
        "{said}"
    );
    let saved = std::fs::read_dir(&db_only_backups)
        .expect("dir")
        .next()
        .expect("one backup")
        .expect("entry")
        .path();
    assert!(saved.join("kashkeshet.db").exists());
    assert!(!saved.join("config.toml").exists());
    assert!(!saved.join("access-key").exists());

    // V1b: `kkt backup DIR --config Y` copies Y and Y's key, never the client's default key.
    let cfg_backups = home.join("backups-cfg");
    let (ok, said) = kkt(
        &home,
        &["backup", arg(&cfg_backups), "--config", arg(&local_cfg)],
    )
    .await;
    assert!(ok, "{said}");
    let saved_cfg = std::fs::read_dir(&cfg_backups)
        .expect("dir")
        .next()
        .expect("one backup")
        .expect("entry")
        .path();
    assert!(saved_cfg.join("kashkeshet.db").exists());
    assert!(saved_cfg.join("config.toml").exists());
    assert_eq!(
        std::fs::read_to_string(saved_cfg.join("access-key")).expect("backed up key"),
        "local-server-key-0123456789abcdef0123\n"
    );

    // V1c: `--config` with no `access_key_file` on a client still does not fall back to default key.
    let keyless_cfg = local_dir.join("keyless.toml");
    std::fs::write(&keyless_cfg, format!("db = \"{}\"\n", local_db.display())).expect("keyless");
    let keyless_backups = home.join("backups-keyless");
    let (ok, said) = kkt(
        &home,
        &[
            "backup",
            arg(&keyless_backups),
            "--config",
            arg(&keyless_cfg),
        ],
    )
    .await;
    assert!(ok, "{said}");
    let saved_keyless = std::fs::read_dir(&keyless_backups)
        .expect("dir")
        .next()
        .expect("one backup")
        .expect("entry")
        .path();
    assert!(saved_keyless.join("config.toml").exists());
    assert!(!saved_keyless.join("access-key").exists());

    server.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn switch_local_reports_each_clients_real_previous_url() {
    let (_local_home, mut local, local_url, local_key) = keyed_server("switch-local-srv").await;
    let (_other_home, mut other, other_url, other_key) = keyed_server("switch-other-srv").await;
    let home = temp_home("switch-local-before");

    // Record a service pointing at local_url, while Codex's config actually points at other_url.
    let (ok, said) = setup_client(&home, &other_url, &other_key, &["--clients", "codex"]).await;
    assert!(ok, "{said}");
    std::fs::write(
        home.join(".config/kkt/setup.toml"),
        format!(
            "format = 1\nmode = \"service\"\nwritten_by = \"1.0.0\"\nurl = \"{local_url}\"\nkey_file = \"{}\"\n\n[service]\ntailscale = false\n",
            local_key.display()
        ),
    )
    .expect("service record");

    let (ok, said) = kkt(&home, &["switch", "--local"]).await;
    assert!(ok, "{said}");
    // Codex's line reports its real previous address (other_url), not local_url.
    assert!(said.contains(&format!("(before: {other_url})")), "{said}");
    assert!(!said.contains(&format!("(before: {local_url})")), "{said}");
    assert!(
        said.contains(&format!("This machine's clients now use {local_url}.")),
        "{said}"
    );

    local.kill().await.expect("stop local");
    other.kill().await.expect("stop other");
}

#[tokio::test]
async fn switch_omits_unsafe_previous_urls_and_still_switches() {
    let (_new_home, mut new_server, new_url, new_key) = keyed_server("switch-unsafe-new").await;
    let home = temp_home("switch-unsafe-before");

    // Record a client setup so `kkt switch` runs.
    let (ok, said) = setup_client(&home, &new_url, &new_key, &["--clients", "none"]).await;
    assert!(ok, "{said}");

    // Plant three client configs with unsafe previous URLs: credentials, query token, and ESC.
    let codex_path = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_path,
        "[mcp_servers.kkt]\nurl = \"http://alice:s3cr3t@old.example/mcp\"\n",
    )
    .unwrap();

    let opencode_path = home.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(opencode_path.parent().unwrap()).unwrap();
    std::fs::write(
        &opencode_path,
        r#"{"mcp":{"kkt":{"type":"remote","url":"https://old.example/mcp?token=secret_query_tok"}}}"#,
    )
    .unwrap();

    let agy_path = home.join(".gemini/antigravity/mcp_config.json");
    std::fs::create_dir_all(agy_path.parent().unwrap()).unwrap();
    std::fs::write(
        &agy_path,
        "{\"mcpServers\":{\"kkt\":{\"serverUrl\":\"https://old.example/mcp\\u001b[31m\"}}}",
    )
    .unwrap();

    let (ok, said) = kkt(
        &home,
        &[
            "switch",
            &new_url,
            "--key-file",
            arg(&new_key),
            "--clients",
            "codex,opencode,agy",
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(!said.contains("alice"), "{said}");
    assert!(!said.contains("s3cr3t"), "{said}");
    assert!(!said.contains("secret_query_tok"), "{said}");
    assert!(!said.contains('\x1b'), "{said}");
    assert!(!said.contains("(before:"), "{said}");
    assert!(codex_config(&home).contains(&new_url));
    assert!(
        std::fs::read_to_string(&opencode_path)
            .unwrap()
            .contains(&new_url)
    );
    assert!(
        std::fs::read_to_string(&agy_path)
            .unwrap()
            .contains(&new_url)
    );

    new_server.kill().await.expect("stop new");
}

#[tokio::test]
async fn switch_puts_previous_url_on_configured_line_for_keyed_agy_and_muse_code() {
    let (_new_home, mut new_server, new_url, new_key) = keyed_server("switch-multiline-new").await;
    let home = temp_home("switch-multiline-before");

    let (ok, said) = setup_client(&home, &new_url, &new_key, &["--clients", "none"]).await;
    assert!(ok, "{said}");

    let previous = "https://previous.example/mcp";
    let agy_path = home.join(".gemini/antigravity/mcp_config.json");
    std::fs::create_dir_all(agy_path.parent().unwrap()).unwrap();
    std::fs::write(
        &agy_path,
        format!(r#"{{"mcpServers":{{"kkt":{{"serverUrl":"{previous}"}}}}}}"#),
    )
    .unwrap();

    let muse_path = home.join(".config/muse/settings.json");
    std::fs::create_dir_all(muse_path.parent().unwrap()).unwrap();
    std::fs::write(
        &muse_path,
        format!(
            r#"{{"schema_version":1,"mcpServers":{{"kkt":{{"transport":"streamable_http","url":"{previous}"}}}}}}"#
        ),
    )
    .unwrap();

    let (ok, said) = kkt(
        &home,
        &[
            "switch",
            &new_url,
            "--key-file",
            arg(&new_key),
            "--clients",
            "agy,muse-code",
        ],
    )
    .await;
    assert!(ok, "{said}");
    let lines: Vec<&str> = said.lines().collect();
    assert!(
        lines
            .contains(&format!("Configured: {} (before: {previous})", agy_path.display()).as_str()),
        "{said}"
    );
    assert!(
        lines.contains(&"Antigravity stores the key in this file; permissions are 0600."),
        "{said}"
    );
    assert!(
        lines.contains(
            &format!("Configured: {} (before: {previous})", muse_path.display()).as_str()
        ),
        "{said}"
    );
    assert!(
        lines.contains(&"Muse Code stores the key in this file; permissions are 0600."),
        "{said}"
    );

    new_server.kill().await.expect("stop new");
}

#[tokio::test]
async fn switch_closing_line_names_only_the_clients_that_moved() {
    let (_local_home, mut local, local_url, local_key) = keyed_server("switch-subset-local").await;
    let (_other_home, mut other, other_url, other_key) = keyed_server("switch-subset-other").await;
    let home = temp_home("switch-subset-home");

    // Record a service at local_url while Claude Code, Codex and OpenCode all point at other_url.
    let (ok, said) = setup_client(&home, &other_url, &other_key, &["--clients", "none"]).await;
    assert!(ok, "{said}");
    std::fs::write(
        home.join(".config/kkt/setup.toml"),
        format!(
            "format = 1\nmode = \"service\"\nwritten_by = \"1.0.0\"\nurl = \"{local_url}\"\nkey_file = \"{}\"\n\n[service]\ntailscale = false\n",
            local_key.display()
        ),
    )
    .expect("service record");

    std::fs::write(
        home.join(".claude.json"),
        format!(r#"{{"mcpServers":{{"kkt":{{"url":"{other_url}"}}}}}}"#),
    )
    .unwrap();
    let codex_path = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_path,
        format!("[mcp_servers.kkt]\nurl = \"{other_url}\"\n"),
    )
    .unwrap();
    let opencode_path = home.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(opencode_path.parent().unwrap()).unwrap();
    std::fs::write(
        &opencode_path,
        format!(r#"{{"mcp":{{"kkt":{{"type":"remote","url":"{other_url}"}}}}}}"#),
    )
    .unwrap();

    // Switch only codex and opencode to --local; Claude Code stays on other_url.
    let (ok, said) = kkt(&home, &["switch", "--local", "--clients", "codex,opencode"]).await;
    assert!(ok, "{said}");
    assert!(
        said.contains(&format!("Codex and OpenCode now use {local_url}.")),
        "{said}"
    );
    assert!(!said.contains("This machine's clients now use"), "{said}");
    assert!(
        std::fs::read_to_string(home.join(".claude.json"))
            .unwrap()
            .contains(&other_url)
    );

    local.kill().await.expect("stop local");
    other.kill().await.expect("stop other");
}

#[cfg(unix)]
#[tokio::test]
async fn client_backup_requires_an_independent_regular_file_and_scans_past_gaps() {
    let original = "model = \"o3\"\n[mcp_servers.kkt]\nurl = \"https://old.example/mcp\"\n";

    // 1. Symlink .bak pointing at config.toml must NOT count as a backup.
    let home = temp_home("bak-symlink");
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(&codex, original).unwrap();
    let bak = home.join(".codex/config.toml.bak");
    std::os::unix::fs::symlink(&codex, &bak).unwrap();
    let (ok, said) = kkt(
        &home,
        &[
            "connect",
            "codex",
            "--url",
            "https://new.example/mcp",
            "--write",
        ],
    )
    .await;
    assert!(ok, "{said}");
    let bak1 = home.join(".codex/config.toml.bak.1");
    assert_eq!(
        std::fs::read_to_string(&bak1).expect("real independent backup created at .bak.1"),
        original
    );
    assert!(
        std::fs::read_to_string(&codex)
            .unwrap()
            .contains("https://new.example/mcp")
    );

    // 2. Hard link .bak to config.toml must NOT count as a backup.
    let home = temp_home("bak-hardlink");
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(&codex, original).unwrap();
    let bak = home.join(".codex/config.toml.bak");
    std::fs::hard_link(&codex, &bak).unwrap();
    let (ok, said) = kkt(
        &home,
        &[
            "connect",
            "codex",
            "--url",
            "https://new.example/mcp",
            "--write",
        ],
    )
    .await;
    assert!(ok, "{said}");
    let bak1 = home.join(".codex/config.toml.bak.1");
    assert_eq!(
        std::fs::read_to_string(&bak1).expect("real independent backup created at .bak.1"),
        original
    );

    // 3. Matching independent .bak.2 past a gap (no .bak) must NOT create a redundant .bak.
    let home = temp_home("bak-gap");
    let codex = home.join(".codex/config.toml");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(&codex, original).unwrap();
    std::fs::write(home.join(".codex/config.toml.bak.2"), original).unwrap();
    let (ok, said) = kkt(
        &home,
        &[
            "connect",
            "codex",
            "--url",
            "https://new.example/mcp",
            "--write",
        ],
    )
    .await;
    assert!(ok, "{said}");
    assert!(!home.join(".codex/config.toml.bak").exists());
    assert!(!home.join(".codex/config.toml.bak.1").exists());
    assert_eq!(
        std::fs::read_to_string(home.join(".codex/config.toml.bak.2")).unwrap(),
        original
    );

    // 4. Bare relative `--client-config config.toml` (empty parent) scans `.` past a gap.
    let home = temp_home("bak-relative-gap");
    let work = home.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("config.toml"), original).unwrap();
    std::fs::write(work.join("config.toml.bak.2"), original).unwrap();
    let out = tokio::process::Command::new(KKT)
        .args([
            "connect",
            "codex",
            "--client-config",
            "config.toml",
            "--url",
            "https://new.example/mcp",
            "--write",
        ])
        .current_dir(&work)
        .own_home(&home)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run kkt connect");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!work.join("config.toml.bak").exists());
    assert!(!work.join("config.toml.bak.1").exists());
    assert_eq!(
        std::fs::read_to_string(work.join("config.toml.bak.2")).unwrap(),
        original
    );
}

/// Matching an existing `.bak.2` past a gap works when the config file's own
/// name is not valid UTF-8. Linux only: macOS requires file names in UTF-8.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn client_backup_scans_past_gaps_for_non_utf8_config_name() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let home = temp_home("bak-non-utf8-gap");
    let dir = home.join("opencode");
    std::fs::create_dir_all(&dir).unwrap();
    let original = r#"{"mcp":{"kkt":{"type":"remote","url":"https://old.example/mcp"}}}"#;
    let config = dir.join(OsString::from_vec(b"opencode-\xff.json".to_vec()));
    let bak2 = dir.join(OsString::from_vec(b"opencode-\xff.json.bak.2".to_vec()));
    let bak = dir.join(OsString::from_vec(b"opencode-\xff.json.bak".to_vec()));
    std::fs::write(&config, original).unwrap();
    std::fs::write(&bak2, original).unwrap();

    let out = tokio::process::Command::new(KKT)
        .args(["connect", "opencode", "--client-config"])
        .arg(&config)
        .args(["--url", "https://new.example/mcp", "--write"])
        .own_home(&home)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run kkt connect");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!bak.exists(), "redundant .bak should not be created");
    assert_eq!(std::fs::read_to_string(&bak2).unwrap(), original);
}
