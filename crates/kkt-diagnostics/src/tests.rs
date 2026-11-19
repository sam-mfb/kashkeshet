//! Unit tests over [`FakeProber`], a stand-in for the machine, and
//! `FixedClock`.

use super::*;
use crate::Probing;
use kkt_clock::Clock;
use kkt_clock::testing::FixedClock;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// What one server answers: a GET, then an `initialize`.
type Server = (Answered, Initialized);

/// A [`Prober`] that answers from fixed values and notes each probe.
struct FakeProber {
    /// Each key file there is; any other path cannot be looked at.
    keys: HashMap<PathBuf, KeyFile>,
    /// What each probe finds, in turn; the last one repeats.
    answers: Vec<Result<Server, Unprobed>>,
    /// The URL and key of each probe made: the key is the one its
    /// `initialize` was given, if it got that far.
    probes: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeProber {
    fn answering(answers: Vec<Result<Server, Unprobed>>) -> FakeProber {
        FakeProber {
            keys: HashMap::new(),
            answers,
            probes: Mutex::new(Vec::new()),
        }
    }

    fn with_key(mut self, path: &str, file: KeyFile) -> FakeProber {
        self.keys.insert(PathBuf::from(path), file);
        self
    }

    fn probes(&self) -> Vec<(String, Option<String>)> {
        self.probes.lock().unwrap().clone()
    }
}

impl Prober for FakeProber {
    fn key_file(&self, path: &Path) -> Result<KeyFile, crate::KeyFileUnread> {
        self.keys
            .get(path)
            .cloned()
            .ok_or_else(|| crate::KeyFileUnread::new("No such file or directory"))
    }

    fn answer<'a>(&'a self, url: &'a str) -> Probing<'a, Answered> {
        let mut probes = self.probes.lock().unwrap();
        probes.push((url.to_string(), None));
        let turn = (probes.len() - 1).min(self.answers.len() - 1);
        let answer = self.answers[turn].clone().map(|(answered, _)| answered);
        Box::pin(async move { answer })
    }

    fn initialize<'a>(&'a self, _: &'a str, key: Option<&'a str>) -> Probing<'a, Initialized> {
        let mut probes = self.probes.lock().unwrap();
        let turn = (probes.len() - 1).min(self.answers.len() - 1);
        if let Some(last) = probes.last_mut() {
            last.1 = key.map(str::to_string);
        }
        let answer = self.answers[turn]
            .clone()
            .map(|(_, initialized)| initialized);
        Box::pin(async move { answer })
    }
}

/// The output of `future`, which must not wait: the fake prober never does.
fn at_once<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the future waited"),
    }
}

fn valid_key() -> String {
    "k".repeat(kkt_keys::testing::MIN_ACCESS_KEY_BYTES)
}

fn key(mode: Option<u32>, text: Result<String, String>) -> KeyFile {
    KeyFile {
        mode,
        text: text.map_err(crate::KeyFileUnread::new),
    }
}

fn status(code: u16, reason: &str) -> Status {
    Status {
        code,
        reason: Some(reason.into()),
    }
}

/// A server that answers and accepts the key, and gives its version.
fn up() -> Server {
    (
        Ok(status(200, "OK")),
        Initialized::Replied {
            reply: Ok(Reply::ServerInfo {
                name: Some("kashkeshet".into()),
                version: Some("1.2.3".into()),
            }),
            cleanup: Some(Ok(status(200, "OK"))),
        },
    )
}

/// A server that does not answer.
fn down() -> Server {
    (
        Err("connection refused".into()),
        Initialized::Failed("connection refused".into()),
    )
}

/// The checks of [`up`].
fn answering() -> Vec<Check> {
    vec![
        Check::ok(SERVER_CHECK, "reachable at http://127.0.0.1:8080/mcp"),
        Check::ok("Host", "accepted"),
        Check::ok(KEY_CHECK, "MCP initialize returned 200"),
        Check::ok(
            "server build",
            "version 1.2.3; commit is not exposed by MCP (see the web page's server runs)",
        ),
    ]
}

