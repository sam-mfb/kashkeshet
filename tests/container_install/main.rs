//! The container install, pinned: every scenario drives kkt-container's
//! `Container` over Docker on a scripted host, and its whole story (each
//! command line in order with the folder it runs in and what the scripted
//! host answered, the variables it runs without, the folders made, the files written with their modes and text,
//! the lines said, the login link, and the result or the error's text) must
//! match its file in `tests/container_install/`. `UPDATE_GOLDEN=1 cargo
//! test --test container_install` writes the files again; a change to one
//! is a change to what existing installs see.
//!
//! The keys here are made up. No key may reach a command's arguments, a
//! line said or an error, and every scenario checks that.

use futures::executor::block_on;
use kkt_clock::Time;
use kkt_clock::testing::FixedClock;
use kkt_container::{
    Arg, Carry, ConfigReader, Container, ContainerConfig, Folder, InServer, Inputs, KeyCopies,
    LoginLink, Sharing,
};
use kkt_container_docker::Browser;
use kkt_container_docker::testing::{Reply, Scripted};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[path = "../container_install_subjects/mod.rs"]
mod subjects;

thread_local! {
    /// The subject the scenario now running drives.
    static SUBJECT: Cell<&'static str> = const { Cell::new("") };
}

const ACCESS: &str = "access0123456789abcdef0123456789abcdef0123456789";
const WEB: &str = "web00123456789abcdef0123456789abcdef0123456789ab";
const ADMIN: &str = "admin123456789abcdef0123456789abcdef0123456789a";
const AUTH: &str = "tskey-auth-k123456CNTRL-abcdefghijklmnop";
const SECRETS: &[&str] = &[ACCESS, WEB, ADMIN, AUTH];

const FOLDER: &str = "/data/docker";
const ENV: &str = "/data/docker/.env";
const WEB_COPY: &str = "/home/me/.config/kkt/docker-web-key";
const LINK: &str = "https://login.tailscale.com/a/abc123";

const COMPOSE: &str = include_str!("../../compose.yaml");
const COMPOSE_TAILSCALE: &str = include_str!("../../compose.tailscale.yaml");
const SERVE: &str = include_str!("../../tailscale/serve.json");

// ---------------------------------------------------------------- the harness

/// What a scripted command answers: a reply, or a failure to run it.
type Answer = Result<Reply, String>;

fn ok(stdout: &str) -> Answer {
    Ok(Reply::ok(stdout))
}

fn exit(code: i32, stdout: &str, stderr: &str) -> Answer {
    Ok(Reply::exit(code, stdout, stderr))
}

fn cannot_run(why: &str) -> Answer {
    Err(why.to_string())
}

/// Answers a command from the first rule whose text it contains. Each rule
/// gives its answers in turn and then keeps giving its last.
struct Rules(Vec<(&'static str, RefCell<VecDeque<Answer>>)>);

impl Rules {
    fn answer(&self, command: &str) -> io::Result<Reply> {
        let (_, answers) = self
            .0
            .iter()
            .find(|(needle, _)| command.contains(needle))
            .unwrap_or_else(|| panic!("no answer scripted for `{command}`"));
        let mut answers = answers.borrow_mut();
        let answer = if answers.len() > 1 {
            answers.pop_front().unwrap()
        } else {
            answers.front().cloned().unwrap()
        };
        answer.map_err(io::Error::other)
    }
}

/// The rules for a host where Docker works and a kkt with an access and a
/// web key but no admin key is running, before `extra`, which come first.
fn rules(extra: Vec<(&'static str, Vec<Answer>)>) -> Rules {
    let mut all = extra;
    all.extend([
        (
            "compose version",
            vec![ok("Docker Compose version v2.29.0\n")],
        ),
        (
            "key show --admin",
            vec![exit(
                4,
                "",
                "there is no admin key at /data/.config/kkt/admin-key",
            )],
        ),
        ("key show --web", vec![ok(&format!("{WEB}\n"))]),
        ("key show", vec![ok(&format!("{ACCESS}\n"))]),
        (
            "kkt backup /backups",
            vec![ok("Backed up to /backups/kkt-backup-20261117-120000\n")],
        ),
        ("", vec![ok("")]),
    ]);
    Rules(
        all.into_iter()
            .map(|(needle, answers)| (needle, RefCell::new(answers.into())))
            .collect(),
    )
}

/// A login link and a browser that write what happens to them into the
/// transcript.
struct Login {
    host: Scripted,
    /// What `open_requested` answers, one per ask; false when it runs out.
    requests: RefCell<VecDeque<bool>>,
    open_fails: bool,
    /// What `open_failure` answers, one per ask; nothing when it runs out.
    failures: RefCell<VecDeque<Option<String>>>,
}

impl LoginLink for Login {
    fn show(&self, url: &str) {
        self.host.note(&format!("link     shown {url}"));
    }

    fn open_requested(&self) -> bool {
        self.requests.borrow_mut().pop_front().unwrap_or(false)
    }
}

struct OpensIn(Rc<Login>);

impl Browser for OpensIn {
    fn open(&self, url: &str) -> io::Result<()> {
        if self.0.open_fails {
            self.0.host.note(&format!("link     failed to open {url}"));
            Err(io::Error::new(io::ErrorKind::NotFound, "no opener"))
        } else {
            self.0.host.note(&format!("link     opened {url}"));
            Ok(())
        }
    }

    fn open_failure(&self) -> Option<String> {
        self.0.failures.borrow_mut().pop_front().flatten()
    }
}

/// The program's reader of a container's config text, in small: each line
/// is `setting = value`, and `name_pack` is a quoted text.
struct Reader;

impl ConfigReader for Reader {
    fn settings(
        &self,
        text: &str,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<ContainerConfig, String> {
        let mut config = ContainerConfig {
            name_pack: None,
            max_lane_chars: None,
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (key, value) = line
                .split_once(" = ")
                .ok_or_else(|| format!("not a setting in {} (home {home:?})", dir.display()))?;
            if key == "name_pack" {
                let quoted = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'));
                config.name_pack = Some(quoted.ok_or("name_pack: not a text")?.to_string());
            }
        }
        Ok(config)
    }
}

/// One scenario's host, login and Docker.
struct Run {
    clock: Arc<FixedClock>,
    host: Scripted,
    login: Rc<Login>,
    docker: Box<dyn Container>,
}

/// How the login link is used in a scenario.
#[derive(Default)]
struct Asks {
    requests: Vec<bool>,
    open_fails: bool,
    failures: Vec<Option<String>>,
}

impl Run {
    fn new(rules: Rules, files: impl FnOnce(Scripted) -> Scripted) -> Run {
        Run::with_login(rules, files, Asks::default())
    }

    fn with_login(rules: Rules, files: impl FnOnce(Scripted) -> Scripted, asks: Asks) -> Run {
        let clock = Arc::new(FixedClock::new(
            Time::from_parts(2026, 11, 17, 12, 0, 0).unwrap(),
        ));
        let host = files(
            Scripted::new(clock.clone(), move |command| rules.answer(command))
                .knowing("the repository's compose.yaml", COMPOSE)
                .knowing("the repository's compose.tailscale.yaml", COMPOSE_TAILSCALE)
                .knowing("the repository's tailscale/serve.json", SERVE),
        );
        let login = Rc::new(Login {
            host: host.clone(),
            requests: RefCell::new(asks.requests.into()),
            open_fails: asks.open_fails,
            failures: RefCell::new(asks.failures.into()),
        });
        let docker = subjects::subject(
            SUBJECT.with(Cell::get),
            Box::new(OpensIn(login.clone())),
            host.clone(),
        );
        Run {
            clock,
            host,
            login,
            docker,
        }
    }

    fn result<T: Debug>(&self, result: Result<T, String>) {
        match result {
            Ok(value) => self.host.note(&format!("result   {value:?}")),
            Err(error) => self.host.note(&format!("error    {error}")),
        }
    }

    fn provision(&self, inputs: &Inputs) {
        let done =
            block_on(
                self.docker
                    .provision(&*self.clock, &*self.login, &folder(), inputs, &keys()),
            );
        self.result(done);
    }

    fn add_admin_key(&self) {
        let done =
            block_on(
                self.docker
                    .add_admin_key(&*self.clock, &folder(), &keys(), "kkt-admin"),
            );
        self.result(done);
    }

    fn status(&self) {
        let done = self.docker.status(
            &folder(),
            &Reader,
            "default-pack",
            Path::new("kkt/config.toml"),
        );
        self.result(done);
    }

    fn upgrade(&self, keys: Option<&KeyCopies>) {
        let done = block_on(self.docker.upgrade(&*self.clock, &folder(), "0.5.0", keys));
        self.result(done);
    }

    fn transcript(&self) -> String {
        self.host.transcript()
    }
}

fn folder() -> Folder {
    Folder(PathBuf::from(FOLDER))
}

/// Where the host's copies of the keys go: the admin key's in a folder of
/// its own, so both a new folder and one already made are seen.
fn keys() -> KeyCopies {
    KeyCopies {
        access: PathBuf::from("/home/me/.config/kkt/docker-access-key"),
        web: PathBuf::from(WEB_COPY),
        admin: PathBuf::from("/home/me/.config/kkt-admin/docker-admin-key"),
    }
}

fn local() -> Inputs {
    Inputs {
        version: "0.4.0".into(),
        project: "kkt".into(),
        port: 7420,
        allow_hosts: vec![],
        sharing: None,
    }
}

fn tailscale(auth_key: Option<&str>) -> Inputs {
    Inputs {
        version: "0.4.0".into(),
        project: "kkt-test".into(),
        port: 7421,
        allow_hosts: vec!["kkt.example".into(), "10.0.0.5".into()],
        sharing: Some(Sharing {
            hostname: "kkt".into(),
            auth_key: auth_key.map(str::to_string),
        }),
    }
}

const LOCAL_ENV: &str = "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
KKT_VERSION=0.4.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\n";

const TAILSCALE_ENV: &str = "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n\
KKT_VERSION=0.4.0\nCOMPOSE_PROJECT_NAME=kkt\nKKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\n\
TS_HOSTNAME=kkt-1\nTS_TAILNET=tail1234.ts.net\n";

/// A local install's folder, with `env` as its settings file.
fn installed_local(env: &'static str) -> impl FnOnce(Scripted) -> Scripted {
    move |host| {
        host.with_file(ENV, env)
            .with_file("/data/docker/compose.yaml", COMPOSE)
            .with_dir("/data/docker/backups")
    }
}

/// A Tailscale install's folder, with `env` as its settings file.
fn installed_tailscale(env: &'static str) -> impl FnOnce(Scripted) -> Scripted {
    move |host| {
        host.with_file(ENV, env)
            .with_file("/data/docker/compose.tailscale.yaml", COMPOSE_TAILSCALE)
            .with_file("/data/docker/tailscale/serve.json", SERVE)
            .with_dir("/data/docker/backups")
    }
}

fn nothing(host: Scripted) -> Scripted {
    host
}

fn needs_login(url: Option<&str>) -> Answer {
    let url = url
        .map(|u| format!(",\"AuthURL\":\"{u}\""))
        .unwrap_or_default();
    ok(&format!("{{\"BackendState\":\"NeedsLogin\"{url}}}"))
}

fn logged_in() -> Answer {
    ok(
        r#"{"BackendState":"Running","Self":{"DNSName":"kkt-1.tail1234.ts.net."},"CertDomains":["kkt-1.tail1234.ts.net"]}"#,
    )
}

/// One case's transcript, under its name.
fn case(name: &str, run: &Run) -> String {
    format!("== {name}\n{}", run.transcript())
}

/// Several cases, one after another.
fn cases(all: &[String]) -> String {
    all.join("\n")
}

/// Compares `actual` with the golden file `name`, or writes it when
/// `UPDATE_GOLDEN` is set; and checks no key reaches a command, a line said
/// or an error.
fn golden(name: &str, actual: String) {
    // A check added in step 6a. It can only catch more, so it is within
    // the rule that the pins do not change (msg 4060).
    keys_only_where_handled(name, &actual);
    for line in actual.lines() {
        let kind = line.split_whitespace().next().unwrap_or("");
        if matches!(kind, "run" | "attached" | "say" | "error" | "link") {
            for secret in SECRETS {
                assert!(
                    !line.contains(secret),
                    "{name}: a key reached a {kind} line: {line}"
                );
            }
        }
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/container_install")
        .join(format!("{name}.txt"));
    // The first subject writes the file; every other one must match it.
    if std::env::var_os("UPDATE_GOLDEN").is_some()
        && SUBJECT.with(Cell::get) == subjects::SUBJECTS[0]
    {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run `UPDATE_GOLDEN=1 cargo test --test container_install` to write it",
            path.display()
        )
    });
    if expected != actual {
        let mut report = String::new();
        let (old, new): (Vec<_>, Vec<_>) = (expected.lines().collect(), actual.lines().collect());
        for index in 0..old.len().max(new.len()) {
            let (was, is) = (old.get(index), new.get(index));
            if was != is {
                report.push_str(&format!(
                    "first difference at line {}:\n- {}\n+ {}\n",
                    index + 1,
                    was.unwrap_or(&"(nothing)"),
                    is.unwrap_or(&"(nothing)")
                ));
                break;
            }
        }
        panic!(
            "{} differs from what {} now does.\n{report}\nThe whole transcript:\n{actual}",
            path.display(),
            SUBJECT.with(Cell::get)
        );
    }
}

macro_rules! pinned {
    ($($name:ident => $body:expr;)*) => {
        mod pins {
            $(
                #[test]
                fn $name() {
                    for subject in super::subjects::SUBJECTS {
                        super::SUBJECT.with(|s| s.set(subject));
                        super::golden(stringify!($name), $body);
                    }
                }
            )*
        }
    };
}

// ---------------------------------------------------------------- setup

fn setup_local() -> String {
    let run = Run::new(rules(vec![]), nothing);
    run.provision(&local());
    run.transcript()
}

fn setup_local_with_an_admin_key() -> String {
    let run = Run::new(
        rules(vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])]),
        nothing,
    );
    run.provision(&local());
    run.transcript()
}

