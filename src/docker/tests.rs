//! Docker setup against a fake host: nothing here runs Docker.

use super::files::{embedded, set_version};
use super::tailscale::{parse_status, split_name};
use super::*;
use crate::clock::testing::FixedClock;
use chrono::{TimeZone, Utc};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;

/// How a fake host answers a command, given as the text a person would type.
type Script = Box<dyn Fn(&str) -> io::Result<Finished>>;

/// A host whose commands are answered by a script, and which keeps what was
/// run, said and written. Every command takes three seconds on its clock,
/// longer than the poll interval, so waiting never really waits.
struct FakeHost {
    clock: Arc<FixedClock>,
    reply: Script,
    ran: RefCell<Vec<String>>,
    said: RefCell<Vec<String>>,
    files: RefCell<BTreeMap<PathBuf, (String, bool)>>,
    dirs: RefCell<Vec<PathBuf>>,
}

fn ok(stdout: &str) -> io::Result<Finished> {
    Ok(Finished {
        code: Some(0),
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn failed(stderr: &str) -> io::Result<Finished> {
    Ok(Finished {
        code: Some(1),
        stdout: String::new(),
        stderr: stderr.into(),
    })
}

impl FakeHost {
    fn new(reply: impl Fn(&str) -> io::Result<Finished> + 'static) -> FakeHost {
        FakeHost {
            clock: Arc::new(FixedClock::new(
                Utc.with_ymd_and_hms(2026, 10, 29, 12, 0, 0).unwrap(),
            )),
            reply: Box::new(reply),
            ran: RefCell::new(Vec::new()),
            said: RefCell::new(Vec::new()),
            files: RefCell::new(BTreeMap::new()),
            dirs: RefCell::new(Vec::new()),
        }
    }

    fn answer(&self, command: &Command) -> io::Result<Finished> {
        let text = command.to_string();
        self.ran.borrow_mut().push(text.clone());
        self.clock.advance(chrono::Duration::seconds(3));
        (self.reply)(&text)
    }

    /// The position of the first command run that contains `needle`.
    fn position(&self, needle: &str) -> Option<usize> {
        self.ran.borrow().iter().position(|c| c.contains(needle))
    }

    fn ran_any(&self, needle: &str) -> bool {
        self.position(needle).is_some()
    }

    fn file(&self, path: &str) -> Option<(String, bool)> {
        self.files.borrow().get(Path::new(path)).cloned()
    }
}

impl Host for FakeHost {
    fn run(&self, command: &Command) -> io::Result<Finished> {
        self.answer(command)
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        self.answer(command)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.dirs.borrow().iter().any(|d| d == path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.files
            .borrow()
            .get(path)
            .map(|(text, _)| text.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.dirs.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()> {
        // Like a real disk: the folder a file goes in must be there.
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let has_parent = self.dirs.borrow().contains(&parent)
            || self
                .files
                .borrow()
                .keys()
                .any(|f| f.parent() == Some(&parent));
        if !has_parent {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} has no folder", path.display()),
            ));
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), (contents.to_string(), private));
        Ok(())
    }

    fn say(&self, line: &str) {
        self.said.borrow_mut().push(line.to_string());
    }

    fn working_dir(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/home/me"))
    }
}

/// A login link that records what it was shown and what was opened, and
/// answers `open_requested` from a queue.
#[derive(Default)]
struct FakeLogin {
    shown: RefCell<Vec<String>>,
    opened: RefCell<Vec<String>>,
    /// What `open_requested` answers, one per ask; false when it runs out.
    requests: RefCell<std::collections::VecDeque<bool>>,
    open_fails: Cell<bool>,
    /// What `open_failure` answers once, after an open.
    later_failure: RefCell<Option<String>>,
}

impl LoginLink for FakeLogin {
    fn show(&self, url: &str) {
        self.shown.borrow_mut().push(url.to_string());
    }

    fn open_requested(&self) -> bool {
        self.requests.borrow_mut().pop_front().unwrap_or(false)
    }

    fn open(&self, url: &str) -> io::Result<()> {
        self.opened.borrow_mut().push(url.to_string());
        if self.open_fails.get() {
            Err(io::Error::new(io::ErrorKind::NotFound, "no opener"))
        } else {
            Ok(())
        }
    }

    fn open_failure(&self) -> Option<String> {
        self.later_failure.borrow_mut().take()
    }
}

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

fn folder() -> Folder {
    Folder(PathBuf::from("/data/docker"))
}

fn plain() -> Inputs {
    Inputs {
        version: "0.4.0".into(),
        project: "kkt".into(),
        port: 7420,
        allow_hosts: vec![],
        tailscale: None,
    }
}

fn with_tailscale(auth_key: Option<&str>) -> Inputs {
    Inputs {
        version: "0.4.0".into(),
        project: "kkt".into(),
        port: 7420,
        allow_hosts: vec![],
        tailscale: Some(Tailscale {
            hostname: "kkt".into(),
            auth_key: auth_key.map(str::to_string),
        }),
    }
}

/// Docker works, kkt makes its key, and `tailscale status` says `status`.
fn working(tailscale_status: &'static str) -> impl Fn(&str) -> io::Result<Finished> {
    move |command| {
        if command.contains("key show --web") {
            ok("web-key-0123456789abcdef0123456789abcdef\n")
        } else if command.contains("key show") {
            ok(&format!("{KEY}\n"))
        } else if command.contains("tailscale status") {
            ok(tailscale_status)
        } else {
            ok("")
        }
    }
}

const NEEDS_LOGIN: &str = r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/abc123","Self":{"DNSName":""}}"#;
const RUNNING: &str = r#"{"BackendState":"Running","AuthURL":"","Self":{"DNSName":"kkt-1.tail1234.ts.net."},"CertDomains":["kkt-1.tail1234.ts.net"]}"#;
const RUNNING_NO_HTTPS: &str =
    r#"{"BackendState":"Running","Self":{"DNSName":"kkt.tail1234.ts.net."},"CertDomains":null}"#;

fn read_repo(path: &str) -> String {
    std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn the_embedded_files_are_the_repositorys() {
    for tailscale in [false, true] {
        for (name, contents) in embedded(tailscale) {
            assert_eq!(contents, read_repo(name), "{name}");
        }
    }
}

#[test]
fn the_settings_file_is_pinned() {
    assert_eq!(
        env_text(&plain(), None),
        "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
         KKT_VERSION=0.4.0\n\
         COMPOSE_PROJECT_NAME=kkt\n\
         KKT_PORT=7420\n"
    );
    assert_eq!(
        env_text(&with_tailscale(None), None),
        "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
         KKT_VERSION=0.4.0\n\
         COMPOSE_PROJECT_NAME=kkt\n\
         KKT_PORT=7420\n\
         COMPOSE_FILE=compose.tailscale.yaml\n\
         TS_HOSTNAME=kkt\n\
         TS_TAILNET=pending.invalid\n"
    );
    assert_eq!(
        env_text(
            &with_tailscale(Some("tskey-auth-xyz")),
            Some(("kkt-1", "tail1234.ts.net"))
        ),
        "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
         KKT_VERSION=0.4.0\n\
         COMPOSE_PROJECT_NAME=kkt\n\
         KKT_PORT=7420\n\
         COMPOSE_FILE=compose.tailscale.yaml\n\
         TS_HOSTNAME=kkt-1\n\
         TS_TAILNET=tail1234.ts.net\n\
         TS_AUTHKEY=tskey-auth-xyz\n"
    );
}

#[test]
fn the_version_is_read_and_replaced_in_the_settings_file() {
    let env = env_text(&with_tailscale(None), None);
    assert_eq!(version_in_env(&env).as_deref(), Some("0.4.0"));
    let newer = set_version(&env, "0.5.0");
    assert_eq!(version_in_env(&newer).as_deref(), Some("0.5.0"));
    assert_eq!(newer.replace("0.5.0", "0.4.0"), env);
    assert_eq!(version_in_env("TS_HOSTNAME=kkt\n"), None);
    assert!(set_version("TS_HOSTNAME=kkt\n", "1.0.0").contains("KKT_VERSION=1.0.0"));
}

#[test]
fn tailscale_status_is_read_for_the_login_link_and_the_node() {
    let waiting = parse_status(NEEDS_LOGIN).unwrap();
    assert_eq!(waiting.backend_state, "NeedsLogin");
    assert_eq!(
        waiting.auth_url.as_deref(),
        Some("https://login.tailscale.com/a/abc123")
    );
    assert_eq!(waiting.dns_name, None);
    let running = parse_status(RUNNING).unwrap();
    assert_eq!(running.auth_url, None);
    assert_eq!(running.dns_name.as_deref(), Some("kkt-1.tail1234.ts.net"));
    assert_eq!(running.cert_domains, ["kkt-1.tail1234.ts.net"]);
    assert!(
        parse_status(RUNNING_NO_HTTPS)
            .unwrap()
            .cert_domains
            .is_empty()
    );
    assert!(parse_status("not json").is_err());
    assert!(parse_status("{}").is_err());
}

#[test]
fn a_node_name_splits_into_its_own_and_the_tailnets() {
    assert_eq!(
        split_name("kkt-1.tail1234.ts.net"),
        Some(("kkt-1".into(), "tail1234.ts.net".into()))
    );
    assert_eq!(split_name("kkt"), None);
    assert_eq!(split_name(".ts.net"), None);
}

#[tokio::test]
async fn a_local_setup_writes_the_files_starts_kkt_and_saves_the_key_privately() {
    let host = FakeHost::new(working(""));
    let done = provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &plain(),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert_eq!(done, Provisioned { node: None });
    assert_eq!(
        host.file("/data/docker/compose.yaml").unwrap(),
        (read_repo("compose.yaml"), false)
    );
    assert_eq!(
        host.file("/data/docker/.env").unwrap(),
        (env_text(&plain(), None), true)
    );
    assert_eq!(
        host.file("/keys/docker").unwrap(),
        (format!("{KEY}\n"), true)
    );
    assert!(
        host.dirs
            .borrow()
            .contains(&PathBuf::from("/data/docker/backups"))
    );
    // The backups folder is handed to the image's user before kkt starts.
    let own = host
        .position("--entrypoint chown kkt 10001 /backups")
        .unwrap();
    let up = host.position("up -d").unwrap();
    assert!(own < up, "{:?}", host.ran.borrow());
    assert!(
        host.ran
            .borrow()
            .iter()
            .all(|c| c == "docker compose version"
                || c.starts_with("docker compose --project-directory /data/docker ")),
        "{:?}",
        host.ran.borrow()
    );
    assert!(!host.ran_any("tailscale"));
}

#[tokio::test]
async fn a_tailscale_setup_shows_the_login_link_once_and_uses_the_nodes_own_name() {
    let polls = Cell::new(0);
    let host = FakeHost::new(move |command| {
        if command.contains("tailscale status") {
            polls.set(polls.get() + 1);
            return ok(if polls.get() < 4 {
                NEEDS_LOGIN
            } else {
                RUNNING
            });
        }
        working("")(command)
    });
    let login = FakeLogin::default();
    let done = provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert_eq!(
        done.node,
        Some(("kkt-1".to_string(), "tail1234.ts.net".to_string()))
    );
    assert_eq!(
        *login.shown.borrow(),
        ["https://login.tailscale.com/a/abc123"]
    );
    // Nothing is opened unless asked.
    assert!(login.opened.borrow().is_empty());
    // The real names go into the settings file, which holds no auth key.
    let env = host.file("/data/docker/.env").unwrap();
    assert_eq!(
        env,
        (
            env_text(&with_tailscale(None), Some(("kkt-1", "tail1234.ts.net"))),
            true
        )
    );
    // Tailscale alone first, then everything once its name is known.
    let alone = host.position("up -d tailscale").unwrap();
    let all = host
        .ran
        .borrow()
        .iter()
        .rposition(|c| c.ends_with(" up -d"))
        .unwrap();
    assert!(alone < host.position("tailscale status").unwrap());
    assert!(host.position("tailscale status").unwrap() < all);
    assert!(host.file("/data/docker/compose.tailscale.yaml").is_some());
    assert!(host.file("/data/docker/tailscale/serve.json").is_some());
}

#[tokio::test]
async fn an_auth_key_goes_only_in_the_private_settings_file() {
    let host = FakeHost::new(working(RUNNING));
    provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &with_tailscale(Some("tskey-auth-xyz")),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    let (env, private) = host.file("/data/docker/.env").unwrap();
    assert!(private);
    assert!(env.contains("TS_AUTHKEY=tskey-auth-xyz"));
    assert!(host.said.borrow().iter().all(|l| !l.contains("tskey")));
    assert!(host.ran.borrow().iter().all(|c| !c.contains("tskey")));
}

#[tokio::test]
async fn disabled_https_stops_setup_before_kkt_starts() {
    let host = FakeHost::new(working(RUNNING_NO_HTTPS));
    let err = provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DockerError::HttpsDisabled), "{err}");
    assert!(err.to_string().contains("Enable HTTPS"));
    assert!(!host.ran.borrow().iter().any(|c| c.ends_with(" up -d")));
    assert!(host.file("/keys/docker").is_none());
}

#[tokio::test]
async fn waiting_for_a_login_gives_up_after_the_deadline() {
    let host = FakeHost::new(working(NEEDS_LOGIN));
    let err = provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DockerError::LoginTimeout), "{err}");
    assert!(!host.ran.borrow().iter().any(|c| c.ends_with(" up -d")));
    assert!(host.file("/keys/docker").is_none());
}

