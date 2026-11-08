use super::launchd::PrintedState;
use super::*;
use kkt_clock::testing::FixedClock;
use kkt_clock::{Clock, Span};
use kkt_config::{LAUNCHD_RELOAD_RETRY, LAUNCHD_RELOAD_WAIT};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// A system that records what is asked of it. Commands succeed with no
/// output unless given an answer.
#[derive(Default)]
struct Fake {
    files: RefCell<BTreeMap<PathBuf, String>>,
    dirs: RefCell<BTreeSet<PathBuf>>,
    ran: RefCell<Vec<String>>,
    answers: HashMap<String, Finished>,
    /// Answers given in turn, each once, before `answers`.
    in_turn: RefCell<HashMap<String, VecDeque<Finished>>>,
    /// Commands that fail until [`Fake::recover`].
    failing: RefCell<BTreeSet<String>>,
    /// Paths that can't be inspected, as when a folder above them can't be
    /// read: checking, reading or removing one is denied.
    denied: BTreeSet<PathBuf>,
}

impl Fake {
    /// `command` fails, as a service manager's can, until
    /// [`Fake::recover`].
    fn failing(self, command: &str) -> Fake {
        self.failing.borrow_mut().insert(command.to_string());
        self
    }

    /// Failing commands work again.
    fn recover(&self) {
        self.failing.borrow_mut().clear();
    }

    fn answer(mut self, command: &str, code: i32, stdout: &str, stderr: &str) -> Fake {
        self.answers.insert(
            command.to_string(),
            Finished {
                code: Some(code),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            },
        );
        self
    }

    /// `command`'s next answer, used once, before any from [`Fake::answer`].
    /// Called again, it adds the answer after those already given.
    fn then_answer(self, command: &str, code: i32, stderr: &str) -> Fake {
        self.in_turn
            .borrow_mut()
            .entry(command.to_string())
            .or_default()
            .push_back(Finished {
                code: Some(code),
                stdout: String::new(),
                stderr: stderr.to_string(),
            });
        self
    }

    /// launchd answers that it doesn't have the service loaded.
    fn not_loaded(self) -> Fake {
        self.answer(
            PRINT,
            113,
            "",
            "Bad request.\nCould not find service \"com.sam-mfb.kashkeshet\" in domain for user gui: 501\n",
        )
    }

    fn with_file(self, path: &str) -> Fake {
        self.files
            .borrow_mut()
            .insert(PathBuf::from(path), String::new());
        self
    }

    /// `path` can't be inspected.
    fn denying(mut self, path: &str) -> Fake {
        self.denied.insert(PathBuf::from(path));
        self
    }

    /// Denied, if `path` can't be inspected.
    fn check(&self, path: &Path) -> io::Result<()> {
        if self.denied.contains(path) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        Ok(())
    }

    fn ran(&self) -> Vec<String> {
        self.ran.borrow().clone()
    }

    fn file(&self, path: &str) -> Option<String> {
        self.files.borrow().get(Path::new(path)).cloned()
    }
}

impl System for Fake {
    fn user_id(&self) -> io::Result<u32> {
        Ok(501)
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        self.check(path)?;
        Ok(self.files.borrow().contains_key(path) || self.dirs.borrow().contains(path))
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.check(path)?;
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.dirs.borrow_mut().insert(path.to_path_buf());
        Ok(())
    }

    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_string());
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.check(path)?;
        self.files.borrow_mut().remove(path);
        Ok(())
    }

    fn run(&self, command: &Command) -> io::Result<Finished> {
        let shown = command.to_string();
        self.ran.borrow_mut().push(shown.clone());
        if self.failing.borrow().contains(&shown) {
            return Ok(Finished {
                code: Some(1),
                stdout: String::new(),
                stderr: "Job failed.".to_string(),
            });
        }
        if let Some(next) = self
            .in_turn
            .borrow_mut()
            .get_mut(&shown)
            .and_then(VecDeque::pop_front)
        {
            return Ok(next);
        }
        Ok(self.answers.get(&shown).cloned().unwrap_or(Finished {
            code: Some(0),
            ..Finished::default()
        }))
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        self.run(command).map(|f| Finished {
            code: f.code,
            ..Finished::default()
        })
    }
}

