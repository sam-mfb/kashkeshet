//! A systemd user unit on Linux. The unit starts the server at login and
//! again only if it fails; its output goes to the journal.

use crate::system::{
    Command, DryRun, System, create_dir, exists, failure_detail, read_service_file, remove_file,
    run, run_attached, run_ok, write_file,
};
use kkt_clock::Clock;
use kkt_service::{
    SERVICE_RESTART_DELAY, SERVICE_STOP_TIMEOUT, ServiceError, ServiceState, Settings, Spec,
    path_text, plain_text,
};
use std::path::{Path, PathBuf};

/// The systemd user unit on Linux.
pub(crate) const SYSTEMD_UNIT: &str = "kashkeshet.service";
/// Where systemd user units live, under `$XDG_CONFIG_HOME`.
pub(crate) const SYSTEMD_USER_DIR: &str = "systemd/user";

/// `systemctl status` and `is-active`'s exit status for a unit that is
/// loaded but not running (stopped or failed): an answer, not a failure.
const INACTIVE: i32 = 3;

pub(crate) struct Unit {
    unit_dir: PathBuf,
}

impl Unit {
    pub(crate) fn new(config_home: &Path) -> Unit {
        Unit {
            unit_dir: config_home.join(SYSTEMD_USER_DIR),
        }
    }

    fn unit(&self) -> PathBuf {
        self.unit_dir.join(SYSTEMD_UNIT)
    }

    pub(crate) fn render(&self, spec: &Spec) -> Result<String, ServiceError> {
        let mut exec = quote(path_text(&spec.program)?)?;
        for arg in &spec.args {
            exec.push(' ');
            exec.push_str(&quote(arg)?);
        }
        Ok(format!(
            "# Written by `kkt service install`. Install again to change it.
[Unit]
Description=Kashkeshet (kkt), a chat server for AI agents
Documentation=https://github.com/sam-mfb/kashkeshet

[Service]
Type=exec
ExecStart={exec}
Restart=on-failure
RestartSec={restart}
KillSignal=SIGTERM
TimeoutStopSec={stop}

[Install]
WantedBy=default.target
",
            restart = SERVICE_RESTART_DELAY.as_secs(),
            stop = SERVICE_STOP_TIMEOUT.as_secs(),
        ))
    }

    /// The arguments in the installed unit's `ExecStart`, after the program.
    pub(crate) fn installed_args(
        &self,
        system: &dyn System,
    ) -> Result<Option<Vec<String>>, ServiceError> {
        let unit = self.unit();
        let Some(text) = read_service_file(system, &unit)? else {
            return Ok(None);
        };
        let unreadable = |reason| ServiceError::Unreadable {
            path: unit.clone(),
            reason,
        };
        let exec = text
            .lines()
            .find_map(|line| line.strip_prefix("ExecStart="))
            .ok_or_else(|| unreadable("it has no ExecStart line"))?;
        let mut words = unquote(exec)
            .ok_or_else(|| unreadable("its ExecStart line is not quoted the way kkt quotes it"))?;
        if words.is_empty() {
            return Err(unreadable("its ExecStart line is empty"));
        }
        words.remove(0);
        Ok(Some(words))
    }

    fn systemctl(args: &[&str]) -> Command {
        let mut all = vec!["--user"];
        all.extend_from_slice(args);
        Command::new("systemctl", &all)
    }

    /// Writes the unit, enables it and starts it, after making the folders
    /// the server needs. Installing again replaces the unit and restarts it.
    pub(crate) fn install(
        &self,
        system: &dyn System,
        spec: &Spec,
    ) -> Result<Vec<String>, ServiceError> {
        for dir in &spec.dirs {
            create_dir(system, dir)?;
        }
        self.write_and_start(system, spec)
    }

    /// What [`Unit::install`] would change on `system`, changing nothing.
    pub(crate) fn dry_run(
        &self,
        system: &dyn System,
        spec: &Spec,
    ) -> Result<Vec<String>, ServiceError> {
        let dry = DryRun::new(system);
        self.install(&dry, spec)?;
        Ok(dry.actions())
    }

    /// Checks, after waiting `settings.service_start_check` on `clock`, that
    /// the server the service just started is still running. If not, the
    /// service is stopped, so it does not start again every few seconds.
    pub(crate) async fn check_started(
        &self,
        system: &dyn System,
        clock: &dyn Clock,
        settings: Settings,
    ) -> Result<(), ServiceError> {
        clock
            .sleep_until(clock.now() + settings.service_start_check)
            .await;
        if self.state(system)?.running {
            return Ok(());
        }
        let stopped = self.stop(system).map(drop);
        Err(ServiceError::DidNotStayUp {
            checked_after: settings.service_start_check,
            stop_failed: stopped.err().map(|e| e.to_string()),
        })
    }