#[tokio::test]
async fn kkt_that_never_makes_a_key_is_a_start_timeout() {
    let host = FakeHost::new(|command| {
        if command.contains("key show") {
            failed("no such container")
        } else {
            ok("")
        }
    });
    let err = provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &plain(),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DockerError::StartTimeout), "{err}");
    assert!(host.file("/keys/docker").is_none());
}

#[tokio::test]
async fn missing_docker_is_said_plainly_and_nothing_is_written() {
    let host = FakeHost::new(|_| Err(io::Error::new(io::ErrorKind::NotFound, "no such file")));
    let err = provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &plain(),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DockerError::NotAvailable(_)), "{err}");
    assert!(err.to_string().contains("Install Docker"));
    assert!(host.files.borrow().is_empty());
}

#[tokio::test]
async fn a_failing_command_is_an_error_naming_it() {
    let host = FakeHost::new(|command| {
        if command.contains("--entrypoint chown") {
            failed("pull access denied")
        } else {
            ok("")
        }
    });
    let err = provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &plain(),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("pull access denied"), "{err}");
    assert!(err.to_string().contains("--entrypoint chown"), "{err}");
}

fn installed(env: &str) -> FakeHost {
    let host = FakeHost::new(|command| {
        if command.contains("kkt backup") {
            ok("Backed up to /backups/kkt-20261029T120000.000Z-v0.4.0\n")
        } else if command.contains("ps --status running") {
            ok("kkt\ntailscale\n")
        } else {
            ok("")
        }
    });
    host.files
        .borrow_mut()
        .insert(PathBuf::from("/data/docker/.env"), (env.to_string(), true));
    host
}