fn setup_local_again() -> String {
    // Setup carries on in a folder it wrote before.
    let run = Run::new(rules(vec![]), installed_local(LOCAL_ENV));
    run.provision(&local());
    run.transcript()
}

fn setup_tailscale() -> String {
    // Still starting, then another state, then a login link (opened when
    // asked), then logged in.
    let run = Run::with_login(
        rules(vec![(
            "tailscale status --json",
            vec![
                exit(1, "", "container not running"),
                ok(r#"{"BackendState":"Starting"}"#),
                needs_login(Some(LINK)),
                needs_login(Some(LINK)),
                logged_in(),
            ],
        )]),
        nothing,
        Asks {
            requests: vec![false, true],
            ..Asks::default()
        },
    );
    run.provision(&tailscale(None));
    run.transcript()
}

fn setup_tailscale_with_an_auth_key() -> String {
    let run = Run::new(
        rules(vec![("tailscale status --json", vec![logged_in()])]),
        nothing,
    );
    run.provision(&tailscale(Some(AUTH)));
    run.transcript()
}

fn setup_tailscale_restarts_for_a_link() -> String {
    // Logged out with no link: restarted after five checks, then a link.
    let run = Run::new(
        rules(vec![(
            "tailscale status --json",
            vec![
                needs_login(None),
                needs_login(None),
                needs_login(None),
                needs_login(None),
                needs_login(None),
                needs_login(None),
                needs_login(Some(LINK)),
                logged_in(),
            ],
        )]),
        nothing,
    );
    run.provision(&tailscale(None));
    run.transcript()
}

fn setup_tailscale_browser_fails() -> String {
    let fails_at_once = Run::with_login(
        rules(vec![(
            "tailscale status --json",
            vec![needs_login(Some(LINK)), logged_in()],
        )]),
        nothing,
        Asks {
            requests: vec![true],
            open_fails: true,
            ..Asks::default()
        },
    );
    fails_at_once.provision(&tailscale(None));
    let fails_later = Run::with_login(
        rules(vec![(
            "tailscale status --json",
            vec![
                needs_login(Some(LINK)),
                needs_login(Some(LINK)),
                logged_in(),
            ],
        )]),
        nothing,
        Asks {
            requests: vec![true],
            failures: vec![None, Some("xdg-open exited with 3".into())],
            ..Asks::default()
        },
    );
    fails_later.provision(&tailscale(None));
    cases(&[
        case("the browser cannot be started", &fails_at_once),
        case("the browser fails later", &fails_later),
    ])
}

fn setup_tailscale_refusals() -> String {
    let status = |answer: Answer| {
        let run = Run::new(
            rules(vec![("tailscale status --json", vec![answer])]),
            nothing,
        );
        run.provision(&tailscale(None));
        run
    };
    cases(&[
        case(
            "HTTPS is off",
            &status(ok(
                r#"{"BackendState":"Running","Self":{"DNSName":"kkt-1.tail1234.ts.net."},"CertDomains":[]}"#,
            )),
        ),
        case(
            "the node has no name",
            &status(ok(r#"{"BackendState":"Running","CertDomains":["x"]}"#)),
        ),
        case(
            "the node's name has no tailnet",
            &status(ok(
                r#"{"BackendState":"Running","Self":{"DNSName":"kkt."},"CertDomains":["x"]}"#,
            )),
        ),
        case("the answer is not JSON", &status(ok("not json"))),
        case("the answer has no state", &status(ok(r#"{"Self":{}}"#))),
        case(
            "the status cannot be asked",
            &status(cannot_run("no such file or directory")),
        ),
    ])
}

fn setup_login_times_out() -> String {
    let run = Run::new(
        rules(vec![(
            "tailscale status --json",
            vec![needs_login(Some(LINK))],
        )]),
        nothing,
    );
    run.provision(&tailscale(None));
    run.transcript()
}

fn setup_start_times_out() -> String {
    let run = Run::new(
        rules(vec![(
            "kkt key show",
            vec![exit(1, "", "service \"kkt\" is not running")],
        )]),
        nothing,
    );
    run.provision(&local());
    run.transcript()
}

fn setup_web_key_times_out() -> String {
    let run = Run::new(rules(vec![("key show --web", vec![ok("")])]), nothing);
    run.provision(&local());
    run.transcript()
}

fn setup_without_docker() -> String {
    let missing = Run::new(
        rules(vec![(
            "compose version",
            vec![cannot_run("No such file or directory (os error 2)")],
        )]),
        nothing,
    );
    missing.provision(&local());
    let failing = Run::new(
        rules(vec![(
            "compose version",
            vec![exit(1, "", "docker: 'compose' is not a docker command.")],
        )]),
        nothing,
    );
    failing.provision(&local());
    cases(&[
        case("Docker is missing", &missing),
        case("Docker has no Compose", &failing),
    ])
}

fn setup_failures() -> String {
    let with = |extra: Vec<(&'static str, Vec<Answer>)>, files: fn(Scripted) -> Scripted| {
        let run = Run::new(rules(extra), files);
        run.provision(&local());
        run
    };
    cases(&[
        case(
            "the backups folder cannot be given to kkt's user",
            &with(
                vec![("--entrypoint chown", vec![exit(1, "", "no such image")])],
                nothing,
            ),
        ),
        case(
            "kkt does not start",
            &with(
                vec![("up -d", vec![exit(1, "", "port is already allocated")])],
                nothing,
            ),
        ),
        case(
            "kkt cannot be started",
            &with(vec![("up -d", vec![cannot_run("broken pipe")])], nothing),
        ),
        case(
            "kkt is ended by a signal",
            &with(vec![("up -d", vec![Ok(Reply::signal(""))])], nothing),
        ),
        case(
            "the managed folder cannot be made",
            &with(vec![], |host| host.failing_dir(FOLDER)),
        ),
        case(
            "the settings file cannot be written",
            &with(vec![], |host| host.failing_write(ENV, 0)),
        ),
        case(
            "the key copies' folder cannot be made",
            &with(vec![], |host| host.failing_dir("/home/me/.config/kkt")),
        ),
        case(
            "a key copy cannot be written",
            &with(vec![], |host| host.failing_write(WEB_COPY, 0)),
        ),
        case(
            "the access key cannot be asked for",
            &with(
                vec![("kkt key show", vec![cannot_run("broken pipe")])],
                nothing,
            ),
        ),
        case(
            "the backups folder cannot be made",
            &with(vec![], |host| host.failing_dir("/data/docker/backups")),
        ),
        case(
            "a Compose file cannot be written",
            &with(vec![], |host| {
                host.failing_write("/data/docker/compose.yaml", 0)
            }),
        ),
        case(
            "the access key's copy cannot be written",
            &with(vec![], |host| {
                host.failing_write("/home/me/.config/kkt/docker-access-key", 0)
            }),
        ),
        case(
            "the admin key's copy cannot be written",
            &with(
                vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])],
                |host| host.failing_write("/home/me/.config/kkt-admin/docker-admin-key", 0),
            ),
        ),
    ])
}

fn setup_tailscale_failures() -> String {
    let with = |extra: Vec<(&'static str, Vec<Answer>)>, files: fn(Scripted) -> Scripted| {
        let mut all = extra;
        all.push((
            "tailscale status --json",
            vec![needs_login(None), logged_in()],
        ));
        let run = Run::new(rules(all), files);
        run.provision(&tailscale(None));
        run
    };
    let restart_fails = Run::new(
        rules(vec![
            ("tailscale status --json", vec![needs_login(None)]),
            ("restart tailscale", vec![exit(1, "", "no such service")]),
        ]),
        nothing,
    );
    restart_fails.provision(&tailscale(None));
    cases(&[
        case(
            "Tailscale does not start",
            &with(
                vec![("up -d tailscale", vec![exit(1, "", "no such image")])],
                nothing,
            ),
        ),
        case(
            "the settings cannot be written after the login",
            &with(vec![], |host| host.failing_write(ENV, 1)),
        ),
        case("the restart for a link fails", &restart_fails),
    ])
}

fn setup_admin_key_reads() -> String {
    let with = |answer: Answer| {
        let run = Run::new(rules(vec![("key show --admin", vec![answer])]), nothing);
        run.provision(&local());
        run
    };
    cases(&[
        case(
            "an older kkt does not know --admin",
            &with(exit(2, "", "error: unexpected argument '--admin' found")),
        ),
        case(
            "unreadable: the output, which may hold the key, is left out",
            &with(exit(1, &format!("{ADMIN}\n"), &format!("partial {ADMIN}"))),
        ),
        case(
            "ended by a signal",
            &with(Ok(Reply::signal(&format!("{ADMIN}\n")))),
        ),
        case("no key printed", &with(ok("  \n"))),
    ])
}

// ---------------------------------------------------------------- the admin key

fn admin_key_new() -> String {
    let run = Run::new(
        rules(vec![(
            "key show --admin",
            vec![
                exit(4, "", "there is no admin key"),
                ok(&format!("{ADMIN}\n")),
            ],
        )]),
        installed_local(LOCAL_ENV),
    );
    run.add_admin_key();
    run.transcript()
}

fn admin_key_kept() -> String {
    // A key made by an earlier run that then failed to restart kkt: no new
    // key, and kkt is restarted.
    let run = Run::new(
        rules(vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])]),
        installed_local(LOCAL_ENV),
    );
    run.add_admin_key();
    run.transcript()
}