/// The checks of [`down`].
fn silent() -> Vec<Check> {
    vec![Check::problem(
        SERVER_CHECK,
        "connection refused",
        "run `kkt service start` or `kkt serve`, and check --url and the bind setting",
    )]
}

const URL: &str = "http://127.0.0.1:8080/mcp";

#[test]
fn shell_word_quotes_text_for_fix_text() {
    assert_eq!(shell_word(""), "''");
    assert_eq!(shell_word("/home/sam/.kkt/key"), "'/home/sam/.kkt/key'");
    assert_eq!(shell_word("/My Keys/$HOME"), "'/My Keys/$HOME'");
    assert_eq!(shell_word("/sam's/key"), "'/sam'\\''s/key'");
    assert_eq!(shell_word("''"), "''\\'''\\'''");
}

#[test]
fn inspect_checks_the_key_file_then_probes_with_its_key() {
    let prober = FakeProber::answering(vec![Ok(up())]).with_key(
        "/keys/key",
        key(Some(0o600), Ok(format!("{}\n", valid_key()))),
    );
    let checks =
        at_once(Inspector::new(&prober).server(URL, Some(Path::new("/keys/key")))).unwrap();
    let mut expected = vec![
        Check::ok("key file", "/keys/key"),
        Check::ok("key permissions", "0600"),
    ];
    expected.extend(answering());
    assert_eq!(checks, expected);
    assert_eq!(prober.probes(), [(URL.to_string(), Some(valid_key()))]);
}

#[test]
fn inspect_without_a_key_file_says_so_and_probes_without_a_key() {
    let prober = FakeProber::answering(vec![Ok(up())]);
    let checks = at_once(Inspector::new(&prober).server(URL, None)).unwrap();
    assert_eq!(checks[0], Check::ok("key", "disabled in the serve config"));
    assert_eq!(checks[1..], answering());
    assert_eq!(prober.probes(), [(URL.to_string(), None)]);
}

#[test]
fn a_key_file_that_cannot_be_looked_at_is_a_problem_and_the_probe_has_no_key() {
    let prober = FakeProber::answering(vec![Ok(up())]);
    let checks =
        at_once(Inspector::new(&prober).server(URL, Some(Path::new("/keys/missing")))).unwrap();
    assert_eq!(
        checks[0],
        Check::problem(
            "key file",
            "/keys/missing: No such file or directory",
            "run `kkt serve` to create the default key, or pass --key-file with an existing key"
        )
    );
    assert_eq!(checks[1..], answering());
    assert_eq!(prober.probes(), [(URL.to_string(), None)]);
}

#[test]
fn key_permissions_are_checked_only_where_the_system_has_them() {
    let prober = FakeProber::answering(vec![Ok(up())])
        .with_key("/my keys/key", key(Some(0o644), Ok(valid_key())))
        .with_key("/keys/no-mode", key(None, Ok(valid_key())));
    let inspector = Inspector::new(&prober);
    let checks = inspector.web_key(Path::new("/my keys/key"), "fix it");
    assert_eq!(
        checks,
        [
            Check::ok("web key file", "/my keys/key"),
            Check::problem("web key permissions", "0644, expected 0600", "fix it"),
        ]
    );
    let checks = inspector.admin_key(Path::new("/keys/no-mode"), "fix it");
    assert_eq!(checks, [Check::ok("admin key file", "/keys/no-mode")]);
    // Without the operator's fix, the fix is to change the permissions.
    let checks = at_once(inspector.server(URL, Some(Path::new("/my keys/key")))).unwrap();
    assert_eq!(checks[1].fix.as_deref(), Some("chmod 600 '/my keys/key'"));
}

