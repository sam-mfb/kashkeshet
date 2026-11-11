//! Unit tests over [`FakeProber`], a stand-in for the machine, and
//! `FixedClock`.

use super::*;
use kkt_clock::Clock;
use kkt_clock::testing::FixedClock;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// A [`Prober`] that answers from fixed values and notes each probe.
struct FakeProber {
    /// Each key file there is; any other path cannot be looked at.
    keys: HashMap<PathBuf, KeyFile>,
    /// What each probe returns, in turn; the last one repeats.
    answers: Vec<Result<Vec<Check>, String>>,
    /// The URL and key of each probe made.
    probes: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeProber {
    fn answering(answers: Vec<Result<Vec<Check>, String>>) -> FakeProber {
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
    fn key_file(&self, path: &Path) -> Result<KeyFile, String> {
        self.keys
            .get(path)
            .cloned()
            .ok_or_else(|| "No such file or directory".to_string())
    }

    fn probe<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a> {
        let mut probes = self.probes.lock().unwrap();
        probes.push((url.to_string(), key.map(str::to_string)));
        let turn = (probes.len() - 1).min(self.answers.len() - 1);
        let answer = self.answers[turn].clone();
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
    "k".repeat(kkt_admission::testing::MIN_ACCESS_KEY_BYTES)
}

fn key(mode: Option<u32>, text: Result<String, String>) -> KeyFile {
    KeyFile { mode, text }
}

/// A server that answers and accepts the key.
fn answering() -> Vec<Check> {
    vec![
        Check::ok(SERVER_CHECK, "reachable at http://127.0.0.1:8080/mcp"),
        Check::ok("Host", "accepted"),
        Check::ok(KEY_CHECK, "MCP initialize returned 200"),
    ]
}

/// A server that does not answer.
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
    let prober = FakeProber::answering(vec![Ok(answering())]).with_key(
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
    let prober = FakeProber::answering(vec![Ok(answering())]);
    let checks = at_once(Inspector::new(&prober).server(URL, None)).unwrap();
    assert_eq!(checks[0], Check::ok("key", "disabled in the serve config"));
    assert_eq!(checks[1..], answering());
    assert_eq!(prober.probes(), [(URL.to_string(), None)]);
}

#[test]
fn a_key_file_that_cannot_be_looked_at_is_a_problem_and_the_probe_has_no_key() {
    let prober = FakeProber::answering(vec![Ok(answering())]);
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
    let prober = FakeProber::answering(vec![Ok(answering())])
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
    let prober = FakeProber::answering(vec![Ok(answering())])
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
    let prober = FakeProber::answering(vec![Ok(answering())]);
    let checks = at_once(Inspector::new(&prober).address(URL)).unwrap();
    assert_eq!(checks, answering()[..2]);
    assert_eq!(prober.probes(), [(URL.to_string(), None)]);
}

#[test]
fn inspect_with_key_probes_with_the_key_given() {
    let prober = FakeProber::answering(vec![Ok(answering())]);
    let checks = at_once(Inspector::new(&prober).server_with_key(URL, Some("typed"))).unwrap();
    assert_eq!(checks, answering());
    assert_eq!(prober.probes(), [(URL.to_string(), Some("typed".into()))]);
}

#[test]
fn a_probe_that_cannot_be_made_is_an_error() {
    let prober = FakeProber::answering(vec![Err("invalid diagnostic URL".into())]);
    assert_eq!(
        at_once(Inspector::new(&prober).server("nonsense", None)).unwrap_err(),
        "invalid diagnostic URL"
    );
    assert!(!unreachable(&answering()));
    assert!(unreachable(&silent()));
}

// Doctor's plan.

/// The server lines a probe of `answer` makes, with no key file.
fn server_lines(answer: Vec<Check>) -> ServerLines {
    let prober = FakeProber::answering(vec![Ok(answer)]);
    at_once(ServerLines::inspect(Inspector::new(&prober), URL, None)).unwrap()
}

fn line(name: &'static str) -> Check {
    Check::ok(name, "fine")
}

/// A machine with a valid web key and admin key.
fn with_keys() -> FakeProber {
    FakeProber::answering(vec![Ok(answering())])
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
        server: server_lines(answering()),
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
        server: server_lines(answering()),
        server_fix: None,
        web_key: None,
        admin_key: None,
        machine,
        unfinished: Ok(None),
        unrecorded: None,
    };
    let server = ["key", SERVER_CHECK, "Host", KEY_CHECK];
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
        server: server_lines(answering()),
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
    let checks = plan(Err("unreadable".into()), None).checks(Inspector::new(&prober));
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
    assert_eq!(checks.len(), 5);
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
    let checks = plan(silent(), Some("start Docker".into())).checks(Inspector::new(&prober));
    assert_eq!(checks[1].fix.as_deref(), Some("start Docker"));
    let checks = plan(silent(), None).checks(Inspector::new(&prober));
    assert_eq!(checks[1], silent()[0]);
    let checks = plan(answering(), Some("start Docker".into())).checks(Inspector::new(&prober));
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
    let prober = FakeProber::answering(vec![Ok(answering())]);
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
    let prober = FakeProber::answering(vec![Ok(silent()), Ok(silent()), Ok(answering())]);
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
    let prober = FakeProber::answering(vec![Ok(silent())]);
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
    let prober = FakeProber::answering(vec![Ok(silent())]);
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
    let prober = FakeProber::answering(vec![Ok(silent()), Err("invalid diagnostic URL".into())]);
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
        poll(future.as_mut()),
        Poll::Ready(Err("invalid diagnostic URL".to_string()))
    );
}
