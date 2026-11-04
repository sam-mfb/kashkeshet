//! Read-only connection diagnostics. HTTP and JSON types stay in this part.
//! The checks and their names are here. What touches the machine, reading
//! key files and probing a server, is behind [`Prober`]; its real
//! implementation, [`OsProber`], is in `probe.rs`.

#[cfg(feature = "real")]
mod probe;

use kkt_config as config;
#[cfg(feature = "real")]
pub use probe::OsProber;
use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use url::Url;

/// What a [`Prober`] found in a key file.
#[derive(Debug, Clone)]
pub struct KeyFile {
    /// The file's permission bits, where the system has them.
    pub mode: Option<u32>,
    /// The file's text, or why it could not be read.
    pub text: Result<String, String>,
}

/// The server checks a [`Prober`] makes, once they are done.
pub type Probing<'a> = Pin<Box<dyn Future<Output = Result<Vec<Check>, String>> + Send + 'a>>;

/// What the checks do to the machine. Neither registers an agent nor
/// changes a server's database.
pub trait Prober: Sync {
    /// What the key file at `path` holds. `Err` says why the file could not
    /// be looked at.
    fn key_file(&self, path: &Path) -> Result<KeyFile, String>;

    /// The checks of the server at `url`, using `key` (`None` for none):
    /// whether it answers, accepts its host name, accepts the key and serves
    /// MCP, and its build. `Err` when the probe cannot be made at all.
    fn probe<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a>;
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
        let mut checks = Vec::new();
        self.key_file(Some(path), &mut checks);
        for check in &mut checks {
            check.name = match check.name {
                "key file" => "web key file",
                "key permissions" => "web key permissions",
                "key contents" => "web key contents",
                _ => "web key",
            };
            if check.fix.is_some() {
                check.fix = Some(fix.to_string());
            }
        }
        checks
    }

    /// Check the admin key locally, as [`Inspector::web_key`] does the web
    /// key. Only for a server that has one: without it, `/mcp-admin` is off.
    pub fn admin_key(self, path: &Path, fix: &str) -> Vec<Check> {
        let mut checks = Vec::new();
        self.key_file(Some(path), &mut checks);
        for check in &mut checks {
            check.name = match check.name {
                "key file" => "admin key file",
                "key permissions" => "admin key permissions",
                "key contents" => "admin key contents",
                _ => "admin key",
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
        if mode == config::ACCESS_KEY_FILE_MODE {
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
        Ok(text) => match config::check_access_key(text.trim()) {
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

/// [`Inspector::web_key`] on this machine.
#[cfg(feature = "real")]
pub fn inspect_web_key(path: &Path, fix: &str) -> Vec<Check> {
    Inspector::new(&OsProber).web_key(path, fix)
}

/// [`Inspector::admin_key`] on this machine.
#[cfg(feature = "real")]
pub fn inspect_admin_key(path: &Path, fix: &str) -> Vec<Check> {
    Inspector::new(&OsProber).admin_key(path, fix)
}

/// [`Inspector::server`] on this machine. The caller adds the platform's
/// service state using the service part.
#[cfg(feature = "real")]
pub async fn inspect(url: &str, key_path: Option<&Path>) -> Result<Vec<Check>, String> {
    Inspector::new(&OsProber).server(url, key_path).await
}

/// [`Inspector::address`] on this machine.
#[cfg(feature = "real")]
pub async fn inspect_address(url: &str) -> Result<Vec<Check>, String> {
    Inspector::new(&OsProber).address(url).await
}

/// [`Inspector::server_with_key`] on this machine.
#[cfg(feature = "real")]
pub async fn inspect_with_key(url: &str, key: Option<&str>) -> Result<Vec<Check>, String> {
    Inspector::new(&OsProber).server_with_key(url, key).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
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

#[cfg(test)]
mod tests;