#[test]
fn key_contents_that_are_unreadable_or_invalid_are_problems() {
    let prober = FakeProber::answering(vec![Ok(up())])
        .with_key("/keys/unreadable", key(Some(0o600), Err("denied".into())))
        .with_key("/keys/empty", key(Some(0o600), Ok("\n".into())));
    let checks = Inspector::new(&prober).admin_key(Path::new("/keys/unreadable"), "fix it");
    assert_eq!(
        checks.last(),
        Some(&Check::problem(
            "admin key contents",
            "could not read the key: denied",
            "fix it"
        ))
    );
    let checks =
        at_once(Inspector::new(&prober).server(URL, Some(Path::new("/keys/empty")))).unwrap();
    let contents = &checks[2];
    assert_eq!((contents.name, contents.ok), ("key contents", false));
    assert_eq!(
        contents.fix.as_deref(),
        Some("choose a valid key file with --key-file")
    );
    assert_eq!(prober.probes(), [(URL.to_string(), None)]);
}

#[test]
fn inspect_address_leaves_out_whether_the_key_was_accepted() {
    let prober = FakeProber::answering(vec![Ok(up())]);
    let checks = at_once(Inspector::new(&prober).address(URL)).unwrap();
    let without_the_key: Vec<Check> = answering()
        .into_iter()
        .filter(|check| check.name != KEY_CHECK)
        .collect();
    assert_eq!(checks, without_the_key);
    assert_eq!(prober.probes(), [(URL.to_string(), None)]);
}

#[test]
fn inspect_with_key_probes_with_the_key_given() {
    let prober = FakeProber::answering(vec![Ok(up())]);
    let checks = at_once(Inspector::new(&prober).server_with_key(URL, Some("typed"))).unwrap();
    assert_eq!(checks, answering());
    assert_eq!(prober.probes(), [(URL.to_string(), Some("typed".into()))]);
}

#[test]
fn a_probe_that_cannot_be_made_is_an_error() {
    let prober = FakeProber::answering(vec![Err(Unprobed::BadUrl)]);
    assert_eq!(
        at_once(Inspector::new(&prober).server("nonsense", None))
            .unwrap_err()
            .to_string(),
        "invalid diagnostic URL"
    );
    assert!(!unreachable(&answering()));
    assert!(unreachable(&silent()));
}

// Doctor's plan.

/// The server lines a probe of `answer` makes, with no key file.
fn server_lines(answer: Server) -> ServerLines {
    let prober = FakeProber::answering(vec![Ok(answer)]);
    at_once(ServerLines::inspect(Inspector::new(&prober), URL, None)).unwrap()
}

fn line(name: &'static str) -> Check {
    Check::ok(name, "fine")
}

/// A machine with a valid web key and admin key.
fn with_keys() -> FakeProber {
    FakeProber::answering(vec![Ok(up())])
        .with_key("/keys/web", key(Some(0o600), Ok(valid_key())))
        .with_key("/keys/admin", key(Some(0o600), Ok(valid_key())))
}

fn names(checks: &[Check]) -> Vec<&'static str> {
    checks.iter().map(|check| check.name).collect()
}

#[test]
fn the_plan_puts_the_server_then_the_keys_then_this_machine_then_setup() {
    let prober = with_keys();
    let plan = Plan {
        server: server_lines(up()),
        server_fix: None,
        web_key: Some(KeyToCheck {
            path: Path::new("/keys/web"),
            fix: "web fix".into(),
        }),
        admin_key: Some(KeyToCheck {
            path: Path::new("/keys/admin"),
            fix: "admin fix".into(),
        }),
        machine: ThisMachine::Docker {
            kkt: line("docker"),
            tailscale_login: Some(line("tailscale login")),
            web_key_copy: Some(line("web key copy")),
        },
        unfinished: Ok(Some("setup stopped".into())),
        unrecorded: Some(line("unrecorded")),
    };
    assert_eq!(
        names(&plan.checks(Inspector::new(&prober))),
        [
            "key",
            SERVER_CHECK,
            "Host",
            KEY_CHECK,
            "server build",
            "web key file",
            "web key permissions",
            "admin key file",
            "admin key permissions",
            "docker",
            "tailscale login",
            "web key copy",
            SETUP_CHECK,
            "unrecorded",
        ]
    );
}