#[test]
fn status_says_whether_it_is_set_up_and_running() {
    let host = FakeHost::new(|_| ok(""));
    assert_eq!(
        status(&host, &folder()).unwrap(),
        State {
            files_present: false,
            running: false,
            version: None,
            tailscale: false,
            login: None
        }
    );
    assert!(host.ran.borrow().is_empty());

    let host = installed(&env_text(&with_tailscale(None), None));
    assert_eq!(
        status(&host, &folder()).unwrap(),
        State {
            files_present: true,
            running: true,
            version: Some("0.4.0".into()),
            tailscale: true,
            // This fake host's Tailscale container answers nothing readable.
            login: Some(Login::Other("an unreadable answer".into()))
        }
    );
    let stopped = FakeHost::new(|_| ok(""));
    stopped.files.borrow_mut().insert(
        PathBuf::from("/data/docker/.env"),
        (env_text(&plain(), None), true),
    );
    let state = status(&stopped, &folder()).unwrap();
    assert!(state.files_present && !state.running && !state.tailscale);
}

#[test]
fn a_backup_is_copied_to_the_host_folder_it_was_asked_for() {
    let host = installed(&env_text(&plain(), None));
    let made = backup(&host, &folder(), Some(Path::new("/home/me/kkt-backups"))).unwrap();
    assert_eq!(
        made,
        PathBuf::from("/home/me/kkt-backups/kkt-20261029T120000.000Z-v0.4.0")
    );
    let ran = host.ran.borrow();
    assert!(
        ran[0].ends_with("exec -T kkt kkt backup /backups"),
        "{ran:?}"
    );
    assert!(
        ran[1].ends_with("cp kkt:/backups/kkt-20261029T120000.000Z-v0.4.0 /home/me/kkt-backups"),
        "{ran:?}"
    );
}

