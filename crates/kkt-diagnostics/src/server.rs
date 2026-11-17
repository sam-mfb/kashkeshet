//! The server checks, made from what a [`Prober`] finds: whether the server
//! answers at its address and accepts its host name, then whether it
//! accepts the key and serves MCP, and its build. Each problem says how to
//! fix it.

use crate::probe::{Initialized, Reply, ReplyError, Status, Unprobed};
use crate::{Check, KEY_CHECK, Prober, SERVER_CHECK};
use url::Url;

/// The checks of the server at `url`, using `key` (`None` for none). `Err`
/// when the probe cannot be made at all.
pub(crate) async fn checks(
    prober: &dyn Prober,
    url: &str,
    key: Option<&str>,
) -> Result<Vec<Check>, String> {
    let mut checks = Vec::new();
    let answered = prober.answer(url).await.map_err(unprobed)?;
    // The address as the prober read it, which is how the check names it.
    let shown = Url::parse(url).map_err(|_| unprobed(Unprobed::BadUrl))?;
    let status = match answered {
        Ok(status) => status,
        Err(e) => {
            checks.push(Check::problem(
                SERVER_CHECK,
                e,
                "run `kkt service start` or `kkt serve`, and check --url and the bind setting",
            ));
            return Ok(checks);
        }
    };
    checks.push(Check::ok(SERVER_CHECK, format!("reachable at {shown}")));
    if status.code == 403 {
        checks.push(Check::problem(
            "Host",
            "the server refused this host name",
            format!(
                "add {:?} to allow_hosts in the serve config, then restart",
                shown.host_str().unwrap_or("")
            ),
        ));
        return Ok(checks);
    }
    checks.push(Check::ok("Host", "accepted"));
    match prober.initialize(url, key).await.map_err(unprobed)? {
        Initialized::Failed(e) => {
            checks.push(Check::problem(
                "MCP",
                e,
                "check the MCP URL and server logs",
            ));
        }
        Initialized::Refused(status) if status.code == 401 => {
            checks.push(Check::problem(
                KEY_CHECK,
                "server returned 401",
                "pass --key-file with the key used by the running server, or restart it after changing keys",
            ));
        }
        Initialized::Refused(status) => {
            checks.push(Check::problem(
                "MCP",
                format!("server returned {}", shown_status(&status)),
                "check that --url points at the server's /mcp endpoint",
            ));
        }
        Initialized::Replied { reply, cleanup } => {
            match cleanup {
                Some(Ok(status)) if (200..300).contains(&status.code) => {}
                Some(Ok(status)) => checks.push(Check::problem(
                    "probe cleanup",
                    format!("server returned {}", shown_status(&status)),
                    "check the server logs",
                )),
                Some(Err(e)) => {
                    checks.push(Check::problem("probe cleanup", e, "check the server logs"))
                }
                None => {}
            }
            match reply {
                Ok(Reply::ServerInfo { version, .. }) => {
                    checks.push(Check::ok(KEY_CHECK, "MCP initialize returned 200"));
                    let version = version.as_deref().unwrap_or("not reported");
                    checks.push(Check::ok(
                        "server build",
                        format!(
                            "version {version}; commit is not exposed by MCP (see the web page's server runs)"
                        ),
                    ));
                }
                Ok(Reply::NoServerInfo) => checks.push(Check::problem(
                    "MCP",
                    "initialize did not return serverInfo",
                    "check --url and the server's MCP implementation",
                )),
                Err(error) => checks.push(Check::problem(
                    "MCP",
                    reply_error(error),
                    "check --url and the server logs",
                )),
            }
        }
    }
    Ok(checks)
}

/// A status as its code and standard reason, "404 Not Found".
fn shown_status(status: &Status) -> String {
    format!(
        "{} {}",
        status.code,
        status.reason.as_deref().unwrap_or("<unknown status code>")
    )
}

fn reply_error(error: ReplyError) -> String {
    match error {
        ReplyError::TooLarge => "MCP response exceeded the diagnostic size limit".into(),
        ReplyError::NoResult => "the endpoint did not return an MCP initialize result".into(),
        ReplyError::Read(e) => format!("could not read MCP response: {e}"),
    }
}

/// Why a probe could not be made, in the words `kkt doctor` uses.
fn unprobed(why: Unprobed) -> String {
    match why {
        Unprobed::BadUrl => "invalid diagnostic URL".into(),
        Unprobed::NoClient(e) => format!("could not construct the diagnostic HTTP client: {e}"),
    }
}