const PLIST: &str = "/Users/sam/Library/LaunchAgents/com.sam-mfb.kashkeshet.plist";
const MAC_LOG: &str = "/Users/sam/Library/Logs/kashkeshet/kashkeshet.log";
const PRINT: &str = "launchctl print gui/501/com.sam-mfb.kashkeshet";
const UNIT: &str = "/home/sam/.config/systemd/user/kashkeshet.service";
const BOOTOUT: &str = "launchctl bootout gui/501/com.sam-mfb.kashkeshet";
/// What `launchctl bootstrap` says while launchd is still removing the
/// service just booted out (#369).
const REMOVING: &str = "Bootstrap failed: 5: Input/output error\n";

fn bootstrap() -> String {
    format!("launchctl bootstrap gui/501 {PLIST}")
}

fn clock() -> FixedClock {
    FixedClock::new(kkt_clock::from_text("2026-01-01T00:00:00Z").expect("a time"))
}

fn advance(clock: &FixedClock, by: Duration) {
    clock.advance(Span::from_std(by).expect("a span"));
}

/// `future` polled once, without waiting.
fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

/// `future`, which must finish without waiting.
fn at_once<F: Future>(future: F) -> F::Output {
    match poll(std::pin::pin!(future)) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the future waited"),
    }
}

/// Installs `spec` with `manager`, which must not wait on the clock.
fn install(
    manager: &Manager,
    system: &dyn System,
    spec: &Spec,
) -> Result<Vec<String>, ServiceError> {
    at_once(manager.install(system, &clock(), spec))
}

fn mac() -> Manager {
    Manager::launchd(Path::new("/Users/sam"))
}

fn linux() -> Manager {
    Manager::systemd(Path::new("/home/sam/.config"))
}

fn mac_spec() -> Spec {
    Spec {
        program: PathBuf::from("/Users/sam/.cargo/bin/kkt"),
        args: [
            "serve",
            "--ignore-setup",
            "--config",
            "/Users/sam/.config/kkt/config.toml",
            "--db",
            "/Users/sam/Library/Application Support/kashkeshet/kashkeshet.db",
        ]
        .map(String::from)
        .to_vec(),
        dirs: vec![PathBuf::from(
            "/Users/sam/Library/Application Support/kashkeshet",
        )],
    }
}

fn linux_spec() -> Spec {
    Spec {
        program: PathBuf::from("/home/sam/.cargo/bin/kkt"),
        args: [
            "serve",
            "--ignore-setup",
            "--config",
            "/home/sam/.config/kkt/config.toml",
            "--db",
            "/home/sam/.local/share/kashkeshet/kashkeshet.db",
        ]
        .map(String::from)
        .to_vec(),
        dirs: vec![PathBuf::from("/home/sam/.local/share/kashkeshet")],
    }
}

/// Compares `actual` with the golden file `name`. With UPDATE_GOLDEN=1,
/// rewrites the file instead; review the diff.
fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).expect("write golden file");
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("read golden file");
    assert_eq!(
        actual, expected,
        "{name} changed; if intended, run UPDATE_GOLDEN=1 cargo test and review the diff"
    );
}

#[test]
fn the_plist_matches_its_golden_file() {
    golden(
        "com.sam-mfb.kashkeshet.plist",
        &mac().render(&mac_spec()).expect("plist"),
    );
}

#[test]
fn the_unit_matches_its_golden_file() {
    golden(
        "kashkeshet.service",
        &linux().render(&linux_spec()).expect("unit"),
    );
}

#[test]
fn plist_values_are_escaped() {
    let spec = Spec {
        program: PathBuf::from("/opt/a&b/kkt"),
        args: vec!["<x>".into(), "it's \"q\"".into()],
        dirs: vec![],
    };
    let plist = mac().render(&spec).expect("plist");
    assert!(
        plist.contains("<string>/opt/a&amp;b/kkt</string>"),
        "{plist}"
    );
    assert!(plist.contains("<string>&lt;x&gt;</string>"), "{plist}");
    assert!(
        plist.contains("<string>it&apos;s &quot;q&quot;</string>"),
        "{plist}"
    );
}