#[test]
fn a_backup_without_a_destination_stays_in_the_managed_folder() {
    let host = installed(&env_text(&plain(), None));
    let made = backup(&host, &folder(), None).unwrap();
    assert_eq!(
        made,
        folder().backups().join("kkt-20261029T120000.000Z-v0.4.0")
    );
    let ran = host.ran.borrow();
    assert_eq!(ran.len(), 1);
    assert!(ran[0].ends_with("exec -T kkt kkt backup /backups"));
}

#[test]
fn a_failed_backup_copies_nothing() {
    let host = FakeHost::new(|command| {
        if command.contains("kkt backup") {
            failed("disk full")
        } else {
            ok("")
        }
    });
    let err = backup(&host, &folder(), Some(Path::new("/home/me/b"))).unwrap_err();
    assert!(err.to_string().contains("disk full"), "{err}");
    assert!(!host.ran_any(" cp "));
}

/// Upgrades the fake install in [`folder`] to 0.5.0, with the access key's
/// host copy at `/keys/docker-access-key`.
async fn upgrade_to_0_5_0(host: &FakeHost) -> Result<(), DockerError> {
    upgrade(
        host,
        &*host.clock,
        &folder(),
        "0.5.0",
        Some(Path::new("/keys/docker-access-key")),
    )
    .await
}

#[tokio::test]
async fn a_failed_image_pull_restores_the_selected_version_for_a_retry() {
    let original = env_text(&plain(), None);
    let host = FakeHost::new(|command| {
        if command.ends_with(" pull") {
            failed("registry unavailable")
        } else if command.contains("kkt backup") {
            ok("Backed up to /backups/kkt-before-upgrade\n")
        } else {
            ok("")
        }
    });
    host.files
        .borrow_mut()
        .insert(folder().path().join(ENV_FILE), (original.clone(), true));
    assert!(
        upgrade_to_0_5_0(&host)
            .await
            .unwrap_err()
            .to_string()
            .contains("registry unavailable")
    );
    assert_eq!(
        host.files
            .borrow()
            .get(&folder().path().join(ENV_FILE))
            .unwrap()
            .0,
        original
    );
    assert!(!host.ran_any(" up -d"));
    assert!(!host.ran_any("key show"));
    assert!(host.file("/keys/docker-web-key").is_none());
}