#[test]
fn each_machine_line_the_program_supplies_appears_only_when_given() {
    let prober = with_keys();
    let plan = |machine| Plan {
        server: server_lines(up()),
        server_fix: None,
        web_key: None,
        admin_key: None,
        machine,
        unfinished: Ok(None),
        unrecorded: None,
    };
    let server = ["key", SERVER_CHECK, "Host", KEY_CHECK, "server build"];
    let checks = plan(ThisMachine::Elsewhere).checks(Inspector::new(&prober));
    assert_eq!(names(&checks), server);
    let checks = plan(ThisMachine::Service(line("service"))).checks(Inspector::new(&prober));
    assert_eq!(names(&checks), [&server[..], &["service"]].concat());
    let docker_alone = ThisMachine::Docker {
        kkt: line("docker"),
        tailscale_login: None,
        web_key_copy: None,
    };
    let checks = plan(docker_alone).checks(Inspector::new(&prober));
    assert_eq!(names(&checks), [&server[..], &["docker"]].concat());
    let with_login = ThisMachine::Docker {
        kkt: line("docker"),
        tailscale_login: Some(line("tailscale login")),
        web_key_copy: None,
    };
    let checks = plan(with_login).checks(Inspector::new(&prober));
    assert_eq!(
        names(&checks),
        [&server[..], &["docker", "tailscale login"]].concat()
    );
    // The program's lines are kept as given.
    let service = Check::problem("service", "not running", "start it");
    let checks = plan(ThisMachine::Service(service.clone())).checks(Inspector::new(&prober));
    assert_eq!(checks.last(), Some(&service));
}

#[test]
fn an_unfinished_setup_is_a_problem_and_so_is_one_that_cannot_be_read() {
    let prober = with_keys();
    let plan = |unfinished, unrecorded| Plan {
        server: server_lines(up()),
        server_fix: None,
        web_key: None,
        admin_key: None,
        machine: ThisMachine::Elsewhere,
        unfinished,
        unrecorded,
    };
    let checks = plan(Ok(Some("setup stopped".into())), None).checks(Inspector::new(&prober));
    assert_eq!(
        checks.last(),
        Some(&Check::problem(
            SETUP_CHECK,
            "setup stopped",
            "run `kkt setup` again to finish it"
        ))
    );
    let checks = plan(Err(crate::Unknown::new("unreadable")), None).checks(Inspector::new(&prober));
    assert_eq!(
        checks.last(),
        Some(&Check::problem(
            SETUP_CHECK,
            "unreadable",
            "run `kkt setup` again"
        ))
    );
    let unrecorded = Check::ok(SETUP_CHECK, "found a service");
    let checks = plan(Ok(None), Some(unrecorded.clone())).checks(Inspector::new(&prober));
    assert_eq!(checks.last(), Some(&unrecorded));
    assert_eq!(checks.len(), 6);
}

#[test]
fn the_server_fix_replaces_only_a_failed_server_lines_fix() {
    let prober = with_keys();
    let plan = |answer, server_fix| Plan {
        server: server_lines(answer),
        server_fix,
        web_key: None,
        admin_key: None,
        machine: ThisMachine::Elsewhere,
        unfinished: Ok(None),
        unrecorded: None,
    };
    let checks = plan(down(), Some("start Docker".into())).checks(Inspector::new(&prober));
    assert_eq!(checks[1].fix.as_deref(), Some("start Docker"));
    let checks = plan(down(), None).checks(Inspector::new(&prober));
    assert_eq!(checks[1], silent()[0]);
    let checks = plan(up(), Some("start Docker".into())).checks(Inspector::new(&prober));
    assert!(checks.iter().all(|check| check.fix.is_none()));
}