#[test]
fn unit_arguments_are_quoted_and_literal() {
    let spec = Spec {
        program: PathBuf::from("/opt/my kkt/kkt"),
        args: vec!["50%".into(), "$HOME".into(), r#"a\"b"#.into()],
        dirs: vec![],
    };
    let unit = linux().render(&spec).expect("unit");
    assert!(
        unit.contains(r#"ExecStart="/opt/my kkt/kkt" "50%%" "$$HOME" "a\\\"b""#),
        "{unit}"
    );
}

#[test]
fn the_installed_args_are_read_back_as_they_were_given() {
    let awkward = Spec {
        program: PathBuf::from("/opt/my kkt/kkt"),
        args: [
            "serve",
            "50%",
            "$HOME",
            r#"a\"b"#,
            "<x> & 'y'",
            "two  spaces",
            "",
        ]
        .map(String::from)
        .to_vec(),
        dirs: vec![],
    };
    for (manager, path, spec) in [
        (mac(), PLIST, mac_spec()),
        (linux(), UNIT, linux_spec()),
        (mac(), PLIST, awkward.clone()),
        (linux(), UNIT, awkward),
    ] {
        let system = Fake::default();
        assert_eq!(manager.installed_args(&system).expect("read"), None);
        system
            .write_file(Path::new(path), &manager.render(&spec).expect("render"))
            .expect("write");
        assert_eq!(
            manager.installed_args(&system).expect("read"),
            Some(spec.args)
        );
    }
}

#[test]
fn a_plist_without_the_log_file_is_read_as_it_is() {
    let system = Fake::default();
    let plist = mac().render(&mac_spec()).expect("plist");
    let older = plist.replace(
        &format!("    <string>--log-file</string>\n    <string>{MAC_LOG}</string>\n"),
        "",
    );
    assert_ne!(older, plist);
    system.write_file(Path::new(PLIST), &older).expect("write");
    assert_eq!(
        mac().installed_args(&system).expect("read"),
        Some(mac_spec().args)
    );
}

#[test]
fn a_service_file_kkt_did_not_write_is_not_read() {
    for (manager, path, text) in [
        (mac(), PLIST, "<plist/>"),
        (linux(), UNIT, "[Service]\n"),
        (linux(), UNIT, "ExecStart=/usr/bin/kkt serve\n"),
        (linux(), UNIT, "ExecStart=\"kkt\"\"serve\"\n"),
        (linux(), UNIT, "ExecStart=\"kkt\" \"50%\"\n"),
        (linux(), UNIT, "ExecStart=\"kkt\n"),
        (linux(), UNIT, "ExecStart=\n"),
    ] {
        let system = Fake::default();
        system.write_file(Path::new(path), text).expect("write");
        assert!(
            matches!(
                manager.installed_args(&system),
                Err(ServiceError::Unreadable { .. })
            ),
            "{text}"
        );
    }
}

#[test]
fn control_characters_are_refused() {
    let spec = Spec {
        program: PathBuf::from("/opt/kkt"),
        args: vec!["--db".into(), "a\nExecStartPre=/bin/evil".into()],
        dirs: vec![],
    };
    for manager in [mac(), linux()] {
        assert!(matches!(
            manager.render(&spec),
            Err(ServiceError::BadValue { .. })
        ));
    }
}

#[test]
fn launchd_install_writes_the_plist_and_loads_it() {
    let system = Fake::default().not_loaded();
    let said = install(&mac(), &system, &mac_spec()).expect("install");
    assert_eq!(
        system.file(PLIST),
        Some(mac().render(&mac_spec()).expect("plist"))
    );
    for dir in [
        "/Users/sam/Library/LaunchAgents",
        "/Users/sam/Library/Logs/kashkeshet",
        "/Users/sam/Library/Application Support/kashkeshet",
    ] {
        assert!(system.exists(Path::new(dir)).expect("check"), "{dir}");
    }
    assert_eq!(
        system.ran(),
        [PRINT, &format!("launchctl bootstrap gui/501 {PLIST}")]
    );
    assert!(said.iter().any(|l| l.contains(MAC_LOG)), "{said:?}");
}

#[test]
fn launchd_install_again_unloads_the_old_one_first() {
    let system = Fake::default();
    install(&mac(), &system, &mac_spec()).expect("install");
    assert_eq!(system.ran(), [PRINT, BOOTOUT, &bootstrap()]);
}

/// launchd removes a booted-out service after `bootout` returns, and a
/// `bootstrap` before it has finished fails (#369). The install tries
/// again, on the clock, until it loads.
#[test]
fn launchd_install_tries_again_while_launchd_removes_the_old_service() {
    let bootstrap = bootstrap();
    let system = Fake::default()
        .then_answer(&bootstrap, 5, REMOVING)
        .then_answer(&bootstrap, 5, REMOVING);
    let clock = clock();
    let (manager, spec) = (mac(), mac_spec());
    let mut future = std::pin::pin!(manager.install(&system, &clock, &spec));
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!(system.ran(), [PRINT, BOOTOUT, &bootstrap]);
    assert_eq!(clock.sleepers(), 1);
    // Not before the retry interval is up.
    advance(&clock, LAUNCHD_RELOAD_RETRY / 2);
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!(system.ran().len(), 3);
    advance(&clock, LAUNCHD_RELOAD_RETRY / 2);
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!(system.ran().len(), 4);
    advance(&clock, LAUNCHD_RELOAD_RETRY);
    let Poll::Ready(said) = poll(future.as_mut()) else {
        panic!("the third bootstrap works");
    };
    let said = said.expect("install");
    assert_eq!(
        system.ran(),
        [PRINT, BOOTOUT, &bootstrap, &bootstrap, &bootstrap]
    );
    assert!(
        said.iter().any(|l| l.starts_with("Started kashkeshet")),
        "{said:?}"
    );
}

/// A `bootstrap` that fails for the whole wait is given up, with the
/// command that finishes the install.
#[test]
fn launchd_install_gives_up_after_the_wait_and_says_how_to_finish() {
    let bootstrap = bootstrap();
    let system = Fake::default().answer(&bootstrap, 5, "", REMOVING);
    let clock = clock();
    let began = clock.now();
    let (manager, spec) = (mac(), mac_spec());
    let mut future = std::pin::pin!(manager.install(&system, &clock, &spec));
    let err = loop {
        match poll(future.as_mut()) {
            Poll::Ready(result) => break result.expect_err("bootstrap never works"),
            Poll::Pending => advance(&clock, LAUNCHD_RELOAD_RETRY),
        }
    };
    // One try at the start, then one each interval, the last as the wait
    // runs out.
    let expected = (LAUNCHD_RELOAD_WAIT.as_millis() / LAUNCHD_RELOAD_RETRY.as_millis()) + 1;
    let tries = system.ran().iter().filter(|c| **c == bootstrap).count();
    assert_eq!(tries as u128, expected);
    assert_eq!(clock.now(), began + LAUNCHD_RELOAD_WAIT);
    assert!(
        matches!(
            &err,
            ServiceError::ReloadFailed { command, tries, waited, .. }
                if *command == bootstrap
                    && u128::from(*tries) == expected
                    && *waited == LAUNCHD_RELOAD_WAIT
        ),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(
        text.starts_with(&format!(
            "`{bootstrap}` failed on each of {expected} tries over {} seconds: \
             Bootstrap failed: 5: Input/output error. ",
            LAUNCHD_RELOAD_WAIT.as_secs()
        )),
        "{text}"
    );
    assert!(
        text.contains("kashkeshet is stopped: ")
            && text.contains("Run `kkt service start` to load it"),
        "{text}"
    );
}

#[test]
fn a_failed_command_is_an_error_saying_what_it_said() {
    let bootstrap = format!("launchctl bootstrap gui/501 {PLIST}");
    let system = Fake::default().not_loaded().answer(
        &bootstrap,
        5,
        "",
        "Bootstrap failed: 5: Input/output error\n",
    );
    let err = install(&mac(), &system, &mac_spec()).expect_err("fails");
    assert_eq!(
        err.to_string(),
        format!("`{bootstrap}` failed: Bootstrap failed: 5: Input/output error")
    );
    // Nothing was unloaded, so there was nothing to wait for: one try.
    assert_eq!(system.ran(), [PRINT, &bootstrap]);
}

#[test]
fn launchd_uninstall_unloads_and_removes() {
    let system = Fake::default().with_file(PLIST);
    mac().uninstall(&system).expect("uninstall");
    assert_eq!(
        system.ran(),
        [PRINT, "launchctl bootout gui/501/com.sam-mfb.kashkeshet"]
    );
    assert_eq!(system.file(PLIST), None);

    let nothing = Fake::default().not_loaded();
    let said = mac().uninstall(&nothing).expect("uninstall");
    assert_eq!(said, ["The service was not installed."]);
}

#[test]
fn launchd_start_kicks_a_loaded_service_and_loads_an_unloaded_one() {
    let loaded = Fake::default().with_file(PLIST);
    mac().start(&loaded).expect("start");
    assert_eq!(
        loaded.ran(),
        [PRINT, "launchctl kickstart gui/501/com.sam-mfb.kashkeshet"]
    );

    let unloaded = Fake::default().with_file(PLIST).not_loaded();
    mac().start(&unloaded).expect("start");
    assert_eq!(
        unloaded.ran(),
        [PRINT, &format!("launchctl bootstrap gui/501 {PLIST}")]
    );
}

#[test]
fn start_and_stop_need_an_installed_service() {
    for manager in [mac(), linux()] {
        let system = Fake::default();
        assert!(matches!(
            manager.start(&system),
            Err(ServiceError::NotInstalled)
        ));
        assert!(matches!(
            manager.stop(&system),
            Err(ServiceError::NotInstalled)
        ));
        assert!(system.ran().is_empty());
    }
}

#[test]
fn launchd_stop_sends_sigterm() {
    let system = Fake::default().with_file(PLIST);
    mac().stop(&system).expect("stop");
    assert_eq!(
        system.ran(),
        [
            PRINT,
            "launchctl kill SIGTERM gui/501/com.sam-mfb.kashkeshet"
        ]
    );
}

#[test]
fn launchd_status_reports_state_pid_and_last_exit() {
    let printed = "gui/501/com.sam-mfb.kashkeshet = {\n\tactive count = 1\n\tstate = running\n\
                   \tpid = 4242\n\tlast exit code = 0\n\tendpoints = {\n\t\tstate = active\n\t}\n}\n";
    let system = Fake::default()
        .with_file(PLIST)
        .answer(PRINT, 0, printed, "");
    let said = mac().status(&system).expect("status");
    assert_eq!(
        said,
        [
            format!("Installed: {PLIST}"),
            "State: running (pid 4242)".to_string(),
            "Last exit: 0".to_string(),
            format!("Log: {MAC_LOG}"),
        ]
    );
}

#[test]
fn launchd_status_of_a_missing_or_unloaded_service() {
    let said = mac().status(&Fake::default()).expect("status");
    assert!(said[0].starts_with("Not installed"), "{said:?}");

    let unloaded = Fake::default().with_file(PLIST).not_loaded();
    let said = mac().status(&unloaded).expect("status");
    assert!(said[1].starts_with("Not loaded"), "{said:?}");
}

#[test]
fn the_first_of_each_printed_key_wins() {
    let state = PrintedState::read("\tstate = waiting\n\t\tstate = active\n\tpid = 7\n");
    assert_eq!(state.state.as_deref(), Some("waiting"));
    assert_eq!(state.pid.as_deref(), Some("7"));
    assert_eq!(state.last_exit, None);
}

#[test]
fn launchd_logs_tail_the_log_file() {
    let system = Fake::default().with_file(MAC_LOG);
    mac().logs(&system, 50, true).expect("logs");
    assert_eq!(system.ran(), [format!("tail -n 50 -F {MAC_LOG}")]);

    let said = mac().logs(&Fake::default(), 50, false).expect("logs");
    assert_eq!(said, [format!("No log yet at {MAC_LOG}")]);
}

#[test]
fn systemd_install_writes_enables_and_restarts() {
    let system = Fake::default();
    install(&linux(), &system, &linux_spec()).expect("install");
    assert_eq!(
        system.file(UNIT),
        Some(linux().render(&linux_spec()).expect("unit"))
    );
    assert!(
        system
            .exists(Path::new("/home/sam/.local/share/kashkeshet"))
            .expect("check")
    );
    assert_eq!(
        system.ran(),
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable kashkeshet.service",
            "systemctl --user restart kashkeshet.service",
        ]
    );
}

#[test]
fn systemd_uninstall_disables_removes_and_reloads() {
    let system = Fake::default().with_file(UNIT);
    linux().uninstall(&system).expect("uninstall");
    assert_eq!(system.file(UNIT), None);
    assert_eq!(
        system.ran(),
        [
            "systemctl --user disable --now kashkeshet.service",
            "systemctl --user daemon-reload",
        ]
    );
    let said = linux().uninstall(&Fake::default()).expect("uninstall");
    assert_eq!(said, ["The service was not installed."]);
}

#[test]
fn systemd_start_stop_status_and_logs() {
    let system = Fake::default().with_file(UNIT).answer(
        "systemctl --user status kashkeshet.service --no-pager",
        3,
        "",
        "",
    );
    linux().start(&system).expect("start");
    linux().stop(&system).expect("stop");
    // A unit that isn't running makes systemctl exit 3; that is a status,
    // not an error.
    linux().status(&system).expect("status");
    linux().logs(&system, 20, false).expect("logs");
    linux().logs(&system, 20, true).expect("logs");
    assert_eq!(
        system.ran(),
        [
            "systemctl --user start kashkeshet.service",
            "systemctl --user stop kashkeshet.service",
            "systemctl --user status kashkeshet.service --no-pager",
            "journalctl --user -u kashkeshet.service -n 20 --no-pager",
            "journalctl --user -u kashkeshet.service -n 20 --no-pager -f",
        ]
    );
}

const ADMIN_KEY: &str = "/home/sam/.config/kkt/admin-key";
const STOP: &str = "systemctl --user stop kashkeshet.service";
const START: &str = "systemctl --user start kashkeshet.service";

/// Writes a new key into `system`'s files on each call, counting them in
/// `made`, as `kkt`'s own key maker does on disk.
fn key_maker<'a>(
    system: &'a Fake,
    made: &'a RefCell<u32>,
) -> impl Fn(&Path) -> Result<(), String> + 'a {
    move |path| {
        *made.borrow_mut() += 1;
        system
            .write_file(path, &format!("key-{}", made.borrow()))
            .map_err(|e| e.to_string())
    }
}

