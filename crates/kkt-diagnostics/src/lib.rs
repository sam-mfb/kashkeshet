//! Read-only connection diagnostics. HTTP and JSON types stay in this part.
//! The checks and their names are here, and doctor's plan of them
//! (`plan.rs`) and setup's wait for a new server (`wait.rs`). What touches
//! the machine, reading key files and probing a server, is behind
//! kkt-probe's `Prober`, which the program injects.

mod plan;
mod wait;

pub(crate) use kkt_probe::{Check, KeyFile, Prober};
pub use plan::{
    KeyToCheck, Plan, SETUP_CHECK, ServerLines, ThisMachine, point_server_fix_at, report,
};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use url::Url;
pub use wait::{SetupServer, Wait, setup_wait, until_reachable};

/// How long the probes and setup's wait for a new server take. The program
/// passes [`Settings::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How long a probe's request may take, connecting included.
    pub doctor_timeout: Duration,
    /// How long `kkt setup` waits for a server it just set up to answer
    /// before its connection check gives up. A new Tailscale node gets its
    /// HTTPS certificate on the first request, which can take a while.
    pub setup_server_wait: Duration,
    /// How often `kkt setup` asks while it waits.
    pub setup_server_poll: Duration,
}

impl Settings {
    /// Probes of five seconds; setup waits two minutes for a new server,
    /// asking every two seconds.
    pub const DEFAULT: Settings = Settings {
        doctor_timeout: Duration::from_secs(5),
        setup_server_wait: Duration::from_secs(2 * 60),
        setup_server_poll: Duration::from_secs(2),
    };
}

/// The checks, made through a [`Prober`].
#[derive(Clone, Copy)]
pub struct Inspector<'a> {
    prober: &'a dyn Prober,
}