#[tokio::test]
async fn an_upgrade_backs_up_first_then_moves_to_the_new_version() {
    let host = installed(&env_text(
        &with_tailscale(Some("tskey-auth-xyz")),
        Some(("kkt", "t.ts.net")),
    ));
    upgrade_to_0_5_0(&host).await.unwrap();
    let backup = host.position("kkt backup").unwrap();
    let pull = host.position(" pull").unwrap();
    let up = host.position(" up -d").unwrap();
    assert!(backup < pull && pull < up, "{:?}", host.ran.borrow());
    let (env, private) = host.file("/data/docker/.env").unwrap();
    assert!(private);
    assert_eq!(version_in_env(&env).as_deref(), Some("0.5.0"));
    // Everything else in the settings file is kept, the auth key included.
    assert!(env.contains("TS_AUTHKEY=tskey-auth-xyz") && env.contains("TS_TAILNET=t.ts.net"));
    assert_eq!(
        host.file("/data/docker/compose.tailscale.yaml").unwrap().0,
        read_repo("compose.tailscale.yaml")
    );
}

/// An install as `kkt setup` left it before the web key: the access key's
/// host copy and no web key, with a kkt container that has both keys once
/// the new version starts, or never has a web key if `web_key_appears` is
/// false.
fn installed_before_the_web_key(web_key_appears: bool) -> FakeHost {
    let host = FakeHost::new(move |command| {
        if command.contains("kkt backup") {
            ok("Backed up to /backups/kkt-20261029T120000.000Z-v0.4.2\n")
        } else if command.contains("key show --web") {
            if web_key_appears {
                ok("web-key-0123456789abcdef0123456789abcdef\n")
            } else {
                failed("no web key")
            }
        } else if command.contains("key show") {
            ok(&format!("{KEY}\n"))
        } else {
            ok("")
        }
    });
    let mut old = plain();
    old.version = "0.4.2".into();
    host.files
        .borrow_mut()
        .insert(folder().path().join(ENV_FILE), (env_text(&old, None), true));
    host.files.borrow_mut().insert(
        PathBuf::from("/keys/docker-access-key"),
        (format!("{KEY}\n"), true),
    );
    host
}

#[tokio::test]
async fn an_upgrade_saves_the_web_keys_host_copy_privately_once_kkt_has_started() {
    let host = installed_before_the_web_key(true);
    upgrade_to_0_5_0(&host).await.unwrap();
    assert_eq!(
        host.file("/keys/docker-web-key"),
        Some(("web-key-0123456789abcdef0123456789abcdef\n".into(), true))
    );
    // The access key's copy is saved again the same way.
    assert_eq!(
        host.file("/keys/docker-access-key"),
        Some((format!("{KEY}\n"), true))
    );
    let up = host.position(" up -d").unwrap();
    let web = host.position("key show --web").unwrap();
    assert!(up < web, "{:?}", host.ran.borrow());
    let said = host.said.borrow().join("\n");
    assert!(
        said.contains("The web key is saved in /keys/docker-web-key"),
        "{said}"
    );
    assert!(
        !said.contains("web-key-0123") && !said.contains(KEY),
        "{said}"
    );
    // Neither key is put in a command's arguments.
    assert!(
        !host
            .ran
            .borrow()
            .iter()
            .any(|c| c.contains("web-key-0123") || c.contains(KEY))
    );
}

#[tokio::test]
async fn an_upgrade_whose_web_key_never_appears_says_how_to_get_it_and_still_succeeds() {
    let host = installed_before_the_web_key(false);
    upgrade_to_0_5_0(&host).await.unwrap();
    assert!(host.file("/keys/docker-web-key").is_none());
    let (env, _) = host.file("/data/docker/.env").unwrap();
    assert_eq!(version_in_env(&env).as_deref(), Some("0.5.0"));
    // It waited as long as setup does before giving up.
    let asked = host
        .ran
        .borrow()
        .iter()
        .filter(|c| c.contains("key show --web"))
        .count();
    assert!(asked > 1, "{:?}", host.ran.borrow());
    let said = host.said.borrow().join("\n");
    assert!(
        said.contains("kkt is running, but its keys could not be saved"),
        "{said}"
    );
    assert!(said.contains("run `kkt setup` again"), "{said}");
    assert!(
        said.contains("`docker compose exec kkt kkt key show --web` in /data/docker"),
        "{said}"
    );
    assert!(!said.contains(KEY), "{said}");
}

#[tokio::test]
async fn an_upgrade_with_no_access_key_copy_saves_no_keys() {
    let host = installed_before_the_web_key(true);
    upgrade(&host, &*host.clock, &folder(), "0.5.0", None)
        .await
        .unwrap();
    assert!(!host.ran_any("key show"));
    assert!(host.file("/keys/docker-web-key").is_none());
}

#[tokio::test]
async fn an_upgrade_whose_backup_fails_changes_nothing() {
    let host = FakeHost::new(|command| {
        if command.contains("kkt backup") {
            failed("disk full")
        } else {
            ok("")
        }
    });
    let env = env_text(&plain(), None);
    host.files
        .borrow_mut()
        .insert(PathBuf::from("/data/docker/.env"), (env.clone(), true));
    assert!(upgrade_to_0_5_0(&host).await.is_err());
    assert_eq!(host.file("/data/docker/.env").unwrap().0, env);
    assert!(!host.ran_any(" pull") && !host.ran_any(" up "));
}