#[test]
fn the_report_has_a_line_for_each_check_and_a_fix_under_each_problem() {
    let (text, all_ok) = report(vec![
        Check::ok("server", "reachable"),
        Check::problem("key file", "missing", "make one"),
    ]);
    assert_eq!(
        text,
        "ok: server: reachable\nproblem: key file: missing\n  fix: make one\n"
    );
    assert!(!all_ok);
    let (text, all_ok) = report(vec![Check::ok("server", "reachable")]);
    assert_eq!(text, "ok: server: reachable\n");
    assert!(all_ok);
    assert_eq!(report(Vec::new()), (String::new(), true));
}

// Setup's wait for a new server.

/// `future` polled once, without waiting.
fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn clock() -> FixedClock {
    FixedClock::new(kkt_clock::from_text("2026-01-01T00:00:00Z").unwrap())
}

fn advance(clock: &FixedClock, seconds: u64) {
    let now = clock.now();
    clock.advance((now + Duration::from_secs(seconds)) - now);
}

const SIX_EVERY_TWO: Wait = Wait {
    total: Duration::from_secs(6),
    every: Duration::from_secs(2),
};

#[test]
fn setup_waits_for_a_new_server_and_not_for_one_elsewhere() {
    assert_eq!(
        setup_wait(Settings::DEFAULT, SetupServer::New),
        Wait {
            total: Settings::DEFAULT.setup_server_wait,
            every: Settings::DEFAULT.setup_server_poll,
        }
    );
    assert_eq!(
        setup_wait(Settings::DEFAULT, SetupServer::Elsewhere),
        Wait {
            total: Duration::ZERO,
            every: Settings::DEFAULT.setup_server_poll,
        }
    );
}

#[test]
fn a_server_that_answers_at_once_is_not_waited_for() {
    let clock = clock();
    let prober = FakeProber::answering(vec![Ok(up())]);
    let mut said = 0;
    let checks = at_once(until_reachable(
        Inspector::new(&prober),
        &clock,
        URL,
        None,
        SIX_EVERY_TWO,
        &mut || said += 1,
    ))
    .unwrap();
    assert_eq!(checks[1..], answering());
    assert_eq!(prober.probes().len(), 1);
    assert_eq!(said, 0);
}

#[test]
fn a_server_that_answers_after_some_tries_is_tried_every_step_until_it_does() {
    let clock = clock();
    let prober = FakeProber::answering(vec![Ok(down()), Ok(down()), Ok(up())]);
    let said = Cell::new(0);
    let mut waiting = || said.set(said.get() + 1);
    let mut future = std::pin::pin!(until_reachable(
        Inspector::new(&prober),
        &clock,
        URL,
        None,
        SIX_EVERY_TWO,
        &mut waiting,
    ));
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!((prober.probes().len(), clock.sleepers()), (1, 1));
    // Not before the step is up.
    advance(&clock, 1);
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!(prober.probes().len(), 1);
    advance(&clock, 1);
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!(prober.probes().len(), 2);
    advance(&clock, 2);
    let Poll::Ready(checks) = poll(future.as_mut()) else {
        panic!("the server answered");
    };
    assert_eq!(checks.unwrap()[1..], answering());
    assert_eq!(prober.probes().len(), 3);
    assert_eq!(said.get(), 1);
}

#[test]
fn a_server_that_never_answers_is_tried_until_the_wait_is_up() {
    let clock = clock();
    let prober = FakeProber::answering(vec![Ok(down())]);
    let said = Cell::new(0);
    let mut waiting = || said.set(said.get() + 1);
    let mut future = std::pin::pin!(until_reachable(
        Inspector::new(&prober),
        &clock,
        URL,
        None,
        SIX_EVERY_TWO,
        &mut waiting,
    ));
    for tries in 1..=3 {
        assert!(poll(future.as_mut()).is_pending());
        assert_eq!(prober.probes().len(), tries);
        advance(&clock, 2);
    }
    // The try at six seconds is the last.
    let Poll::Ready(checks) = poll(future.as_mut()) else {
        panic!("the wait is up");
    };
    assert_eq!(checks.unwrap()[1..], silent());
    assert_eq!(prober.probes().len(), 4);
    assert_eq!(said.get(), 1);
}

