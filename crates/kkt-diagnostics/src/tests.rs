//! Unit tests over [`FakeProber`], a stand-in for the machine.

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

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
    "k".repeat(config::MIN_ACCESS_KEY_BYTES)
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