fn admin_key_failures() -> String {
    let with = |extra: Vec<(&'static str, Vec<Answer>)>| {
        let run = Run::new(rules(extra), installed_local(LOCAL_ENV));
        run.add_admin_key();
        run
    };
    cases(&[
        case(
            "kkt does not start",
            &with(vec![("kkt key show", vec![exit(1, "", "not running")])]),
        ),
        case(
            "making the key fails",
            &with(vec![(
                "key create --admin",
                vec![exit(1, "", "no such option")],
            )]),
        ),
        case(
            "the restart fails",
            &with(vec![("restart kkt", vec![exit(1, "", "no such service")])]),
        ),
        case("still no key after making one", &with(vec![])),
        case(
            "unreadable: the output, which may hold the key, is left out",
            &with(vec![(
                "key show --admin",
                vec![exit(1, &format!("{ADMIN}\n"), "")],
            )]),
        ),
        case("the key copies cannot be saved", &{
            let run = Run::new(
                rules(vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])]),
                |host| installed_local(LOCAL_ENV)(host).failing_write(WEB_COPY, 0),
            );
            run.add_admin_key();
            run
        }),
        case(
            "an older kkt does not know --admin",
            &with(vec![(
                "key show --admin",
                vec![exit(2, "", "error: unexpected argument '--admin' found")],
            )]),
        ),
    ])
}

// ---------------------------------------------------------------- status

fn status_not_set_up() -> String {
    let run = Run::new(rules(vec![]), nothing);
    run.status();
    run.transcript()
}

fn status_local() -> String {
    let stopped = Run::new(
        rules(vec![("ps --status running", vec![ok("")])]),
        installed_local(LOCAL_ENV),
    );
    stopped.status();
    let unknown = Run::new(
        rules(vec![(
            "ps --status running",
            vec![exit(1, "", "Cannot connect to the Docker daemon")],
        )]),
        installed_local(LOCAL_ENV),
    );
    unknown.status();
    let cannot = Run::new(
        rules(vec![(
            "ps --status running",
            vec![cannot_run("broken pipe")],
        )]),
        installed_local(LOCAL_ENV),
    );
    cannot.status();
    let unreadable = Run::new(rules(vec![]), |host| host.with_dir(ENV));
    unreadable.status();
    cases(&[
        case("stopped", &stopped),
        case("not known", &unknown),
        case("Docker cannot be asked", &cannot),
        case("the settings file cannot be read", &unreadable),
    ])
}