#[test]
fn a_server_elsewhere_that_does_not_answer_is_tried_once() {
    let clock = clock();
    let prober = FakeProber::answering(vec![Ok(down())]);
    let mut said = 0;
    let checks = at_once(until_reachable(
        Inspector::new(&prober),
        &clock,
        URL,
        None,
        setup_wait(Settings::DEFAULT, SetupServer::Elsewhere),
        &mut || said += 1,
    ))
    .unwrap();
    assert!(unreachable(&checks));
    assert_eq!(prober.probes().len(), 1);
    assert_eq!(said, 0);
}

#[test]
fn a_probe_that_cannot_be_made_while_waiting_ends_the_wait() {
    let clock = clock();
    let prober = FakeProber::answering(vec![Ok(down()), Err(Unprobed::BadUrl)]);
    let mut waiting = || {};
    let mut future = std::pin::pin!(until_reachable(
        Inspector::new(&prober),
        &clock,
        URL,
        None,
        SIX_EVERY_TWO,
        &mut waiting,
    ));
    assert!(poll(future.as_mut()).is_pending());
    advance(&clock, 2);
    assert_eq!(
        poll(future.as_mut()).map(|done| done.map_err(|e| e.to_string())),
        Poll::Ready(Err("invalid diagnostic URL".to_string()))
    );
}

// The server checks, from each fact a probe can report. The details and
// fixes are those `kkt doctor` has always printed.

/// The checks of a server at [`URL`] answering as `server` does, with a key.
fn checks_of(server: Server) -> Vec<Check> {
    let prober = FakeProber::answering(vec![Ok(server)]);
    at_once(Inspector::new(&prober).server_with_key(URL, Some("k"))).unwrap()
}

fn replied(
    reply: Result<Reply, ReplyError>,
    cleanup: Option<Result<Status, String>>,
) -> Initialized {
    Initialized::Replied { reply, cleanup }
}

const REACHABLE: &str = "reachable at http://127.0.0.1:8080/mcp";

#[test]
fn a_server_that_refuses_its_host_name_stops_the_checks() {
    assert_eq!(
        checks_of((
            Ok(status(403, "Forbidden")),
            Initialized::Failed("unused".into())
        )),
        [
            Check::ok(SERVER_CHECK, REACHABLE),
            Check::problem(
                "Host",
                "the server refused this host name",
                "add \"127.0.0.1\" to allow_hosts in the serve config, then restart"
            ),
        ]
    );
}

#[test]
fn an_initialize_that_fails_or_is_refused_is_a_problem() {
    let host_ok = || {
        vec![
            Check::ok(SERVER_CHECK, REACHABLE),
            Check::ok("Host", "accepted"),
        ]
    };
    let with = |last: Check| {
        let mut checks = host_ok();
        checks.push(last);
        checks
    };
    let ok = || Ok(status(200, "OK"));
    assert_eq!(
        checks_of((ok(), Initialized::Failed("connection reset".into()))),
        with(Check::problem(
            "MCP",
            "connection reset",
            "check the MCP URL and server logs"
        ))
    );
    assert_eq!(
        checks_of((ok(), Initialized::Refused(status(401, "Unauthorized")))),
        with(Check::problem(
            KEY_CHECK,
            "server returned 401",
            "pass --key-file with the key used by the running server, or restart it after changing keys"
        ))
    );
    assert_eq!(
        checks_of((ok(), Initialized::Refused(status(404, "Not Found")))),
        with(Check::problem(
            "MCP",
            "server returned 404 Not Found",
            "check that --url points at the server's /mcp endpoint"
        ))
    );
    let odd = Status {
        code: 599,
        reason: None,
    };
    assert_eq!(
        checks_of((ok(), Initialized::Refused(odd))),
        with(Check::problem(
            "MCP",
            "server returned 599 <unknown status code>",
            "check that --url points at the server's /mcp endpoint"
        ))
    );
}