#[test]
fn a_new_key_is_made_and_the_service_restarted_to_read_it() {
    let system = Fake::default().with_file(UNIT);
    let made = RefCell::new(0);
    let added = linux().add_key(&system, Path::new(ADMIN_KEY), &key_maker(&system, &made));
    assert!(added.expect("added"));
    assert_eq!(system.file(ADMIN_KEY).as_deref(), Some("key-1"));
    assert_eq!(system.ran(), [STOP, START]);
}

#[test]
fn adding_a_key_again_after_a_failed_restart_restarts_without_a_new_key() {
    // The restart fails at the stop or at the start; either way the key
    // was made, and the service is not yet using it.
    for broken in [STOP, START] {
        let system = Fake::default().with_file(UNIT).failing(broken);
        let made = RefCell::new(0);
        let make = key_maker(&system, &made);
        let first = linux().add_key(&system, Path::new(ADMIN_KEY), &make);
        assert!(
            matches!(&first, Err(ServiceError::Failed { command, .. }) if command == broken),
            "{first:?}"
        );
        assert_eq!(system.file(ADMIN_KEY).as_deref(), Some("key-1"));

        // Tried again while the service manager still fails: the restart is
        // attempted again, and the key stays.
        let again = linux().add_key(&system, Path::new(ADMIN_KEY), &make);
        assert!(again.is_err(), "{again:?}");

        // Tried again once it works: restarted, with the same key.
        system.recover();
        let last = linux().add_key(&system, Path::new(ADMIN_KEY), &make);
        assert!(!last.expect("added"), "the key was already there");
        assert_eq!(*made.borrow(), 1, "made once, for {broken}");
        assert_eq!(system.file(ADMIN_KEY).as_deref(), Some("key-1"));
        let ran = system.ran();
        let tries = match broken {
            STOP => vec![STOP, STOP, STOP, START],
            _ => vec![STOP, START, STOP, START, STOP, START],
        };
        assert_eq!(ran, tries, "for {broken}");
    }
}

