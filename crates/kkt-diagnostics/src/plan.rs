//! `kkt doctor`'s plan: which checks it makes, in what order, and how it
//! reports them. The program supplies what only it can find out about this
//! machine (kkt in Docker, the service, an unrecorded install) as values.

use super::{Check, Inspector, SERVER_CHECK};
use std::path::Path;

/// The name of the doctor line about how `kkt setup` set this machine up.
pub const SETUP_CHECK: &str = "this machine's setup";

/// Doctor's first lines: the server the clients use, and its key file. Made
/// only by [`ServerLines::inspect`], before anything else is looked at.
#[derive(Debug)]
pub struct ServerLines(Vec<Check>);

impl ServerLines {
    /// Checks the key file at `key_file`, then the server at `url` with it.
    pub async fn inspect(
        inspector: Inspector<'_>,
        url: &str,
        key_file: Option<&Path>,
    ) -> Result<ServerLines, String> {
        inspector.server(url, key_file).await.map(ServerLines)
    }
}

/// An operator key file doctor checks on this machine, and the fix for a
/// problem with it, which depends on how the server is run.
#[derive(Debug)]
pub struct KeyToCheck<'a> {
    pub path: &'a Path,
    pub fix: String,
}

/// The lines about how this machine runs kkt.
#[derive(Debug)]
pub enum ThisMachine {
    /// The server is on another machine; the server lines are about it.
    Elsewhere,
    /// kkt in Docker: kkt itself, the Tailscale login if it is served over
    /// Tailscale, and whether the web key's host copy is the server's, if
    /// kkt runs and there is a copy.
    Docker {
        kkt: Check,
        tailscale_login: Option<Check>,
        web_key_copy: Option<Check>,
    },
    /// The kkt service: set up as one, or not set up by `kkt setup` at all.
    Service(Check),
}

/// Everything `kkt doctor` checks, by name. [`Plan::checks`] puts the lines
/// in their order.
#[derive(Debug)]
pub struct Plan<'a> {
    pub server: ServerLines,
    /// The fix for a server that does not answer, where starting the kkt
    /// service is not how kkt runs; `None` keeps the service fix.
    pub server_fix: Option<String>,
    /// The web key, if this machine has one.
    pub web_key: Option<KeyToCheck<'a>>,
    /// The admin key, if this machine has one.
    pub admin_key: Option<KeyToCheck<'a>>,
    pub machine: ThisMachine,
    /// An install `kkt setup` began and did not finish: none, its summary,
    /// or why its note could not be read.
    pub unfinished: Result<Option<String>, String>,
    /// The line about a server kkt installed here with no record, if one was
    /// looked for and found.
    pub unrecorded: Option<Check>,
}

impl Plan<'_> {
    /// The doctor's lines, in order: the server, the web key, the admin key,
    /// this machine, an unfinished setup, an unrecorded install. Key files
    /// are read through `inspector`.
    pub fn checks(self, inspector: Inspector<'_>) -> Vec<Check> {
        let ServerLines(mut checks) = self.server;
        if let Some(fix) = &self.server_fix {
            point_server_fix_at(&mut checks, fix);
        }
        if let Some(key) = &self.web_key {
            checks.extend(inspector.web_key(key.path, &key.fix));
        }
        if let Some(key) = &self.admin_key {
            checks.extend(inspector.admin_key(key.path, &key.fix));
        }
        match self.machine {
            ThisMachine::Elsewhere => {}
            ThisMachine::Docker {
                kkt,
                tailscale_login,
                web_key_copy,
            } => {
                checks.push(kkt);
                checks.extend(tailscale_login);
                checks.extend(web_key_copy);
            }
            ThisMachine::Service(service) => checks.push(service),
        }
        match self.unfinished {
            Ok(None) => {}
            Ok(Some(summary)) => checks.push(Check::problem(
                SETUP_CHECK,
                summary,
                "run `kkt setup` again to finish it",
            )),
            Err(e) => checks.push(Check::problem(SETUP_CHECK, e, "run `kkt setup` again")),
        }
        checks.extend(self.unrecorded);
        checks
    }
}

/// Replaces the fix on a failed server check, which tells the person to
/// start the kkt service, with `fix`, for a machine where that is not how kkt
/// runs.
pub fn point_server_fix_at(checks: &mut [Check], fix: &str) {
    for check in checks.iter_mut() {
        if check.name == SERVER_CHECK && !check.ok {
            check.fix = Some(fix.to_string());
        }
    }
}

/// The doctor's lines for `checks`, and whether all of them are ok.
pub fn report(checks: Vec<Check>) -> (String, bool) {
    let mut report = String::new();
    let mut all_ok = true;
    for check in checks {
        all_ok &= check.ok;
        report.push_str(&format!(
            "{}: {}: {}\n",
            if check.ok { "ok" } else { "problem" },
            check.name,
            check.detail
        ));
        if let Some(fix) = check.fix {
            report.push_str(&format!("  fix: {fix}\n"));
        }
    }
    (report, all_ok)
}