#[test]
fn a_reply_without_a_result_or_server_info_is_a_problem() {
    let ok = || Ok(status(200, "OK"));
    let last = |initialized: Initialized| checks_of((ok(), initialized)).pop().unwrap();
    for (error, detail) in [
        (
            ReplyError::TooLarge,
            "MCP response exceeded the diagnostic size limit",
        ),
        (
            ReplyError::NoResult,
            "the endpoint did not return an MCP initialize result",
        ),
        (
            ReplyError::Read("stream ended".into()),
            "could not read MCP response: stream ended",
        ),
    ] {
        assert_eq!(
            last(replied(Err(error), None)),
            Check::problem("MCP", detail, "check --url and the server logs")
        );
    }
    assert_eq!(
        last(replied(Ok(Reply::NoServerInfo), None)),
        Check::problem(
            "MCP",
            "initialize did not return serverInfo",
            "check --url and the server's MCP implementation"
        )
    );
    let unversioned = Reply::ServerInfo {
        name: None,
        version: None,
    };
    assert_eq!(
        last(replied(Ok(unversioned), None)),
        Check::ok(
            "server build",
            "version not reported; commit is not exposed by MCP (see the web page's server runs)"
        )
    );
}

#[test]
fn a_session_that_does_not_close_is_a_problem_before_the_reply_checks() {
    let ok = || Ok(status(200, "OK"));
    let info = || {
        Ok(Reply::ServerInfo {
            name: None,
            version: Some("1.2.3".into()),
        })
    };
    let cleanup = |checks: Vec<Check>| checks[2].clone();
    assert_eq!(
        cleanup(checks_of((
            ok(),
            replied(info(), Some(Ok(status(500, "Internal Server Error"))))
        ))),
        Check::problem(
            "probe cleanup",
            "server returned 500 Internal Server Error",
            "check the server logs"
        )
    );
    assert_eq!(
        cleanup(checks_of((
            ok(),
            replied(info(), Some(Err("connection closed".into())))
        ))),
        Check::problem(
            "probe cleanup",
            "connection closed",
            "check the server logs"
        )
    );
    // A session closed with any 2xx, or none opened, adds no check.
    for closed in [Some(Ok(status(204, "No Content"))), None] {
        assert_eq!(checks_of((ok(), replied(info(), closed))).len(), 4);
    }
}

#[test]
fn a_probe_that_cannot_be_made_says_why() {
    for (why, said) in [
        (Unprobed::BadUrl, "invalid diagnostic URL".to_string()),
        (
            Unprobed::NoClient("no TLS backend".into()),
            "could not construct the diagnostic HTTP client: no TLS backend".to_string(),
        ),
    ] {
        let prober = FakeProber::answering(vec![Err(why)]);
        assert_eq!(
            at_once(Inspector::new(&prober).server_with_key(URL, None)).map_err(|e| e.to_string()),
            Err(said)
        );
    }
}

#[test]
fn the_doctor_report_reads_as_it_always_has() {
    let prober = FakeProber::answering(vec![Ok(up())]).with_key(
        "/keys/key",
        key(Some(0o600), Ok(format!("{}\n", valid_key()))),
    );
    let checks =
        at_once(Inspector::new(&prober).server(URL, Some(Path::new("/keys/key")))).unwrap();
    assert_eq!(
        report(checks),
        (
            "ok: key file: /keys/key\n\
             ok: key permissions: 0600\n\
             ok: server: reachable at http://127.0.0.1:8080/mcp\n\
             ok: Host: accepted\n\
             ok: key accepted: MCP initialize returned 200\n\
             ok: server build: version 1.2.3; commit is not exposed by MCP (see the web page's server runs)\n"
                .to_string(),
            true
        )
    );
    let prober = FakeProber::answering(vec![Ok(down())]);
    let checks = at_once(Inspector::new(&prober).server(URL, None)).unwrap();
    assert_eq!(
        report(checks),
        (
            "ok: key: disabled in the serve config\n\
             problem: server: connection refused\n  \
             fix: run `kkt service start` or `kkt serve`, and check --url and the bind setting\n"
                .to_string(),
            false
        )
    );
}