    /// Gives the server the key file `key`: `make` writes it unless it is
    /// already there, then the service is stopped and started.
    pub(crate) fn add_key(
        &self,
        system: &dyn System,
        key: &Path,
        make: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<bool, ServiceError> {
        let made = !exists(system, key)?;
        if made {
            make(key).map_err(ServiceError::Key)?;
        }
        self.stop(system)?;
        self.start(system)?;
        Ok(made)
    }

    fn write_and_start(
        &self,
        system: &dyn System,
        spec: &Spec,
    ) -> Result<Vec<String>, ServiceError> {
        let unit = self.render(spec)?;
        create_dir(system, &self.unit_dir)?;
        write_file(system, &self.unit(), &unit)?;
        run_ok(system, &Self::systemctl(&["daemon-reload"]))?;
        run_ok(system, &Self::systemctl(&["enable", SYSTEMD_UNIT]))?;
        // Restart, not start, so a reinstall runs the new unit.
        run_ok(system, &Self::systemctl(&["restart", SYSTEMD_UNIT]))?;
        Ok(vec![
            format!("Installed {}", self.unit().display()),
            "Started kashkeshet; it starts again at each login.".to_string(),
            "Log: `kkt service logs` (the systemd journal).".to_string(),
            "To keep it running while you are logged out: loginctl enable-linger".to_string(),
        ])
    }

    pub(crate) fn uninstall(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        if !exists(system, &self.unit())? {
            return Ok(vec!["The service was not installed.".to_string()]);
        }
        run_ok(
            system,
            &Self::systemctl(&["disable", "--now", SYSTEMD_UNIT]),
        )?;
        remove_file(system, &self.unit())?;
        run_ok(system, &Self::systemctl(&["daemon-reload"]))?;
        Ok(vec![
            "Stopped kashkeshet.".to_string(),
            format!("Removed {}", self.unit().display()),
        ])
    }

    pub(crate) fn start(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        if !exists(system, &self.unit())? {
            return Err(ServiceError::NotInstalled);
        }
        run_ok(system, &Self::systemctl(&["start", SYSTEMD_UNIT]))?;
        Ok(vec!["Started kashkeshet.".to_string()])
    }

    pub(crate) fn stop(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        if !exists(system, &self.unit())? {
            return Err(ServiceError::NotInstalled);
        }
        run_ok(system, &Self::systemctl(&["stop", SYSTEMD_UNIT]))?;
        Ok(vec![
            "Stopped kashkeshet. It starts again at the next login, or with `kkt service start`."
                .to_string(),
        ])
    }

    pub(crate) fn state(&self, system: &dyn System) -> Result<ServiceState, ServiceError> {
        if !exists(system, &self.unit())? {
            return Ok(ServiceState {
                installed: false,
                running: false,
            });
        }
        let is_active = Command::query("systemctl", &["--user", "is-active", SYSTEMD_UNIT]);
        let finished = run(system, &is_active)?;
        let running = match finished.code {
            Some(0) => true,
            Some(INACTIVE) => false,
            _ => {
                return Err(ServiceError::Failed {
                    command: is_active.to_string(),
                    detail: failure_detail(&finished),
                });
            }
        };
        Ok(ServiceState {
            installed: true,
            running,
        })
    }

    pub(crate) fn status(&self, system: &dyn System) -> Result<Vec<String>, ServiceError> {
        let unit = self.unit();
        if !exists(system, &unit)? {
            return Ok(vec![format!(
                "Not installed (no {}). Run `kkt service install`.",
                unit.display()
            )]);
        }
        // systemctl prints the state itself.
        run_attached(
            system,
            &Command::query(
                "systemctl",
                &["--user", "status", SYSTEMD_UNIT, "--no-pager"],
            ),
            &[INACTIVE],
        )?;
        Ok(vec![format!("Installed: {}", unit.display())])
    }

    pub(crate) fn logs(
        &self,
        system: &dyn System,
        lines: usize,
        follow: bool,
    ) -> Result<Vec<String>, ServiceError> {
        let lines = lines.to_string();
        let mut args = vec!["--user", "-u", SYSTEMD_UNIT, "-n", &lines, "--no-pager"];
        if follow {
            args.push("-f");
        }
        run_attached(system, &Command::query("journalctl", &args), &[])?;
        Ok(Vec::new())
    }
}

/// One `ExecStart=` argument: double-quoted, with backslashes and quotes
/// escaped, and `%` and `$` doubled so systemd takes them literally rather
/// than as specifiers or variables.
fn quote(arg: &str) -> Result<String, ServiceError> {
    let mut quoted = String::from("\"");
    for c in plain_text(arg)?.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '%' => quoted.push_str("%%"),
            '$' => quoted.push_str("$$"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

/// The words of `line`, each written as [`quote`] writes it and separated
/// by spaces; `None` for anything else.
fn unquote(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut chars = line.trim_end().chars().peekable();
    loop {
        while chars.next_if_eq(&' ').is_some() {}
        match chars.next() {
            None => return Some(words),
            Some('"') => {}
            Some(_) => return None,
        }
        let mut word = String::new();
        loop {
            match chars.next()? {
                '"' => break,
                '\\' => word.push(chars.next().filter(|c| matches!(c, '\\' | '"'))?),
                c @ ('%' | '$') => word.push(chars.next().filter(|next| *next == c)?),
                c => word.push(c),
            }
        }
        if chars.peek().is_some_and(|c| *c != ' ') {
            return None;
        }
        words.push(word);
    }
}