#[test]
fn down_keeps_the_data_unless_asked_to_remove_it() {
    let host = FakeHost::new(|_| ok(""));
    down(&host, &folder(), false).unwrap();
    down(&host, &folder(), true).unwrap();
    let ran = host.ran.borrow();
    assert!(ran[0].ends_with(" down"), "{ran:?}");
    assert!(ran[1].ends_with(" down --volumes"), "{ran:?}");
}

#[cfg(unix)]
#[test]
fn a_private_file_is_made_private_even_over_a_looser_one() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kkt-docker-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("key");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    OsHost.write_file(&path, "new", true).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(mode(&path), 0o600);
    let fresh = dir.join("fresh");
    OsHost.write_file(&fresh, "x", true).unwrap();
    assert_eq!(mode(&fresh), 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_test_setup_can_use_its_own_project_name_and_port() {
    let beside = Inputs {
        project: "kkt-test".into(),
        port: 7421,
        ..plain()
    };
    let env = env_text(&beside, None);
    assert!(env.contains("COMPOSE_PROJECT_NAME=kkt-test\n"), "{env}");
    assert!(env.contains("KKT_PORT=7421\n"), "{env}");
}

#[test]
fn extra_host_names_go_into_the_settings_file_as_one_list() {
    let extra = Inputs {
        allow_hosts: vec!["host.docker.internal".into(), "box.example".into()],
        ..with_tailscale(None)
    };
    let env = env_text(&extra, None);
    assert!(
        env.contains("KKT_ALLOW_HOSTS=host.docker.internal,box.example\n"),
        "{env}"
    );
    // Upgrading changes only the version, so the extra names stay.
    let upgraded = super::files::set_version(&env, "0.5.0");
    assert!(
        upgraded.contains("KKT_ALLOW_HOSTS=host.docker.internal,box.example\n"),
        "{upgraded}"
    );
    // With none, the line is left out and the Compose files' default applies.
    assert!(!env_text(&plain(), None).contains("KKT_ALLOW_HOSTS"));
}

#[test]
fn commands_run_without_the_variables_that_would_override_the_settings_file() {
    let command = super::compose::compose(&folder(), &["up", "-d"]);
    for name in [
        "COMPOSE_FILE",
        "COMPOSE_PROJECT_NAME",
        "KKT_VERSION",
        "KKT_PORT",
        "TS_HOSTNAME",
        "TS_TAILNET",
        "TS_AUTHKEY",
    ] {
        assert!(command.unset.iter().any(|v| v == name), "{name}");
    }
    assert!(
        super::compose::version()
            .unset
            .iter()
            .any(|v| v == "COMPOSE_FILE")
    );
}

#[cfg(unix)]
#[test]
fn the_real_host_leaves_unset_variables_out_of_a_commands_environment() {
    // Cargo sets CARGO_MANIFEST_DIR for every test run.
    let print = |unset: Vec<String>| {
        OsHost
            .run(&Command {
                program: "sh",
                args: vec![
                    "-c".into(),
                    "printf %s \"${CARGO_MANIFEST_DIR-unset}\"".into(),
                ],
                unset,
                current_dir: None,
            })
            .unwrap()
            .stdout
    };
    assert_ne!(print(vec![]), "unset");
    assert_eq!(print(vec!["CARGO_MANIFEST_DIR".into()]), "unset");
}

#[test]
fn status_does_not_call_a_container_stopped_when_compose_failed() {
    let host = FakeHost::new(|command| {
        if command.contains("ps --status") {
            failed("Cannot connect to the Docker daemon")
        } else {
            ok("")
        }
    });
    host.files.borrow_mut().insert(
        PathBuf::from("/data/docker/.env"),
        (env_text(&plain(), None), true),
    );
    let err = status(&host, &folder()).unwrap_err();
    assert!(
        err.to_string()
            .contains("Cannot connect to the Docker daemon"),
        "{err}"
    );
}

#[tokio::test]
async fn both_keys_go_in_a_folder_that_did_not_exist_yet() {
    let host = FakeHost::new(working(""));
    provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &plain(),
        Path::new("/home/me/.config/kkt/docker-access-key"),
    )
    .await
    .unwrap();
    assert!(
        host.dirs
            .borrow()
            .contains(&PathBuf::from("/home/me/.config/kkt"))
    );
    assert_eq!(
        host.file("/home/me/.config/kkt/docker-access-key").unwrap(),
        (format!("{KEY}\n"), true)
    );
    assert_eq!(
        host.file("/home/me/.config/kkt/docker-web-key").unwrap(),
        ("web-key-0123456789abcdef0123456789abcdef\n".into(), true)
    );
}

#[test]
fn compose_commands_run_in_the_managed_folder() {
    // Compose finds a relative COMPOSE_FILE from where it runs.
    let command = super::compose::compose(&folder(), &["up", "-d"]);
    assert_eq!(command.current_dir, Some(PathBuf::from("/data/docker")));
    assert_eq!(super::compose::version().current_dir, None);
}