impl<'a> Inspector<'a> {
    pub fn new(prober: &'a dyn Prober) -> Inspector<'a> {
        Inspector { prober }
    }

    /// Check the operator key locally, without sending it to any endpoint. A
    /// problem's fix is `fix`, which depends on how the server is run.
    pub fn web_key(self, path: &Path, fix: &str) -> Vec<Check> {
        self.named_key(path, fix, &WEB_KEY_NAMES)
    }

    /// Check the admin key locally, as [`Inspector::web_key`] does the web
    /// key. Only for a server that has one: without it, `/mcp-admin` is off.
    pub fn admin_key(self, path: &Path, fix: &str) -> Vec<Check> {
        self.named_key(path, fix, &ADMIN_KEY_NAMES)
    }

    /// Checks the key file at `path` locally, giving each check its name
    /// from `names` and each problem the fix `fix`.
    fn named_key(self, path: &Path, fix: &str, names: &KeyNames) -> Vec<Check> {
        let mut checks = Vec::new();
        self.key_file(Some(path), &mut checks);
        for check in &mut checks {
            check.name = match check.name {
                "key file" => names.file,
                "key permissions" => names.permissions,
                "key contents" => names.contents,
                _ => names.other,
            };
            if check.fix.is_some() {
                check.fix = Some(fix.to_string());
            }
        }
        checks
    }

    /// Check a server without registering an agent or changing its
    /// database: the key file at `key_path`, then the server with that key.
    pub async fn server(self, url: &str, key_path: Option<&Path>) -> Result<Vec<Check>, String> {
        let mut checks = Vec::new();
        let key = self.key_file(key_path, &mut checks);
        checks.extend(self.prober.probe(url, key.as_deref()).await?);
        Ok(checks)
    }

    /// Checks that a server answers at `url`, accepts its host name, and
    /// serves MCP there, without a key: a server that asks for one passes.
    pub async fn address(self, url: &str) -> Result<Vec<Check>, String> {
        let mut checks = self.prober.probe(url, None).await?;
        checks.retain(|check| check.name != KEY_CHECK);
        Ok(checks)
    }

    /// Checks a server as [`Inspector::server`] does, with the key given as
    /// text (`None` for no key) rather than in a file.
    pub async fn server_with_key(self, url: &str, key: Option<&str>) -> Result<Vec<Check>, String> {
        self.prober.probe(url, key).await
    }

    /// Adds the checks of the key file at `path` to `checks`, and returns
    /// the key if it is valid.
    fn key_file(self, path: Option<&Path>, checks: &mut Vec<Check>) -> Option<String> {
        let Some(path) = path else {
            checks.push(Check::ok("key", "disabled in the serve config"));
            return None;
        };
        key_checks(self.prober.key_file(path), path, checks)
    }
}

/// The check names one key's local checks are given.
struct KeyNames {
    file: &'static str,
    permissions: &'static str,
    contents: &'static str,
    /// Any other check of the key.
    other: &'static str,
}

const WEB_KEY_NAMES: KeyNames = KeyNames {
    file: "web key file",
    permissions: "web key permissions",
    contents: "web key contents",
    other: "web key",
};

const ADMIN_KEY_NAMES: KeyNames = KeyNames {
    file: "admin key file",
    permissions: "admin key permissions",
    contents: "admin key contents",
    other: "admin key",
};

/// Adds the checks of what was found in the key file at `path` to `checks`,
/// and returns the key if it is valid.
fn key_checks(
    found: Result<KeyFile, String>,
    path: &Path,
    checks: &mut Vec<Check>,
) -> Option<String> {
    let file = match found {
        Err(e) => {
            checks.push(Check::problem("key file", format!("{}: {e}", path.display()),
                "run `kkt serve` to create the default key, or pass --key-file with an existing key"));
            return None;
        }
        Ok(file) => file,
    };
    checks.push(Check::ok("key file", path.display().to_string()));
    if let Some(mode) = file.mode {
        if mode == kkt_admission::ACCESS_KEY_FILE_MODE {
            checks.push(Check::ok("key permissions", "0600"));
        } else {
            checks.push(Check::problem(
                "key permissions",
                format!("{mode:04o}, expected 0600"),
                format!("chmod 600 {}", shell_word(&path.to_string_lossy())),
            ));
        }
    }
    match file.text {
        Ok(text) => match kkt_admission::check_access_key(text.trim()) {
            Ok(()) => Some(text.trim().to_string()),
            Err(e) => {
                checks.push(Check::problem(
                    "key contents",
                    e.to_string(),
                    "choose a valid key file with --key-file",
                ));
                None
            }
        },
        Err(e) => {
            checks.push(Check::problem(
                "key contents",
                format!("could not read the key: {e}"),
                "make the key file readable by your user",
            ));
            None
        }
    }
}

/// [`Inspector::admin_key`] on this machine.
pub fn inspect_admin_key(prober: &dyn Prober, path: &Path, fix: &str) -> Vec<Check> {
    Inspector::new(prober).admin_key(path, fix)
}

/// [`Inspector::server`] on this machine. The caller adds the platform's
/// service state using the service part.
pub async fn inspect(
    prober: &dyn Prober,
    url: &str,
    key_path: Option<&Path>,
) -> Result<Vec<Check>, String> {
    Inspector::new(prober).server(url, key_path).await
}

/// [`Inspector::address`] on this machine.
pub async fn inspect_address(prober: &dyn Prober, url: &str) -> Result<Vec<Check>, String> {
    Inspector::new(prober).address(url).await
}

/// [`Inspector::server_with_key`] on this machine.
pub async fn inspect_with_key(
    prober: &dyn Prober,
    url: &str,
    key: Option<&str>,
) -> Result<Vec<Check>, String> {
    Inspector::new(prober).server_with_key(url, key).await
}

/// `text` as one shell word, for fix text the operator copies into a shell
/// (not for running a process here). Single-quoted, with each `'` written as
/// `'\''`.
pub fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Validate an HTTP MCP endpoint without allowing secrets in its URL. A bare
/// origin means /mcp; a supplied path is left intact for forwarding setups.
pub fn endpoint(bind: SocketAddr, given: Option<&str>) -> Result<String, String> {
    let default = format!("http://{bind}/mcp");
    let text = given.unwrap_or(&default);
    if text.chars().any(char::is_control) {
        return Err("URL contains a control character".into());
    }
    let mut url =
        Url::parse(text).map_err(|_| "URL must be an absolute http:// or https:// address")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("URL must be an absolute http:// or https:// address".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("put credentials in the key file, not in the URL; query strings and fragments are not supported".into());
    }
    if url.path() == "/" {
        url.set_path("/mcp");
    }
    Ok(url.to_string())
}

/// The name of the check that the server answered at all.
pub const SERVER_CHECK: &str = "server";

/// Whether `checks` found the server not answering at all, as opposed to
/// answering with a problem.
pub fn unreachable(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.name == SERVER_CHECK && !c.ok)
}

/// The name of the check that the server accepted the key.
pub const KEY_CHECK: &str = "key accepted";

/// The most bytes of an MCP response a probe reads.
pub const DOCTOR_MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// The MCP protocol version a probe asks for.
pub const DOCTOR_PROTOCOL_VERSION: &str = "2026-07-28";

#[cfg(test)]
mod tests;
