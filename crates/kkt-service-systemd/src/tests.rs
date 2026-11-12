use crate::system::{Command, Finished, System};
use crate::unit::Unit;
use kkt_clock::Span;
use kkt_clock::testing::FixedClock;
use kkt_service::{ServiceError, ServiceState, Settings, Spec};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
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

const UNIT: &str = "/home/sam/.config/systemd/user/kashkeshet.service";

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

/// Installs `spec` with `manager`.
fn install(manager: &Unit, system: &dyn System, spec: &Spec) -> Result<Vec<String>, ServiceError> {
    manager.install(system, spec)
}

fn linux() -> Unit {
    Unit::new(Path::new("/home/sam/.config"))
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

const IS_ACTIVE: &str = "systemctl --user is-active kashkeshet.service";

/// Runs `check_started` with `manager` on `system`: it must wait for
/// `Settings::DEFAULT.service_start_check` on the clock, asking nothing before then.
fn check_started(manager: &Unit, system: &Fake) -> Result<(), ServiceError> {
    let clock = clock();
    let mut future = std::pin::pin!(manager.check_started(system, &clock, Settings::DEFAULT));
    assert!(poll(future.as_mut()).is_pending());
    advance(&clock, Settings::DEFAULT.service_start_check / 2);
    assert!(poll(future.as_mut()).is_pending());
    assert!(system.ran().is_empty(), "nothing is asked before the wait");
    advance(&clock, Settings::DEFAULT.service_start_check / 2);
    match poll(future.as_mut()) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("the check is made once the wait is over"),
    }
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
fn the_unit_matches_its_golden_file() {
    golden(
        "kashkeshet.service",
        &linux().render(&linux_spec()).expect("unit"),
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

/// #371: a server that fails as it starts, as when its port is taken, is
/// waiting to be started again when checked. systemd's service is stopped,
/// so it does not start and fail every few seconds, and the install fails.
#[test]
fn systemd_stops_a_server_that_did_not_stay_up() {
    let system = Fake::default()
        .with_file(UNIT)
        .answer(IS_ACTIVE, 3, "activating\n", "");
    let err = check_started(&linux(), &system).expect_err("not running");
    assert!(
        matches!(
            err,
            ServiceError::DidNotStayUp {
                stop_failed: None,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        system.ran(),
        [IS_ACTIVE, "systemctl --user stop kashkeshet.service"]
    );
    assert_eq!(
        err.to_string(),
        "kashkeshet started but was no longer running 3 seconds later. kkt stopped the service \
         so it does not keep starting again every 5 seconds. `kkt service logs` shows why; a \
         common cause is another program using the address in the `bind` setting. The service \
         is still installed and starts again at the next login; once the cause is fixed, run \
         `kkt service start`."
    );
}

/// When stopping the failed server fails too, the error says so, and that
/// it may keep starting again.
#[test]
fn a_failed_stop_after_a_failed_start_is_reported() {
    let stop = "systemctl --user stop kashkeshet.service";
    let system = Fake::default()
        .with_file(UNIT)
        .answer(IS_ACTIVE, 3, "activating\n", "")
        .failing(stop);
    let err = check_started(&linux(), &system).expect_err("not running");
    assert_eq!(
        err.to_string(),
        format!(
            "kashkeshet started but was no longer running 3 seconds later. Stopping the service \
             failed too: `{stop}` failed: Job failed. It may keep starting again every 5 \
             seconds until `kkt service stop` works. `kkt service logs` shows why; a common \
             cause is another program using the address in the `bind` setting. The service is \
             still installed and starts again at the next login; once the cause is fixed, run \
             `kkt service start`."
        )
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
fn commands_show_as_typed() {
    let command = Command::new("tail", &["-n", "5", "/a b/log"]);
    assert_eq!(command.to_string(), r#"tail -n 5 "/a b/log""#);
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
    for spec in [linux_spec(), awkward] {
        let (manager, system) = (linux(), Fake::default());
        assert_eq!(manager.installed_args(&system).expect("read"), None);
        system
            .write_file(Path::new(UNIT), &manager.render(&spec).expect("render"))
            .expect("write");
        assert_eq!(
            manager.installed_args(&system).expect("read"),
            Some(spec.args)
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
    assert!(matches!(
        linux().render(&spec),
        Err(ServiceError::BadValue { .. })
    ));
}

#[test]
fn start_and_stop_need_an_installed_service() {
    let (manager, system) = (linux(), Fake::default());
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

#[test]
fn a_service_file_that_cannot_be_inspected_is_an_error_not_a_missing_one() {
    let (manager, path) = (linux(), UNIT);
    let system = Fake::default().with_file(path).denying(path);
    assert_denied(manager.installed_args(&system), path);
    assert_denied(manager.state(&system), path);
    assert_denied(manager.status(&system), path);
    assert_denied(manager.start(&system), path);
    assert_denied(manager.stop(&system), path);
    assert!(system.ran().is_empty(), "{:?}", system.ran());
}

#[test]
fn a_service_file_that_cannot_be_inspected_is_not_taken_as_nothing_to_remove() {
    let system = Fake::default().with_file(UNIT).denying(UNIT);
    assert_denied(linux().uninstall(&system), UNIT);
    assert!(system.file(UNIT).is_some());
    assert!(system.ran().is_empty(), "{:?}", system.ran());
}

#[test]
fn a_unit_kkt_did_not_write_is_not_read() {
    for text in [
        "[Service]\n",
        "ExecStart=/usr/bin/kkt serve\n",
        "ExecStart=\"kkt\"\"serve\"\n",
        "ExecStart=\"kkt\" \"50%\"\n",
        "ExecStart=\"kkt\n",
        "ExecStart=\n",
    ] {
        let system = Fake::default();
        system.write_file(Path::new(UNIT), text).expect("write");
        assert!(
            matches!(
                linux().installed_args(&system),
                Err(ServiceError::Unreadable { .. })
            ),
            "{text}"
        );
    }
}

#[test]
fn a_server_still_running_after_the_start_passes_the_check() {
    let system = Fake::default()
        .with_file(UNIT)
        .answer(IS_ACTIVE, 0, "active\n", "");
    check_started(&linux(), &system).expect("running");
    assert_eq!(system.ran(), [IS_ACTIVE]);
}

#[test]
fn failed_log_and_status_commands_are_errors() {
    let journal = "journalctl --user -u kashkeshet.service -n 10 --no-pager";
    let status = "systemctl --user status kashkeshet.service --no-pager";
    let system = Fake::default()
        .with_file(UNIT)
        .answer(journal, 5, "", "")
        .answer(status, 1, "", "Failed to connect to bus\n");
    let err = linux()
        .logs(&system, 10, false)
        .expect_err("journalctl failed");
    assert_eq!(
        err.to_string(),
        format!("`{journal}` failed: exit status 5")
    );
    let err = linux().status(&system).expect_err("systemctl failed");
    assert_eq!(err.to_string(), format!("`{status}` failed: exit status 1"));
}

#[test]
fn a_dry_run_changes_nothing() {
    let real = Fake::default();
    let actions = linux().dry_run(&real, &linux_spec()).expect("dry run");
    assert!(real.files.borrow().is_empty());
    assert!(real.dirs.borrow().is_empty());
    assert!(real.ran().is_empty());
    assert!(
        actions
            .iter()
            .any(|a| a.starts_with(&format!("Would write {UNIT}:")))
    );
    assert!(actions.contains(&"Would run: systemctl --user daemon-reload".to_string()));
}

/// Real files, with no path to a service-manager command.
struct NoCommands;

impl System for NoCommands {
    fn exists(&self, path: &Path) -> io::Result<bool> {
        crate::system::OsSystem.exists(path)
    }
    fn read_file(&self, path: &Path) -> io::Result<String> {
        crate::system::OsSystem.read_file(path)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        crate::system::OsSystem.create_dir_all(path)
    }
    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        crate::system::OsSystem.write_file(path, contents)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        crate::system::OsSystem.remove_file(path)
    }
    fn run(&self, _command: &Command) -> io::Result<Finished> {
        panic!("permission test must not run a command")
    }
    fn run_attached(&self, _command: &Command) -> io::Result<Finished> {
        panic!("permission test must not run an attached command")
    }
}

/// On the real system: a unit in a folder that can't be searched is an
/// error naming it, for reading and for removal, while a missing one still
/// reads as not installed. Skipped where the folder can still be searched,
/// as when running as root.
#[cfg(unix)]
#[test]
fn the_real_system_tells_a_denied_service_file_from_a_missing_one() {
    use std::os::unix::fs::PermissionsExt;
    let dir = kkt_scratch::Dir::new("kashkeshet-service-denied");
    let manager = Unit::new(dir.path());
    let unit_dir = dir.path().join("systemd/user");
    let unit = unit_dir.join("kashkeshet.service");

    assert_eq!(manager.installed_args(&NoCommands).expect("read"), None);
    assert_eq!(
        manager.uninstall(&NoCommands).expect("nothing to remove"),
        ["The service was not installed."]
    );

    std::fs::create_dir_all(&unit_dir).expect("unit dir");
    std::fs::write(&unit, manager.render(&linux_spec()).expect("unit")).expect("write unit");
    assert_eq!(
        manager.installed_args(&NoCommands).expect("read"),
        Some(linux_spec().args)
    );
    std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o000))
        .expect("deny the unit folder");
    let searchable = std::fs::metadata(&unit).is_ok();
    let read = manager.installed_args(&NoCommands);
    // Skip first when permissions do not deny access (e.g. root), so this
    // test never invokes a real service manager.
    if searchable {
        std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o700))
            .expect("allow the unit folder");
        eprintln!("skipped: the folder can still be searched, as when running as root");
        return;
    }
    let removed = manager.uninstall(&NoCommands);
    std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o700))
        .expect("allow the unit folder");
    let unit = unit.to_str().expect("utf-8");
    assert_denied(read, unit);
    assert_denied(removed, unit);
    assert!(Path::new(unit).exists(), "the unit is still there");
}