#[cfg(unix)]
#[test]
fn the_real_host_runs_a_command_in_the_folder_it_names() {
    let dir = std::env::temp_dir().join(format!("kkt-docker-cwd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let here = OsHost
        .run(&Command {
            program: "pwd",
            args: vec!["-P".into()],
            unset: Vec::new(),
            current_dir: Some(dir.clone()),
        })
        .unwrap()
        .stdout;
    assert_eq!(
        Path::new(here.trim()),
        std::fs::canonicalize(&dir).unwrap().as_path()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_backup_folder_typed_relative_is_taken_from_where_the_person_is() {
    let host = installed(&env_text(&plain(), None));
    let made = backup(&host, &folder(), Some(Path::new("export"))).unwrap();
    assert_eq!(
        made,
        PathBuf::from("/home/me/export/kkt-20261029T120000.000Z-v0.4.0")
    );
    assert!(
        host.dirs
            .borrow()
            .contains(&PathBuf::from("/home/me/export"))
    );
    let ran = host.ran.borrow();
    assert!(ran[1].ends_with(" /home/me/export"), "{ran:?}");
}

#[tokio::test]
async fn the_link_is_opened_when_asked_and_setup_goes_on_watching_for_the_login() {
    let polls = Cell::new(0);
    let host = FakeHost::new(move |command| {
        if command.contains("tailscale status") {
            polls.set(polls.get() + 1);
            return ok(if polls.get() < 6 {
                NEEDS_LOGIN
            } else {
                RUNNING
            });
        }
        working("")(command)
    });
    let login = FakeLogin::default();
    // Not at first; then asked once; then the person logs in on their own.
    login
        .requests
        .borrow_mut()
        .extend([false, false, true, false]);
    let done = provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert!(done.node.is_some());
    assert_eq!(
        *login.opened.borrow(),
        ["https://login.tailscale.com/a/abc123"]
    );
    assert!(
        host.said
            .borrow()
            .iter()
            .any(|l| l.contains("Opening the link"))
    );
}

#[tokio::test]
async fn a_link_that_cannot_be_opened_is_said_and_setup_carries_on() {
    let polls = Cell::new(0);
    let host = FakeHost::new(move |command| {
        if command.contains("tailscale status") {
            polls.set(polls.get() + 1);
            return ok(if polls.get() < 4 {
                NEEDS_LOGIN
            } else {
                RUNNING
            });
        }
        working("")(command)
    });
    let login = FakeLogin::default();
    login.requests.borrow_mut().extend([true]);
    login.open_fails.set(true);
    provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    let said = host.said.borrow();
    assert!(
        said.iter()
            .any(|l| l.contains("Could not open a browser") && l.contains("no opener")),
        "{said:?}"
    );
    assert_eq!(login.shown.borrow().len(), 1);
}

#[tokio::test]
async fn no_link_is_shown_when_no_login_is_needed() {
    let host = FakeHost::new(working(RUNNING));
    let login = FakeLogin::default();
    provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(Some("tskey-auth-xyz")),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert!(login.shown.borrow().is_empty());
}

#[tokio::test]
async fn an_opener_that_fails_after_starting_is_said_while_setup_goes_on_waiting() {
    let polls = Cell::new(0);
    let host = FakeHost::new(move |command| {
        if command.contains("tailscale status") {
            polls.set(polls.get() + 1);
            return ok(if polls.get() < 5 {
                NEEDS_LOGIN
            } else {
                RUNNING
            });
        }
        working("")(command)
    });
    let login = FakeLogin::default();
    login.requests.borrow_mut().extend([true]);
    *login.later_failure.borrow_mut() = Some("the opener ended with exit status: 7".into());
    provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    let said = host.said.borrow();
    assert!(
        said.iter().any(|l| l.contains("Opening the link")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.contains("Could not open a browser") && l.contains("exit status: 7")),
        "{said:?}"
    );
}

const NEEDS_LOGIN_NO_LINK: &str =
    r#"{"BackendState":"NeedsLogin","AuthURL":"","Self":{"DNSName":""}}"#;

fn tailscale_env() -> String {
    env_text(&with_tailscale(None), Some(("kkt", "tail1.ts.net")))
}

/// A machine set up with Tailscale, whose Tailscale container answers
/// `tailscale status --json` with `status`.
fn with_tailscale_status(status: Option<&'static str>) -> FakeHost {
    let host = FakeHost::new(move |command| {
        if command.contains("tailscale status --json") {
            return match status {
                Some(json) => ok(json),
                None => failed("no such container"),
            };
        }
        if command.contains("ps --status running") {
            return ok("kkt\ntailscale\n");
        }
        ok("")
    });
    host.files
        .borrow_mut()
        .insert(PathBuf::from("/data/docker/.env"), (tailscale_env(), true));
    host
}

#[test]
fn status_reports_the_tailscale_login() {
    for (status, expected) in [
        (Some(RUNNING), Login::LoggedIn),
        (Some(NEEDS_LOGIN_NO_LINK), Login::NeedsLogin),
        (None, Login::Unreachable),
        (
            Some(r#"{"BackendState":"Stopped"}"#),
            Login::Other("Stopped".into()),
        ),
        (Some("garbage"), Login::Other("an unreadable answer".into())),
    ] {
        let host = with_tailscale_status(status);
        assert_eq!(status_of(&host).login, Some(expected), "{status:?}");
    }
}

fn status_of(host: &FakeHost) -> State {
    status(host, &folder()).unwrap()
}

#[test]
fn a_setup_without_tailscale_has_no_login_to_report() {
    let host = installed(&env_text(&plain(), None));
    assert_eq!(status_of(&host).login, None);
    assert!(!host.ran_any("tailscale status"));
}

#[tokio::test]
async fn a_machine_that_needs_a_login_again_gets_its_container_restarted_for_a_link() {
    // Logged out with no link until the container is restarted, as after an
    // expired key or a `tailscale logout`.
    let restarted = Cell::new(false);
    let after_restart = Cell::new(0);
    let host = FakeHost::new(move |command| {
        if command.contains("restart tailscale") {
            restarted.set(true);
            return ok("");
        }
        if command.contains("tailscale status") {
            if !restarted.get() {
                return ok(NEEDS_LOGIN_NO_LINK);
            }
            after_restart.set(after_restart.get() + 1);
            return ok(if after_restart.get() < 3 {
                NEEDS_LOGIN
            } else {
                RUNNING
            });
        }
        working("")(command)
    });
    let login = FakeLogin::default();
    let done = provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert!(done.node.is_some());
    let restarts = host
        .ran
        .borrow()
        .iter()
        .filter(|c| c.ends_with(" restart tailscale"))
        .count();
    assert_eq!(restarts, 1, "{:?}", host.ran.borrow());
    assert_eq!(
        *login.shown.borrow(),
        ["https://login.tailscale.com/a/abc123"]
    );
    assert!(
        host.said
            .borrow()
            .iter()
            .any(|l| l.contains("log in again"))
    );
}

#[tokio::test]
async fn a_new_machine_whose_link_comes_within_a_few_seconds_is_not_restarted() {
    let polls = Cell::new(0);
    let host = FakeHost::new(move |command| {
        if command.contains("tailscale status") {
            polls.set(polls.get() + 1);
            return ok(match polls.get() {
                1 | 2 => NEEDS_LOGIN_NO_LINK,
                3 | 4 => NEEDS_LOGIN,
                _ => RUNNING,
            });
        }
        working("")(command)
    });
    provision(
        &host,
        &*host.clock,
        &FakeLogin::default(),
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert!(!host.ran_any(" restart "));
}

#[tokio::test]
async fn a_machine_that_is_already_logged_in_is_not_restarted_or_shown_a_link() {
    let host = FakeHost::new(working(RUNNING));
    let login = FakeLogin::default();
    provision(
        &host,
        &*host.clock,
        &login,
        &folder(),
        &with_tailscale(None),
        Path::new("/keys/docker"),
    )
    .await
    .unwrap();
    assert!(!host.ran_any(" restart "));
    assert!(login.shown.borrow().is_empty());
}

/// A host whose kkt container answers `key show --web` with `answer`, and
/// whose web key's host copy holds `copy`.
fn with_web_key(answer: fn() -> io::Result<Finished>, copy: &str) -> FakeHost {
    let host = FakeHost::new(move |command| {
        if command.contains("key show --web") {
            answer()
        } else {
            ok("")
        }
    });
    host.files.borrow_mut().insert(
        PathBuf::from("/keys/docker-web-key"),
        (copy.to_string(), true),
    );
    host
}

#[test]
fn a_web_key_copy_is_compared_with_the_containers_key() {
    let same = with_web_key(
        || ok("web-key-0123456789abcdef0123456789abcdef\n"),
        "web-key-0123456789abcdef0123456789abcdef\n",
    );
    assert_eq!(
        compare_web_key(&same, &folder(), Path::new("/keys/docker-web-key")).unwrap(),
        KeyCopy::Same
    );
    let rotated = with_web_key(
        || ok("web-key-rotated-89abcdef0123456789abcdef\n"),
        "web-key-0123456789abcdef0123456789abcdef\n",
    );
    assert_eq!(
        compare_web_key(&rotated, &folder(), Path::new("/keys/docker-web-key")).unwrap(),
        KeyCopy::Differs
    );
    assert!(rotated.said.borrow().is_empty());
}

#[test]
fn a_web_key_that_cannot_be_read_from_the_container_is_an_error_without_its_output() {
    let host = with_web_key(
        || {
            Ok(Finished {
                code: Some(1),
                stdout: "web-key-0123456789abcdef0123456789abcdef\n".into(),
                stderr: String::new(),
            })
        },
        "web-key-0123456789abcdef0123456789abcdef\n",
    );
    let err = compare_web_key(&host, &folder(), Path::new("/keys/docker-web-key")).unwrap_err();
    assert!(err.to_string().contains("exit 1"), "{err}");
    assert!(!err.to_string().contains("web-key-0123"), "{err}");
}