fn status_tailscale_logins() -> String {
    let with = |answer: Answer| {
        let run = Run::new(
            rules(vec![
                ("ps --status running", vec![ok("tailscale\n")]),
                ("tailscale status --json", vec![answer]),
            ]),
            installed_tailscale(TAILSCALE_ENV),
        );
        run.status();
        run
    };
    cases(&[
        case("logged in", &with(logged_in())),
        case("needs a login", &with(needs_login(Some(LINK)))),
        case("another state", &with(ok(r#"{"BackendState":"Stopped"}"#))),
        case("an unreadable answer", &with(ok("not json"))),
        case(
            "unreachable",
            &with(exit(1, "", "service \"tailscale\" is not running")),
        ),
        case("cannot be asked", &with(cannot_run("broken pipe"))),
    ])
}

fn status_tailscale_answers_without_a_state() -> String {
    let with = |answer: Answer| {
        let run = Run::new(
            rules(vec![
                ("ps --status running", vec![ok("tailscale\n")]),
                ("tailscale status --json", vec![answer]),
            ]),
            installed_tailscale(TAILSCALE_ENV),
        );
        run.status();
        run
    };
    cases(&[
        case(
            "an answer with no backend state",
            &with(ok(r#"{"Self":{"DNSName":"kkt.tail1234.ts.net."}}"#)),
        ),
        case(
            "a backend state that is not text",
            &with(ok(r#"{"BackendState":3}"#)),
        ),
    ])
}

const SERVE_ARGS: &str = r#""Cmd":["serve","--bind","0.0.0.0:7420"],"Entrypoint":["/usr/local/bin/kkt"],"Env":["HOME=/home/kkt","PATH=/usr/bin"],"WorkingDir":"/data""#;

/// A running local install whose container `docker inspect` describes with
/// `config`, and whose other answers are `extra`.
fn running(config: String, extra: Vec<(&'static str, Vec<Answer>)>) -> Run {
    let mut all = extra;
    all.extend([
        ("ps --status running", vec![ok("kkt\n")]),
        ("ps --quiet kkt", vec![ok("0123abcd\n")]),
        ("inspect --format", vec![ok(&config)]),
    ]);
    let run = Run::new(rules(all), installed_local(LOCAL_ENV));
    run.status();
    run
}

fn status_name_pack() -> String {
    let inspect = |inner: &str| format!("{{{inner}}}");
    cases(&[
        case(
            "the default: no flag and no config file",
            &running(
                inspect(SERVE_ARGS),
                vec![("test -e", vec![exit(1, "", "")])],
            ),
        ),
        case(
            "from the config file",
            &running(
                inspect(SERVE_ARGS),
                vec![
                    ("test -e", vec![ok("")]),
                    ("cat --", vec![ok("name_pack = \"birds\"\n")]),
                ],
            ),
        ),
        case(
            "a config file that names none",
            &running(
                inspect(SERVE_ARGS),
                vec![("test -e", vec![ok("")]), ("cat --", vec![ok("\n")])],
            ),
        ),
        case(
            "from --name-pack",
            &running(
                inspect(&SERVE_ARGS.replace("\"--bind\"", "\"--name-pack\",\"trees\",\"--bind\"")),
                vec![],
            ),
        ),
        case(
            "from --name-pack=",
            &running(
                inspect(&SERVE_ARGS.replace("\"--bind\"", "\"--name-pack=rivers\",\"--bind\"")),
                vec![],
            ),
        ),
        case(
            "from --config, absolute",
            &running(
                inspect(
                    &SERVE_ARGS.replace("\"--bind\"", "\"--config\",\"/etc/kkt.toml\",\"--bind\""),
                ),
                vec![("cat --", vec![ok("name_pack = \"stones\"\n")])],
            ),
        ),
        case(
            "from --config, relative to the working folder",
            &running(
                inspect(&SERVE_ARGS.replace("\"--bind\"", "\"--config=conf/kkt.toml\",\"--bind\"")),
                vec![("cat --", vec![ok("name_pack = \"stones\"\n")])],
            ),
        ),
        case(
            "from --config, relative, with no working folder",
            &running(
                inspect(
                    &SERVE_ARGS
                        .replace("\"--bind\"", "\"--config=kkt.toml\",\"--bind\"")
                        .replace("\"WorkingDir\":\"/data\"", "\"WorkingDir\":\"\""),
                ),
                vec![("cat --", vec![ok("name_pack = \"stones\"\n")])],
            ),
        ),
        case(
            "under XDG_CONFIG_HOME",
            &running(
                inspect(&SERVE_ARGS.replace(
                    "\"HOME=/home/kkt\"",
                    "\"HOME=/home/kkt\",\"XDG_CONFIG_HOME=/config\"",
                )),
                vec![("test -e", vec![exit(1, "", "")])],
            ),
        ),
        case(
            "an entry with no = is skipped",
            &running(
                inspect(&SERVE_ARGS.replace(
                    "\"HOME=/home/kkt\"",
                    "\"XDG_CONFIG_HOME\",\"HOME\",\"HOME=/home/kkt\"",
                )),
                vec![("test -e", vec![exit(1, "", "")])],
            ),
        ),
    ])
}

fn status_name_pack_errors() -> String {
    let inspect = |inner: &str| format!("{{{inner}}}");
    let quiet = |answer: Answer| {
        let run = Run::new(
            rules(vec![
                ("ps --status running", vec![ok("kkt\n")]),
                ("ps --quiet kkt", vec![answer]),
            ]),
            installed_local(LOCAL_ENV),
        );
        run.status();
        run
    };
    cases(&[
        case("no container ID", &quiet(ok("\n"))),
        case("not a container ID", &quiet(ok("kkt-1\n"))),
        case("the ID query fails", &quiet(exit(1, "", "daemon"))),
        case("the ID query cannot run", &quiet(cannot_run("broken pipe"))),
        case(
            "inspect fails",
            &running(
                String::new(),
                vec![("inspect --format", vec![exit(1, "", "no such object")])],
            ),
        ),
        case("inspect is not JSON", &running("not json".into(), vec![])),
        case(
            "no command",
            &running(
                inspect(&SERVE_ARGS.replace("\"Cmd\":[", "\"Cmd\":7,\"X\":[")),
                vec![],
            ),
        ),
        case(
            "no entrypoint",
            &running(
                inspect(&SERVE_ARGS.replace("\"Entrypoint\":[", "\"Entrypoint\":null,\"X\":[")),
                vec![],
            ),
        ),
        case(
            "not kkt serve",
            &running(
                inspect(&SERVE_ARGS.replace("\"serve\"", "\"watch\"")),
                vec![],
            ),
        ),
        case(
            "another program",
            &running(
                inspect(&SERVE_ARGS.replace("/usr/local/bin/kkt", "/bin/sh")),
                vec![],
            ),
        ),
        case(
            "two entrypoint words",
            &running(
                inspect(&SERVE_ARGS.replace("[\"/usr/local/bin/kkt\"]", "[\"tini\",\"kkt\"]")),
                vec![],
            ),
        ),
        case(
            "--name-pack with no value",
            &running(
                inspect(
                    &SERVE_ARGS.replace("\"0.0.0.0:7420\"", "\"0.0.0.0:7420\",\"--name-pack\""),
                ),
                vec![],
            ),
        ),
        case(
            "no environment",
            &running(
                inspect(&SERVE_ARGS.replace("\"Env\":[", "\"Env\":{},\"X\":[")),
                vec![],
            ),
        ),
        case(
            "no home folder",
            &running(
                inspect(&SERVE_ARGS.replace("\"HOME=/home/kkt\",", "\"HOME=\",")),
                vec![],
            ),
        ),
        case(
            "the check for the config file fails",
            &running(
                inspect(SERVE_ARGS),
                vec![("test -e", vec![exit(2, "", "")])],
            ),
        ),
        case(
            "the check for the config file cannot run",
            &running(
                inspect(SERVE_ARGS),
                vec![("test -e", vec![cannot_run("broken pipe")])],
            ),
        ),
        case(
            "the config file cannot be read",
            &running(
                inspect(SERVE_ARGS),
                vec![
                    ("test -e", vec![ok("")]),
                    ("cat --", vec![exit(1, "", "permission denied")]),
                ],
            ),
        ),
        case(
            "the config file is unreadable",
            &running(
                inspect(SERVE_ARGS),
                vec![
                    ("test -e", vec![ok("")]),
                    ("cat --", vec![ok("name_pack: birds\n")]),
                ],
            ),
        ),
    ])
}

// ---------------------------------------------------------------- what is there

fn existing() -> String {
    let with = |files: Box<dyn FnOnce(Scripted) -> Scripted>| {
        let run = Run::new(rules(vec![]), files);
        let done = run.docker.existing(&folder());
        run.result(done);
        run
    };
    let local_env = |env: &'static str| -> Box<dyn FnOnce(Scripted) -> Scripted> {
        Box::new(installed_local(env))
    };
    let tailscale_env = |env: &'static str| -> Box<dyn FnOnce(Scripted) -> Scripted> {
        Box::new(installed_tailscale(env))
    };
    cases(&[
        case("nothing there", &with(Box::new(nothing))),
        case("a local install", &with(local_env(LOCAL_ENV))),
        case("a Tailscale install", &with(tailscale_env(TAILSCALE_ENV))),
        case(
            "the settings file cannot be read",
            &with(Box::new(|host: Scripted| host.with_dir(ENV))),
        ),
        case(
            "another COMPOSE_FILE",
            &with(local_env("KKT_PORT=7420\nCOMPOSE_FILE=other.yaml\n")),
        ),
        case(
            "a local install's Compose file is missing",
            &with(Box::new(|host: Scripted| host.with_file(ENV, LOCAL_ENV))),
        ),
        case(
            "a Tailscale install's serve file is missing",
            &with(Box::new(|host: Scripted| {
                host.with_file(ENV, TAILSCALE_ENV)
                    .with_file("/data/docker/compose.tailscale.yaml", COMPOSE_TAILSCALE)
            })),
        ),
        case("no KKT_PORT", &with(local_env("KKT_VERSION=0.4.0\n"))),
        case("KKT_PORT not a number", &with(local_env("KKT_PORT=http\n"))),
        case("KKT_PORT zero", &with(local_env("KKT_PORT=0\n"))),
        case(
            "no TS_HOSTNAME",
            &with(tailscale_env(
                "KKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_TAILNET=t.ts.net\n",
            )),
        ),
        case(
            "no TS_TAILNET",
            &with(tailscale_env(
                "KKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_HOSTNAME=kkt\n",
            )),
        ),
        case(
            "the login never finished",
            &with(tailscale_env(
                "KKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_HOSTNAME=kkt\nTS_TAILNET=pending.invalid\n",
            )),
        ),
        case(
            "a hostname with a dot",
            &with(tailscale_env(
                "KKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_HOSTNAME=kkt.a\nTS_TAILNET=t.ts.net\n",
            )),
        ),
        case(
            "a hostname that is not a DNS label",
            &with(tailscale_env(
                "KKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_HOSTNAME=-kkt\nTS_TAILNET=t.ts.net\n",
            )),
        ),
        case(
            "a tailnet that is not a DNS name",
            &with(tailscale_env(
                "KKT_PORT=7420\nCOMPOSE_FILE=compose.tailscale.yaml\nTS_HOSTNAME=kkt\nTS_TAILNET=t..net\n",
            )),
        ),
    ])
}

fn reads() -> String {
    let run = |files: Box<dyn FnOnce(Scripted) -> Scripted>| Run::new(rules(vec![]), files);
    let installed = run(Box::new(installed_local(LOCAL_ENV)));
    installed.result(Ok::<_, String>(installed.docker.has_settings(&folder())));
    installed.result(Ok::<_, String>(installed.docker.settings_file(&folder())));
    installed.result(installed.docker.version(&folder()));
    let empty = run(Box::new(nothing));
    empty.result(Ok::<_, String>(empty.docker.has_settings(&folder())));
    empty.result(Ok::<_, String>(empty.docker.settings_file(&folder())));
    empty.result(empty.docker.version(&folder()));
    let a_folder = run(Box::new(|host: Scripted| host.with_dir(ENV)));
    a_folder.result(Ok::<_, String>(a_folder.docker.has_settings(&folder())));
    a_folder.result(Ok::<_, String>(a_folder.docker.settings_file(&folder())));
    let no_version = run(Box::new(installed_local("KKT_PORT=7420\n")));
    no_version.result(no_version.docker.version(&folder()));
    let key = run(Box::new(|host: Scripted| {
        host.with_file("/home/me/ts-key", "  tskey-auth-k1-abc\n")
    }));
    key.result(key.docker.sharing_key(Path::new("/home/me/ts-key")));
    key.result(key.docker.sharing_key(Path::new("/home/me/no-key")));
    cases(&[
        case(
            "installed: settings, a settings file, the version",
            &installed,
        ),
        case("nothing there", &empty),
        case("a folder where the settings file goes", &a_folder),
        case("no KKT_VERSION", &no_version),
        case("the sharing key, trimmed, and a missing one", &key),
    ])
}

// ---------------------------------------------------------------- the web key

fn compare_web_key() -> String {
    let with = |answer: Answer, copy: Option<&'static str>| {
        let run = Run::new(rules(vec![("key show --web", vec![answer])]), move |host| {
            let host = installed_local(LOCAL_ENV)(host);
            match copy {
                Some(text) => host.with_file(WEB_COPY, text),
                None => host,
            }
        });
        let done = run.docker.compare_web_key(&folder(), Path::new(WEB_COPY));
        run.result(done);
        run
    };
    let web_line: &'static str = Box::leak(format!("{WEB}\n").into_boxed_str());
    cases(&[
        case("the same", &with(ok(web_line), Some(web_line))),
        case("different", &with(ok(web_line), Some("an older key\n"))),
        case("no copy", &with(ok(web_line), None)),
        case(
            "unreadable: the output, which may hold the key, is left out",
            &with(exit(1, web_line, &format!("partial {WEB}")), Some(web_line)),
        ),
        case("no key printed", &with(ok("\n"), Some(web_line))),
        case(
            "ended by a signal",
            &with(Ok(Reply::signal(web_line)), Some(web_line)),
        ),
        case(
            "cannot be asked",
            &with(cannot_run("broken pipe"), Some(web_line)),
        ),
    ])
}

// ---------------------------------------------------------------- up and down

fn up_and_down() -> String {
    let fine = Run::new(rules(vec![]), installed_local(LOCAL_ENV));
    fine.result(fine.docker.up(&folder()));
    fine.result(fine.docker.down(&folder(), false));
    fine.result(fine.docker.down(&folder(), true));
    let failing = Run::new(
        rules(vec![
            ("up -d", vec![exit(1, "", "no such image")]),
            (
                "down",
                vec![exit(1, "", "Cannot connect to the Docker daemon")],
            ),
        ]),
        installed_local(LOCAL_ENV),
    );
    failing.result(failing.docker.up(&folder()));
    failing.result(failing.docker.down(&folder(), false));
    cases(&[
        case("up; down keeping the data; down with it", &fine),
        case("both fail", &failing),
    ])
}

fn up_and_down_cannot_run() -> String {
    let cannot = Run::new(
        rules(vec![
            ("up -d", vec![cannot_run("no such file or directory")]),
            ("down", vec![cannot_run("no such file or directory")]),
        ]),
        installed_local(LOCAL_ENV),
    );
    cannot.result(cannot.docker.up(&folder()));
    cannot.result(cannot.docker.down(&folder(), false));
    cases(&[case("the tool cannot run at all", &cannot)])
}

// ---------------------------------------------------------------- backup

fn backup() -> String {
    let with = |extra: Vec<(&'static str, Vec<Answer>)>,
                files: Box<dyn FnOnce(Scripted) -> Scripted>,
                dest: Option<&str>| {
        let run = Run::new(rules(extra), files);
        let done = run.docker.backup(&folder(), dest.map(Path::new));
        run.result(done);
        run
    };
    let installed =
        || -> Box<dyn FnOnce(Scripted) -> Scripted> { Box::new(installed_local(LOCAL_ENV)) };
    cases(&[
        case("in the managed folder", &with(vec![], installed(), None)),
        case(
            "to a folder typed relative",
            &with(vec![], installed(), Some("saved")),
        ),
        case(
            "to an absolute folder",
            &with(vec![], installed(), Some("/srv/backups")),
        ),
        case(
            "the backup fails",
            &with(
                vec![("kkt backup", vec![exit(1, "", "database is locked")])],
                installed(),
                Some("saved"),
            ),
        ),
        case(
            "the backup cannot run",
            &with(
                vec![("kkt backup", vec![cannot_run("broken pipe")])],
                installed(),
                None,
            ),
        ),
        case(
            "the backup says something else",
            &with(vec![("kkt backup", vec![ok("done\n")])], installed(), None),
        ),
        case(
            "the backup names no folder",
            &with(
                vec![("kkt backup", vec![ok("Backed up to /\n")])],
                installed(),
                None,
            ),
        ),
        case(
            "the current folder cannot be found",
            &with(
                vec![],
                Box::new(|host: Scripted| installed_local(LOCAL_ENV)(host).without_working_dir()),
                Some("saved"),
            ),
        ),
        case(
            "the destination cannot be made",
            &with(
                vec![],
                Box::new(|host: Scripted| {
                    installed_local(LOCAL_ENV)(host).failing_dir("/home/me/saved")
                }),
                Some("saved"),
            ),
        ),
        case(
            "the copy out fails",
            &with(
                vec![(" cp kkt:", vec![exit(1, "", "no space left")])],
                installed(),
                Some("saved"),
            ),
        ),
        case(
            "the copy out cannot run",
            &with(
                vec![(" cp kkt:", vec![cannot_run("broken pipe")])],
                installed(),
                Some("saved"),
            ),
        ),
    ])
}

// ---------------------------------------------------------------- upgrade

fn upgrade_local() -> String {
    let run = Run::new(rules(vec![]), installed_local(LOCAL_ENV));
    run.upgrade(Some(&keys()));
    run.transcript()
}

fn upgrade_tailscale_with_an_admin_key() -> String {
    let run = Run::new(
        rules(vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])]),
        installed_tailscale(TAILSCALE_ENV),
    );
    run.upgrade(Some(&keys()));
    run.transcript()
}

fn upgrade_without_key_copies() -> String {
    let run = Run::new(rules(vec![]), installed_local(LOCAL_ENV));
    run.upgrade(None);
    run.transcript()
}

fn upgrade_keeps_a_hand_edited_settings_file() -> String {
    let edited = Run::new(
        rules(vec![]),
        installed_local("# mine\nKKT_PORT=8000\nKKT_VERSION=0.3.9\nEXTRA=1\n"),
    );
    edited.upgrade(None);
    let no_version = Run::new(rules(vec![]), installed_local("KKT_PORT=8000"));
    no_version.upgrade(None);
    cases(&[
        case("edited by hand", &edited),
        case("with no KKT_VERSION and no newline at the end", &no_version),
    ])
}

fn upgrade_failures() -> String {
    let with = |extra: Vec<(&'static str, Vec<Answer>)>,
                files: Box<dyn FnOnce(Scripted) -> Scripted>| {
        let run = Run::new(rules(extra), files);
        run.upgrade(Some(&keys()));
        run
    };
    let installed =
        || -> Box<dyn FnOnce(Scripted) -> Scripted> { Box::new(installed_local(LOCAL_ENV)) };
    cases(&[
        case(
            "the settings file cannot be read",
            &with(vec![], Box::new(nothing)),
        ),
        case(
            "the backup fails",
            &with(
                vec![("kkt backup", vec![exit(1, "", "database is locked")])],
                installed(),
            ),
        ),
        case(
            "the Compose files cannot be written",
            &with(
                vec![],
                Box::new(|host: Scripted| {
                    installed_local(LOCAL_ENV)(host).failing_write("/data/docker/compose.yaml", 0)
                }),
            ),
        ),
        case(
            "the settings file cannot be written",
            &with(
                vec![],
                Box::new(|host: Scripted| installed_local(LOCAL_ENV)(host).failing_write(ENV, 0)),
            ),
        ),
        case(
            "the pull fails: the settings are put back",
            &with(
                vec![(
                    "compose --project-directory /data/docker pull",
                    vec![exit(1, "", "no network")],
                )],
                installed(),
            ),
        ),
        case(
            "the start fails: the settings are put back",
            &with(
                vec![("up -d", vec![exit(1, "", "port is already allocated")])],
                installed(),
            ),
        ),
        case(
            "the pull fails and the settings cannot be put back",
            &with(
                vec![(
                    "compose --project-directory /data/docker pull",
                    vec![exit(1, "", "no network")],
                )],
                Box::new(|host: Scripted| installed_local(LOCAL_ENV)(host).failing_write(ENV, 1)),
            ),
        ),
        case(
            "the keys are not saved: said, and the upgrade stands",
            &with(
                vec![("key show --web", vec![exit(1, "", "not running")])],
                installed(),
            ),
        ),
    ])
}

// ---------------------------------------------------------------- runs in the container

fn in_server(carry: Carry) -> InServer<'static> {
    let args: &'static [Arg] = Box::leak(
        vec![
            Arg::Text("lanes".into()),
            Arg::Text("import".into()),
            Arg::Folder,
            Arg::Text("--note=a b".into()),
        ]
        .into_boxed_slice(),
    );
    InServer {
        args,
        folder_prefix: "/tmp/kkt-lanes",
        carry,
    }
}

fn run_in_server_with(extra: Vec<(&'static str, Vec<Answer>)>, carry: Carry) -> Run {
    let run_with = |extra| Run::new(rules(extra), installed_local(LOCAL_ENV));
    let mut all = extra;
    all.extend([
        ("mktemp -d", vec![ok("/tmp/kkt-lanes.AbC123xyz0\n")]),
        ("kkt kkt lanes", vec![ok("Imported 2 lanes.\n")]),
    ]);
    let run = run_with(all);
    let done = run.docker.run_in_server(&folder(), &in_server(carry));
    run.result(done);
    run
}

fn carried_in() -> Carry {
    Carry::In(vec![
        PathBuf::from("/home/me/lanes/one.md"),
        PathBuf::from("/home/me/lanes/two.md"),
    ])
}

fn carried_out() -> Carry {
    Carry::Out(PathBuf::from("/home/me/export"))
}

fn run_in_server() -> String {
    cases(&[
        case(
            "files carried in",
            &run_in_server_with(vec![], carried_in()),
        ),
        case(
            "files carried out",
            &run_in_server_with(vec![], carried_out()),
        ),
    ])
}

fn run_in_server_failures() -> String {
    let in_ = |extra| run_in_server_with(extra, carried_in());
    let out = |extra| run_in_server_with(extra, carried_out());
    let no_name = {
        let run = Run::new(
            rules(vec![("mktemp -d", vec![ok("/tmp/kkt-lanes.AbC123xyz0\n")])]),
            installed_local(LOCAL_ENV),
        );
        let done = run.docker.run_in_server(
            &folder(),
            &in_server(Carry::In(vec![PathBuf::from("/home/me/..")])),
        );
        run.result(done);
        run
    };
    let no_dest = {
        let run = Run::new(
            rules(vec![("mktemp -d", vec![ok("/tmp/kkt-lanes.AbC123xyz0\n")])]),
            |host| installed_local(LOCAL_ENV)(host).failing_dir("/home/me/export"),
        );
        let done = run
            .docker
            .run_in_server(&folder(), &in_server(carried_out()));
        run.result(done);
        run
    };
    cases(&[
        case(
            "mktemp fails",
            &in_(vec![(
                "mktemp -d",
                vec![exit(1, "", "read-only file system")],
            )]),
        ),
        case(
            "mktemp names another folder",
            &in_(vec![("mktemp -d", vec![ok("/tmp/other\n")])]),
        ),
        case(
            "mktemp names an odd folder",
            &in_(vec![("mktemp -d", vec![ok("/tmp/kkt-lanes.ab/c\n")])]),
        ),
        case(
            "mktemp names only the prefix",
            &in_(vec![("mktemp -d", vec![ok("/tmp/kkt-lanes.\n")])]),
        ),
        case("a file with no name", &no_name),
        case(
            "a copy in fails",
            &in_(vec![(
                " cp /home/me/lanes/two.md",
                vec![exit(1, "", "no such file")],
            )]),
        ),
        case(
            "the chmod fails",
            &in_(vec![(
                "chmod -R",
                vec![exit(1, "", "operation not permitted")],
            )]),
        ),
        case(
            "the run fails; the folder is removed",
            &in_(vec![(
                "kkt kkt lanes",
                vec![exit(1, "", "lane one.md: no title")],
            )]),
        ),
        case(
            "the run fails and so does the removal: said",
            &in_(vec![
                ("kkt kkt lanes", vec![exit(1, "", "lane one.md: no title")]),
                ("rm -rf", vec![exit(1, "", "device busy")]),
            ]),
        ),
        case(
            "the run works but the removal fails",
            &in_(vec![("rm -rf", vec![exit(1, "", "device busy")])]),
        ),
        case(
            "the run fails before a copy out; the folder is removed",
            &out(vec![("kkt kkt lanes", vec![exit(1, "", "no lanes")])]),
        ),
        case("the destination cannot be made", &no_dest),
        case(
            "the copy out fails",
            &out(vec![(" cp kkt:", vec![exit(1, "", "no space left")])]),
        ),
    ])
}

fn config_home_file() -> String {
    let with = |answer: Answer, relative: &str| {
        let run = Run::new(
            rules(vec![("--entrypoint sh", vec![answer])]),
            installed_local(LOCAL_ENV),
        );
        let done = run.docker.config_home_file(&folder(), Path::new(relative));
        run.result(done);
        run
    };
    cases(&[
        case(
            "present",
            &with(ok("name_pack = \"birds\"\n"), "kkt/config.toml"),
        ),
        case("absent", &with(ok(""), "kkt/config.toml")),
        case(
            "the run fails",
            &with(exit(1, "", "no such image"), "kkt/config.toml"),
        ),
        case("absolute: refused", &with(ok(""), "/etc/passwd")),
        case(
            "leaves its folder: refused",
            &with(ok(""), "kkt/../../etc/passwd"),
        ),
        case("a space: refused", &with(ok(""), "kkt/my config.toml")),
        case("empty: refused", &with(ok(""), "")),
        case("an empty part: refused", &with(ok(""), "kkt//config.toml")),
    ])
}

// ---------------------------------------------------------------- the tool cannot run, per call

fn run_in_server_cannot_run() -> String {
    let in_ = |extra| run_in_server_with(extra, carried_in());
    let out = |extra| run_in_server_with(extra, carried_out());
    let gone = || vec![cannot_run("no such file or directory")];
    cases(&[
        case("mktemp cannot run", &in_(vec![("mktemp -d", gone())])),
        case(
            "a copy in cannot run",
            &in_(vec![(" cp /home/me/lanes/one.md", gone())]),
        ),
        case("the chmod cannot run", &in_(vec![("chmod -R", gone())])),
        case(
            "the run cannot run; the folder is removed",
            &in_(vec![("kkt kkt lanes", gone())]),
        ),
        case("the copy out cannot run", &out(vec![(" cp kkt:", gone())])),
        case(
            "the run works but the removal cannot run",
            &in_(vec![("rm -rf", gone())]),
        ),
        case(
            "the run fails and the removal cannot run: said",
            &in_(vec![
                ("kkt kkt lanes", vec![exit(1, "", "lane one.md: no title")]),
                ("rm -rf", gone()),
            ]),
        ),
    ])
}

fn config_home_file_cannot_run() -> String {
    let run = Run::new(
        rules(vec![(
            "--entrypoint sh",
            vec![cannot_run("no such file or directory")],
        )]),
        installed_local(LOCAL_ENV),
    );
    let done = run
        .docker
        .config_home_file(&folder(), Path::new("kkt/config.toml"));
    run.result(done);
    cases(&[case("the run cannot run", &run)])
}

fn status_name_pack_cannot_run() -> String {
    cases(&[
        case(
            "inspect cannot run",
            &running(
                String::new(),
                vec![("inspect --format", vec![cannot_run("broken pipe")])],
            ),
        ),
        case(
            "reading the config file cannot run",
            &running(
                format!("{{{SERVE_ARGS}}}"),
                vec![
                    ("test -e", vec![ok("")]),
                    ("cat --", vec![cannot_run("broken pipe")]),
                ],
            ),
        ),
    ])
}

/// A key appears in a transcript only where the install handles it: in
/// what the scripted host answered (`  -> `, such as `kkt key show`
/// printing it) and in the text of a file written (`  | `, a key copy).
/// Never in a command, a line said, a link, an error or a result, nor in a
/// failure's text, however a failed command's output reached it.
fn keys_only_where_handled(name: &str, actual: &str) {
    for line in actual.lines() {
        if line.starts_with("  -> ") || line.starts_with("  | ") {
            continue;
        }
        for secret in SECRETS {
            assert!(
                !line.contains(secret),
                "{name}: a key is outside an answer or a file written: {line}"
            );
        }
    }
}

// ---------------------------------------------------------------- setup, before it moves

fn setup_cannot_run() -> String {
    let local_with = |extra: Vec<(&'static str, Vec<Answer>)>| {
        let run = Run::new(rules(extra), nothing);
        run.provision(&local());
        run
    };
    let shared_with = |extra: Vec<(&'static str, Vec<Answer>)>| {
        let mut all = extra;
        all.push(("tailscale status --json", vec![needs_login(None)]));
        let run = Run::new(rules(all), nothing);
        run.provision(&tailscale(None));
        run
    };
    cases(&[
        case(
            "the backups folder's owner cannot be set",
            &local_with(vec![(
                "--entrypoint chown",
                vec![cannot_run("broken pipe")],
            )]),
        ),
        case(
            "the web key cannot be asked for",
            &local_with(vec![("key show --web", vec![cannot_run("broken pipe")])]),
        ),
        case(
            "the admin key cannot be asked for",
            &local_with(vec![("key show --admin", vec![cannot_run("broken pipe")])]),
        ),
        case(
            "Tailscale cannot be started",
            &shared_with(vec![("up -d tailscale", vec![cannot_run("broken pipe")])]),
        ),
        case(
            "Tailscale cannot be restarted for a link",
            &shared_with(vec![("restart tailscale", vec![cannot_run("broken pipe")])]),
        ),
    ])
}

fn setup_tailscale_file_failures() -> String {
    let with = |files: fn(Scripted) -> Scripted| {
        let run = Run::new(
            rules(vec![("tailscale status --json", vec![logged_in()])]),
            files,
        );
        run.provision(&tailscale(None));
        run
    };
    cases(&[
        case(
            "the Tailscale Compose file cannot be written",
            &with(|host| host.failing_write("/data/docker/compose.tailscale.yaml", 0)),
        ),
        case(
            "the serve settings' folder cannot be made",
            &with(|host| host.failing_dir("/data/docker/tailscale")),
        ),
        case(
            "the serve settings cannot be written",
            &with(|host| host.failing_write("/data/docker/tailscale/serve.json", 0)),
        ),
    ])
}

fn setup_tailscale_link_lost() -> String {
    // A link was shown; then the answers carry none for longer than the
    // restart waits for, and no restart comes, since the person has the link.
    let mut answers = vec![needs_login(Some(LINK))];
    answers.extend((0..7).map(|_| needs_login(None)));
    answers.push(logged_in());
    let run = Run::new(rules(vec![("tailscale status --json", answers)]), nothing);
    run.provision(&tailscale(None));
    run.transcript()
}

fn setup_key_copy_without_a_folder() -> String {
    // A key copy's path with no folder above it: nothing is made for it.
    let run = Run::new(rules(vec![]), nothing);
    let keys = KeyCopies {
        access: PathBuf::new(),
        ..keys()
    };
    let done = block_on(
        run.docker
            .provision(&*run.clock, &*run.login, &folder(), &local(), &keys),
    );
    run.result(done);
    run.transcript()
}

/// A folder set up before, with old copies of all three keys on this
/// machine.
fn installed_with_old_copies(host: Scripted) -> Scripted {
    installed_local(LOCAL_ENV)(host)
        .with_file("/home/me/.config/kkt/docker-access-key", "old-access\n")
        .with_file(WEB_COPY, "old-web\n")
        .with_file("/home/me/.config/kkt-admin/docker-admin-key", "old-admin\n")
}

fn setup_replaces_key_copies() -> String {
    let run = Run::new(
        rules(vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])]),
        installed_with_old_copies,
    );
    run.provision(&local());
    run.transcript()
}