#[test]
fn a_key_that_cannot_be_made_leaves_the_service_alone() {
    let system = Fake::default().with_file(UNIT);
    let added = linux().add_key(&system, Path::new(ADMIN_KEY), &|_| {
        Err("could not write the key".to_string())
    });
    assert!(
        matches!(&added, Err(ServiceError::Key(e)) if e == "could not write the key"),
        "{added:?}"
    );
    assert!(system.ran().is_empty());
}

#[test]
fn a_dry_run_changes_nothing_but_still_asks() {
    let real = Fake::default().not_loaded();
    let dry = DryRun::new(&real);
    install(&mac(), &dry, &mac_spec()).expect("install");
    let actions = dry.actions();
    assert!(real.files.borrow().is_empty());
    assert!(real.dirs.borrow().is_empty());
    // The query ran for real; the change did not.
    assert_eq!(real.ran(), [PRINT]);
    assert!(
        actions
            .iter()
            .any(|a| a.starts_with(&format!("Would write {PLIST}:")))
    );
    assert!(actions.contains(&format!("Would run: launchctl bootstrap gui/501 {PLIST}")));
}

#[test]
fn commands_show_as_typed() {
    let command = Command::new("tail", &["-n", "5", "/a b/log"]);
    assert_eq!(command.to_string(), r#"tail -n 5 "/a b/log""#);
}

#[test]
fn launchctl_failures_other_than_not_loaded_are_errors() {
    let system = Fake::default()
        .with_file(PLIST)
        .answer(PRINT, 5, "", "Input/output error");
    for result in [
        mac().status(&system),
        mac().stop(&system),
        mac().start(&system),
        install(&mac(), &system, &mac_spec()),
    ] {
        let err = result.expect_err("a failed print is not \"not loaded\"");
        assert_eq!(
            err.to_string(),
            format!("`{PRINT}` failed: Input/output error")
        );
    }
}

#[test]
fn launchd_stop_tells_not_running_from_failure() {
    let kill = "launchctl kill SIGTERM gui/501/com.sam-mfb.kashkeshet";
    let idle = Fake::default()
        .with_file(PLIST)
        .answer(kill, 3, "", "No such process\n");
    assert_eq!(
        mac().stop(&idle).expect("stop"),
        ["kashkeshet was not running."]
    );
    let broken = Fake::default()
        .with_file(PLIST)
        .answer(kill, 1, "", "Operation not permitted\n");
    let err = mac().stop(&broken).expect_err("a failed kill is an error");
    assert_eq!(
        err.to_string(),
        format!("`{kill}` failed: Operation not permitted")
    );
}

#[test]
fn failed_log_and_status_commands_are_errors() {
    let tail = format!("tail -n 10 {MAC_LOG}");
    let mac_system = Fake::default().with_file(MAC_LOG).answer(&tail, 1, "", "");
    let err = mac().logs(&mac_system, 10, false).expect_err("tail failed");
    assert_eq!(err.to_string(), format!("`{tail}` failed: exit status 1"));

    let journal = "journalctl --user -u kashkeshet.service -n 10 --no-pager";
    let status = "systemctl --user status kashkeshet.service --no-pager";
    let linux_system = Fake::default()
        .with_file(UNIT)
        .answer(journal, 5, "", "")
        .answer(status, 1, "", "Failed to connect to bus\n");
    let err = linux()
        .logs(&linux_system, 10, false)
        .expect_err("journalctl failed");
    assert_eq!(
        err.to_string(),
        format!("`{journal}` failed: exit status 5")
    );
    let err = linux().status(&linux_system).expect_err("systemctl failed");
    assert_eq!(err.to_string(), format!("`{status}` failed: exit status 1"));
}

#[test]
fn launchd_state_reads_installed_and_running() {
    let none = mac().state(&Fake::default()).expect("state");
    assert_eq!(
        none,
        ServiceState {
            installed: false,
            running: false
        }
    );

    let running = Fake::default().with_file(PLIST).answer(
        PRINT,
        0,
        "gui/501/com.sam-mfb.kashkeshet = {\n\tstate = running\n\tpid = 7\n}\n",
        "",
    );
    assert_eq!(
        mac().state(&running).expect("state"),
        ServiceState {
            installed: true,
            running: true
        }
    );

    let waiting = Fake::default().with_file(PLIST).answer(
        PRINT,
        0,
        "gui/501/com.sam-mfb.kashkeshet = {\n\tstate = not running\n}\n",
        "",
    );
    assert!(!mac().state(&waiting).expect("state").running);

    let unloaded = Fake::default().with_file(PLIST).not_loaded();
    assert_eq!(
        mac().state(&unloaded).expect("state"),
        ServiceState {
            installed: true,
            running: false
        }
    );

    let broken = Fake::default()
        .with_file(PLIST)
        .answer(PRINT, 5, "", "Input/output error");
    assert!(matches!(
        mac().state(&broken),
        Err(ServiceError::Failed { .. })
    ));
}

#[test]
fn systemd_state_reads_is_active() {
    let is_active = "systemctl --user is-active kashkeshet.service";
    let missing = Fake::default();
    assert!(!linux().state(&missing).expect("state").installed);
    assert!(missing.ran().is_empty(), "no unit, so nothing to ask");

    let active = Fake::default()
        .with_file(UNIT)
        .answer(is_active, 0, "active\n", "");
    assert_eq!(
        linux().state(&active).expect("state"),
        ServiceState {
            installed: true,
            running: true
        }
    );
    let inactive = Fake::default()
        .with_file(UNIT)
        .answer(is_active, 3, "inactive\n", "");
    assert!(!linux().state(&inactive).expect("state").running);

    let broken =
        Fake::default()
            .with_file(UNIT)
            .answer(is_active, 1, "", "Failed to connect to bus\n");
    let err = linux()
        .state(&broken)
        .expect_err("a failed query is an error");
    assert_eq!(
        err.to_string(),
        format!("`{is_active}` failed: Failed to connect to bus")
    );
}

/// The error for a service file that can't be inspected: it names the path
/// and the cause.
fn assert_denied(result: Result<impl fmt::Debug, ServiceError>, path: &str) {
    match result {
        Err(ServiceError::File {
            action: "check",
            path: p,
            source,
        }) if p == Path::new(path) && source.kind() == io::ErrorKind::PermissionDenied => {}
        other => panic!("expected a denied check of {path}, got {other:?}"),
    }
}

#[test]
fn a_service_file_that_cannot_be_inspected_is_an_error_not_a_missing_one() {
    for (manager, path) in [(mac(), PLIST), (linux(), UNIT)] {
        let system = Fake::default().with_file(path).denying(path);
        assert_denied(manager.installed_args(&system), path);
        assert_denied(manager.state(&system), path);
        assert_denied(manager.status(&system), path);
        assert_denied(manager.start(&system), path);
        assert_denied(manager.stop(&system), path);
        assert!(system.ran().is_empty(), "{:?}", system.ran());
    }
}

#[test]
fn a_service_file_that_cannot_be_inspected_is_not_taken_as_nothing_to_remove() {
    let mac_system = Fake::default().with_file(PLIST).denying(PLIST);
    assert_denied(mac().uninstall(&mac_system), PLIST);
    assert!(mac_system.file(PLIST).is_some());

    let linux_system = Fake::default().with_file(UNIT).denying(UNIT);
    assert_denied(linux().uninstall(&linux_system), UNIT);
    assert!(linux_system.file(UNIT).is_some());
    assert!(linux_system.ran().is_empty(), "{:?}", linux_system.ran());
}

#[test]
fn a_key_that_cannot_be_inspected_is_not_made() {
    let key = "/home/sam/.config/kashkeshet/admin.key";
    let system = Fake::default().with_file(UNIT).denying(key);
    let made = std::cell::Cell::new(false);
    let make = |_: &Path| {
        made.set(true);
        Ok(())
    };
    assert_denied(linux().add_key(&system, Path::new(key), &make), key);
    assert!(!made.get());
    assert!(system.ran().is_empty(), "{:?}", system.ran());
}

/// On the real system: a service file in a folder that can't be searched is
/// an error naming it, for reading and for removal, while a missing one
/// still reads as not installed. Skipped where the folder can still be
/// searched, as when running as root.
#[cfg(unix)]
#[test]
fn the_real_system_tells_a_denied_service_file_from_a_missing_one() {
    use std::os::unix::fs::PermissionsExt;
    let dir = kkt_scratch::Dir::new("kashkeshet-service-denied");
    let manager = Manager::systemd(dir.path());
    let unit_dir = dir.path().join(kkt_config::SYSTEMD_USER_DIR);
    let unit = unit_dir.join(kkt_config::SYSTEMD_UNIT);

    assert_eq!(manager.installed_args(&OsSystem).expect("read"), None);
    assert!(!remove_file(&OsSystem, &unit).expect("nothing to remove"));

    std::fs::create_dir_all(&unit_dir).expect("unit dir");
    std::fs::write(&unit, manager.render(&linux_spec()).expect("unit")).expect("write unit");
    assert_eq!(
        manager.installed_args(&OsSystem).expect("read"),
        Some(linux_spec().args)
    );
    std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o000))
        .expect("deny the unit folder");
    let searchable = std::fs::metadata(&unit).is_ok();
    let read = manager.installed_args(&OsSystem);
    let removed = remove_file(&OsSystem, &unit);
    std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o700))
        .expect("allow the unit folder");
    if searchable {
        eprintln!("skipped: the folder can still be searched, as when running as root");
        return;
    }
    let unit = unit.to_str().expect("utf-8");
    assert_denied(read, unit);
    assert_denied(removed, unit);
    assert!(Path::new(unit).exists(), "the unit is still there");
}