fn upgrade_replaces_key_copies() -> String {
    let run = Run::new(
        rules(vec![("key show --admin", vec![ok(&format!("{ADMIN}\n"))])]),
        installed_with_old_copies,
    );
    run.upgrade(Some(&keys()));
    run.transcript()
}

// ---------------------------------------------------------------- the admin key and upgrade, every way they fail

/// Answers for the server's three key reads, the most particular first, so
/// `kkt key show` stands only for the access key's read; then `extra`.
fn key_reads(
    access: Vec<Answer>,
    web: Vec<Answer>,
    admin: Vec<Answer>,
    extra: Vec<(&'static str, Vec<Answer>)>,
) -> Rules {
    let mut all = vec![
        ("key show --admin", admin),
        ("key show --web", web),
        ("kkt key show", access),
    ];
    all.extend(extra);
    rules(all)
}

fn admin_key_cannot_run() -> String {
    let with = |rules: Rules| {
        let run = Run::new(rules, installed_local(LOCAL_ENV));
        run.add_admin_key();
        run
    };
    let access = || vec![ok(&format!("{ACCESS}\n"))];
    let web = || vec![ok(&format!("{WEB}\n"))];
    let none_then_made = || vec![exit(4, "", "no admin key"), ok(&format!("{ADMIN}\n"))];
    cases(&[
        case(
            "the first access key read cannot run",
            &with(key_reads(
                vec![cannot_run("broken pipe")],
                web(),
                none_then_made(),
                vec![],
            )),
        ),
        case(
            "the admin key read cannot run",
            &with(key_reads(
                access(),
                web(),
                vec![cannot_run("broken pipe")],
                vec![],
            )),
        ),
        case(
            "making the key cannot run",
            &with(key_reads(
                access(),
                web(),
                none_then_made(),
                vec![("key create", vec![cannot_run("broken pipe")])],
            )),
        ),
        case(
            "the restart cannot run",
            &with(key_reads(
                access(),
                web(),
                none_then_made(),
                vec![("restart kkt", vec![cannot_run("broken pipe")])],
            )),
        ),
        case(
            "the access key read after the restart cannot run",
            &with(key_reads(
                vec![ok(&format!("{ACCESS}\n")), cannot_run("broken pipe")],
                web(),
                none_then_made(),
                vec![],
            )),
        ),
        case(
            "the web key read cannot run",
            &with(key_reads(
                access(),
                vec![cannot_run("broken pipe")],
                none_then_made(),
                vec![],
            )),
        ),
        case(
            "the second admin key read cannot run",
            &with(key_reads(
                access(),
                web(),
                vec![exit(4, "", "no admin key"), cannot_run("broken pipe")],
                vec![],
            )),
        ),
    ])
}

fn admin_key_more_failures() -> String {
    let with = |rules: Rules, files: Box<dyn FnOnce(Scripted) -> Scripted>| {
        let run = Run::new(rules, files);
        run.add_admin_key();
        run
    };
    let access = || vec![ok(&format!("{ACCESS}\n"))];
    let web = || vec![ok(&format!("{WEB}\n"))];
    let has_one = || vec![ok(&format!("{ADMIN}\n"))];
    let installed =
        || -> Box<dyn FnOnce(Scripted) -> Scripted> { Box::new(installed_local(LOCAL_ENV)) };
    let failing = |make: fn(Scripted) -> Scripted| -> Box<dyn FnOnce(Scripted) -> Scripted> {
        Box::new(move |host| make(installed_local(LOCAL_ENV)(host)))
    };
    cases(&[
        case(
            "the admin key read ends by a signal, having printed the key",
            &with(
                key_reads(
                    access(),
                    web(),
                    vec![Ok(Reply::signal(&format!("{ADMIN}\n")))],
                    vec![],
                ),
                installed(),
            ),
        ),
        case(
            "the admin key read prints no key",
            &with(
                key_reads(access(), web(), vec![ok("")], vec![]),
                installed(),
            ),
        ),
        case(
            "after the restart the admin key read fails, having printed the key",
            &with(
                key_reads(
                    access(),
                    web(),
                    vec![
                        exit(4, "", "no admin key"),
                        exit(1, &format!("{ADMIN}\n"), ""),
                    ],
                    vec![],
                ),
                installed(),
            ),
        ),
        case(
            "the key copies' folder cannot be made",
            &with(
                key_reads(access(), web(), has_one(), vec![]),
                failing(|host| host.failing_dir("/home/me/.config/kkt")),
            ),
        ),
        case(
            "the admin key copy's folder cannot be made",
            &with(
                key_reads(access(), web(), has_one(), vec![]),
                failing(|host| host.failing_dir("/home/me/.config/kkt-admin")),
            ),
        ),
        case(
            "the access key copy cannot be written",
            &with(
                key_reads(access(), web(), has_one(), vec![]),
                failing(|host| host.failing_write("/home/me/.config/kkt/docker-access-key", 0)),
            ),
        ),
        case(
            "the admin key copy cannot be written",
            &with(
                key_reads(access(), web(), has_one(), vec![]),
                failing(|host| {
                    host.failing_write("/home/me/.config/kkt-admin/docker-admin-key", 0)
                }),
            ),
        ),
    ])
}

fn upgrade_cannot_run() -> String {
    let with = |rules: Rules| {
        let run = Run::new(rules, installed_local(LOCAL_ENV));
        run.upgrade(Some(&keys()));
        run
    };
    let access = || vec![ok(&format!("{ACCESS}\n"))];
    let web = || vec![ok(&format!("{WEB}\n"))];
    let no_admin = || vec![exit(4, "", "no admin key")];
    cases(&[
        case(
            "the backup cannot run",
            &with(rules(vec![("kkt backup", vec![cannot_run("broken pipe")])])),
        ),
        case(
            "the pull cannot run: the settings are put back",
            &with(rules(vec![(
                "compose --project-directory /data/docker pull",
                vec![cannot_run("broken pipe")],
            )])),
        ),
        case(
            "the start cannot run: the settings are put back",
            &with(rules(vec![("up -d", vec![cannot_run("broken pipe")])])),
        ),
        case(
            "the access key read cannot run: said, and the upgrade stands",
            &with(key_reads(
                vec![cannot_run("broken pipe")],
                web(),
                no_admin(),
                vec![],
            )),
        ),
        case(
            "the web key read cannot run: said, and the upgrade stands",
            &with(key_reads(
                access(),
                vec![cannot_run("broken pipe")],
                no_admin(),
                vec![],
            )),
        ),
        case(
            "the admin key read cannot run: said, and the upgrade stands",
            &with(key_reads(
                access(),
                web(),
                vec![cannot_run("broken pipe")],
                vec![],
            )),
        ),
    ])
}

fn upgrade_more_failures() -> String {
    let with = |rules: Rules, files: Box<dyn FnOnce(Scripted) -> Scripted>| {
        let run = Run::new(rules, files);
        run.upgrade(Some(&keys()));
        run
    };
    let installed =
        || -> Box<dyn FnOnce(Scripted) -> Scripted> { Box::new(installed_local(LOCAL_ENV)) };
    cases(&[
        case(
            "the backup says nothing of where it went",
            &with(rules(vec![("kkt backup", vec![ok("done\n")])]), installed()),
        ),
        case(
            "the backup names no folder",
            &with(
                rules(vec![("kkt backup", vec![ok("Backed up to /\n")])]),
                installed(),
            ),
        ),
        case(
            "the pull ends by a signal: the settings are put back",
            &with(
                rules(vec![(
                    "compose --project-directory /data/docker pull",
                    vec![Ok(Reply::signal(""))],
                )]),
                installed(),
            ),
        ),
        case(
            "the start fails and the settings cannot be put back",
            &with(
                rules(vec![(
                    "up -d",
                    vec![exit(1, "", "port is already allocated")],
                )]),
                Box::new(|host: Scripted| installed_local(LOCAL_ENV)(host).failing_write(ENV, 1)),
            ),
        ),
        case(
            "a Tailscale install's serve folder cannot be made",
            &with(
                rules(vec![]),
                Box::new(|host: Scripted| {
                    installed_tailscale(TAILSCALE_ENV)(host).failing_dir("/data/docker/tailscale")
                }),
            ),
        ),
        case(
            "the admin key read fails, having printed the key: said, and the upgrade stands",
            &with(
                rules(vec![(
                    "key show --admin",
                    vec![exit(1, &format!("{ADMIN}\n"), "")],
                )]),
                installed(),
            ),
        ),
        case(
            "a key copy cannot be written: said, and the upgrade stands",
            &with(
                rules(vec![]),
                Box::new(|host: Scripted| {
                    installed_local(LOCAL_ENV)(host).failing_write(WEB_COPY, 0)
                }),
            ),
        ),
    ])
}

pinned! {
    setup_local => super::setup_local();
    setup_local_with_an_admin_key => super::setup_local_with_an_admin_key();
    setup_local_again => super::setup_local_again();
    setup_tailscale => super::setup_tailscale();
    setup_tailscale_with_an_auth_key => super::setup_tailscale_with_an_auth_key();
    setup_tailscale_restarts_for_a_link => super::setup_tailscale_restarts_for_a_link();
    setup_tailscale_browser_fails => super::setup_tailscale_browser_fails();
    setup_tailscale_refusals => super::setup_tailscale_refusals();
    setup_login_times_out => super::setup_login_times_out();
    setup_start_times_out => super::setup_start_times_out();
    setup_web_key_times_out => super::setup_web_key_times_out();
    setup_without_docker => super::setup_without_docker();
    setup_failures => super::setup_failures();
    setup_admin_key_reads => super::setup_admin_key_reads();
    setup_tailscale_failures => super::setup_tailscale_failures();
    admin_key_new => super::admin_key_new();
    admin_key_kept => super::admin_key_kept();
    admin_key_failures => super::admin_key_failures();
    status_not_set_up => super::status_not_set_up();
    status_local => super::status_local();
    status_tailscale_logins => super::status_tailscale_logins();
    status_tailscale_answers_without_a_state => super::status_tailscale_answers_without_a_state();
    status_name_pack => super::status_name_pack();
    status_name_pack_errors => super::status_name_pack_errors();
    existing => super::existing();
    reads => super::reads();
    compare_web_key => super::compare_web_key();
    up_and_down => super::up_and_down();
    up_and_down_cannot_run => super::up_and_down_cannot_run();
    backup => super::backup();
    upgrade_local => super::upgrade_local();
    upgrade_tailscale_with_an_admin_key => super::upgrade_tailscale_with_an_admin_key();
    upgrade_without_key_copies => super::upgrade_without_key_copies();
    upgrade_keeps_a_hand_edited_settings_file => super::upgrade_keeps_a_hand_edited_settings_file();
    upgrade_failures => super::upgrade_failures();
    run_in_server => super::run_in_server();
    run_in_server_failures => super::run_in_server_failures();
    config_home_file => super::config_home_file();
    run_in_server_cannot_run => super::run_in_server_cannot_run();
    config_home_file_cannot_run => super::config_home_file_cannot_run();
    status_name_pack_cannot_run => super::status_name_pack_cannot_run();
    setup_cannot_run => super::setup_cannot_run();
    setup_tailscale_file_failures => super::setup_tailscale_file_failures();
    setup_tailscale_link_lost => super::setup_tailscale_link_lost();
    setup_key_copy_without_a_folder => super::setup_key_copy_without_a_folder();
    setup_replaces_key_copies => super::setup_replaces_key_copies();
    upgrade_replaces_key_copies => super::upgrade_replaces_key_copies();
    admin_key_cannot_run => super::admin_key_cannot_run();
    admin_key_more_failures => super::admin_key_more_failures();
    upgrade_cannot_run => super::upgrade_cannot_run();
    upgrade_more_failures => super::upgrade_more_failures();
}
