//! Sharing a local server over the host's existing Tailscale login:
//! `Tailscale` implements the remote-access contract,
//! [`RemoteAccess`](kkt_remote_access::RemoteAccess). Commands go through a
//! private runner, so what their answers mean is tested against a fake. It
//! leaves the login, DNS preferences and the caller's own settings as they
//! are, and owns only the HTTPS root proxy it installs.

use serde::Deserialize;
use serde_json::Value;
use serde_json::error::Category;
use std::net::SocketAddr;

/// The HTTPS port Tailscale Serve listens on for this node.
const TAILSCALE_HTTPS_PORT: u16 = 443;

mod runner;
use kkt_remote_access::Node;
use runner::{Output, RunError, Runner};

/// Shares this machine's server through its existing Tailscale login.
pub struct Tailscale {
    /// The command a person should rerun once they have fixed what a
    /// refusal says, as the refusal words it, such as "`app setup`".
    rerun: &'static str,
}

impl Tailscale {
    /// `rerun` is the command a person should rerun after fixing what a
    /// refusal names; the refusals tell them to.
    pub const fn new(rerun: &'static str) -> Tailscale {
        Tailscale { rerun }
    }

    /// The command whose answer [`Tailscale::read_status`] reads: run it
    /// wherever Tailscale runs, such as in a container.
    pub const STATUS_COMMAND: [&'static str; 3] = ["tailscale", "status", "--json"];

    fn runner(&self) -> runner::OsRunner {
        runner::OsRunner { rerun: self.rerun }
    }

    /// What an answer of `tailscale status --json` says, read from its
    /// text. It runs nothing: the caller asked Tailscale, wherever it runs.
    /// Only `BackendState` must be there; the other fields are `None` or
    /// empty when they are missing, empty or not text. The error says why
    /// the answer could not be read.
    pub fn read_status(text: &str) -> Result<Status, String> {
        let json: Value = serde_json::from_str(text).map_err(|e| json_cause(&e))?;
        let backend_state = json["BackendState"]
            .as_str()
            .ok_or("no BackendState")?
            .to_string();
        let non_empty = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        Ok(Status {
            backend_state,
            auth_url: non_empty(&json["AuthURL"]),
            dns_name: non_empty(&json["Self"]["DNSName"])
                .map(|name| name.trim_end_matches('.').to_string()),
            cert_domains: json["CertDomains"]
                .as_array()
                .map(|domains| {
                    domains
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// What a Tailscale node's status answer says, in Tailscale's terms
/// ([`Tailscale::read_status`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The node's state: `Running` once logged in, `NeedsLogin` while it
    /// waits for a login, or another of Tailscale's states.
    pub backend_state: String,
    /// The link to log in with, while one is needed.
    pub auth_url: Option<String>,
    /// The node's full name on the tailnet, without the trailing dot.
    pub dns_name: Option<String>,
    /// The names HTTPS certificates can be issued for; empty when the
    /// tailnet has not enabled HTTPS.
    pub cert_domains: Vec<String>,
}

/// The backend state of a node that is logged in and running.
const RUNNING: &str = "Running";
/// The backend state of a node that waits for someone to log in.
const NEEDS_LOGIN: &str = "NeedsLogin";

impl Status {
    /// Whether the node is logged in and running.
    pub fn running(&self) -> bool {
        self.backend_state == RUNNING
    }

    /// Whether the node waits for someone to log in.
    pub fn needs_login(&self) -> bool {
        self.backend_state == NEEDS_LOGIN
    }
}

impl kkt_remote_access::RemoteAccess for Tailscale {
    fn prepare(&self) -> Result<Node, String> {
        prepare(&self.runner(), self.rerun)
    }
    fn serve(&self, bind: SocketAddr) -> Result<(), String> {
        serve(&self.runner(), bind, self.rerun)
    }
    fn served(&self, bind: SocketAddr) -> Result<Option<Node>, String> {
        served(&self.runner(), bind)
    }
    fn stop(&self, bind: SocketAddr) -> Result<(), String> {
        stop(&self.runner(), bind, self.rerun)
    }
}

/// The status answer as sharing this machine's server checks it: every
/// field it needs must be there and well formed.
#[derive(Deserialize)]
struct NodeReport {
    #[serde(rename = "BackendState")]
    backend_state: String,
    #[serde(rename = "Self")]
    own: Option<Own>,
    #[serde(rename = "CertDomains", default)]
    cert_domains: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Own {
    #[serde(rename = "DNSName")]
    dns_name: String,
}

/// Check the existing login and HTTPS setting; never runs `tailscale up`.
fn prepare(host: &dyn Runner, rerun: &str) -> Result<Node, String> {
    node(host, true, rerun)
}

fn node(host: &dyn Runner, https: bool, rerun: &str) -> Result<Node, String> {
    let output = host.run(&["status".into(), "--json".into()])?;
    // Tailscale may exit nonzero while logged out but still return status JSON.
    let status: NodeReport = serde_json::from_str(&output.stdout).map_err(|e| {
        format!(
            "could not read Tailscale status; start Tailscale and log in, then rerun {rerun} \
             (its output could not be read: {})",
            json_cause(&e)
        )
    })?;
    if status.backend_state != RUNNING {
        return Err(format!(
            "Tailscale is {}; open Tailscale and log in (or run `tailscale up`), then rerun {rerun}",
            status.backend_state
        ));
    }
    if !output.success {
        return Err(format!(
            "Tailscale status failed; check `tailscale status` before rerunning {rerun}{}",
            said(&output)
        ));
    }
    let dns_name = status.own.map(|own| own.dns_name).unwrap_or_default();
    let dns_name = dns_name.trim_end_matches('.');
    if !valid_dns(dns_name) {
        return Err(format!(
            "Tailscale did not report a valid node DNS name; enable MagicDNS in https://login.tailscale.com/admin/dns and rerun {rerun}"
        ));
    }
    if https
        && !status
            .cert_domains
            .unwrap_or_default()
            .iter()
            .any(|domain| domain.trim_end_matches('.').eq_ignore_ascii_case(dns_name))
    {
        return Err(format!(
            "Tailscale HTTPS certificates are not enabled for this node; open https://login.tailscale.com/admin/dns, enable MagicDNS, then under HTTPS Certificates select Enable HTTPS and accept the certificate transparency notice. Rerun {rerun} afterwards"
        ));
    }
    Ok(Node {
        dns_name: dns_name.to_string(),
    })
}

fn valid_dns(name: &str) -> bool {
    name.contains('.')
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

fn target(bind: SocketAddr) -> Result<String, String> {
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Err(
            "host Tailscale sharing needs a loopback server address with a fixed port".into(),
        );
    }
    Ok(format!("http://{bind}"))
}

fn config(host: &dyn Runner) -> Result<Value, String> {
    let output = host.run(&["serve".into(), "status".into(), "--json".into()])?;
    if !output.success {
        return Err(format!(
            "could not inspect Tailscale Serve; run `tailscale serve status` and check permissions{}",
            said(&output)
        ));
    }
    serde_json::from_str(&output.stdout).map_err(invalid_config)
}

/// What a failed Tailscale command printed to stderr, to follow its message;
/// nothing when it printed nothing.
fn said(output: &Output) -> String {
    match output.stderr.trim() {
        "" => String::new(),
        stderr => format!(" (Tailscale said: {stderr})"),
    }
}

fn invalid_config(e: serde_json::Error) -> String {
    format!(
        "Tailscale Serve returned invalid configuration JSON: {}",
        json_cause(&e)
    )
}

/// What kind of JSON error this is and where, without serde_json's own text:
/// that text can quote values from the input, which may be private.
fn json_cause(e: &serde_json::Error) -> String {
    let kind = match e.classify() {
        Category::Syntax => "syntax error",
        Category::Eof => "unexpected end",
        Category::Data => "unexpected data",
        Category::Io => "read error",
    };
    format!("{kind} at line {} column {}", e.line(), e.column())
}

fn root<'a>(config: &'a Value, node: &Node) -> Option<&'a Value> {
    config
        .get("Web")?
        .get(format!("{}:{TAILSCALE_HTTPS_PORT}", node.dns_name))?
        .get("Handlers")?
        .get("/")
}

fn changed(host: &dyn Runner, args: &[String]) -> Result<(), String> {
    let output = host.run(args)?;
    if output.success {
        Ok(())
    } else {
        Err(format!(
            "Tailscale Serve failed: {}; check `tailscale serve status` and permissions",
            output.stderr.trim()
        ))
    }
}

/// Install a persistent HTTPS root proxy. Refuse to replace a different root
/// handler, raw TCP listener, or Funnel exposure on this port.
fn serve(host: &dyn Runner, bind: SocketAddr, rerun: &str) -> Result<(), String> {
    let target = target(bind)?;
    let node = prepare(host, rerun)?;
    let config = config(host)?;
    if let Some(handler) = root(&config, &node)
        && handler.get("Proxy").and_then(Value::as_str) != Some(&target)
    {
        return Err("Tailscale HTTPS root already serves something else; choose a different Tailscale node or remove that handler yourself".into());
    }
    let port = TAILSCALE_HTTPS_PORT.to_string();
    if let Some(listener) = config.get("TCP").and_then(|tcp| tcp.get(&port))
        && listener.get("HTTPS").and_then(Value::as_bool) != Some(true)
    {
        return Err("Tailscale already uses the HTTPS port for another TCP listener".into());
    }
    let authority = format!("{}:{TAILSCALE_HTTPS_PORT}", node.dns_name);
    if config
        .get("AllowFunnel")
        .and_then(|funnel| funnel.get(authority))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err("Tailscale Funnel is enabled on this port; disable it before setting up private tailnet access".into());
    }
    changed(
        host,
        &[
            "serve".into(),
            "--bg".into(),
            format!("--https={TAILSCALE_HTTPS_PORT}"),
            target,
        ],
    )
}

/// The node whose HTTPS root Tailscale Serve proxies to the server at
/// `bind`, as [`serve`] sets it up, if there is one. Reads Serve's settings
/// only, so it needs no login. With no `tailscale` program, nothing is
/// served: `None`. Any other failure to run it is an error.
fn served(host: &dyn Runner, bind: SocketAddr) -> Result<Option<Node>, String> {
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Ok(None);
    }
    let target = format!("http://{bind}");
    let args = ["serve".into(), "status".into(), "--json".into()];
    let output = match host.run(&args) {
        Ok(output) => output,
        Err(RunError { missing: true, .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !output.success {
        return Err(format!(
            "could not read Tailscale Serve's settings ({}); start Tailscale and check \
             `tailscale serve status`",
            output.stderr.trim()
        ));
    }
    // With nothing served, Tailscale may print nothing at all.
    if output.stdout.trim().is_empty() {
        return Ok(None);
    }
    let config: Value = serde_json::from_str(&output.stdout).map_err(invalid_config)?;
    let suffix = format!(":{TAILSCALE_HTTPS_PORT}");
    let found = config
        .get("Web")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .find_map(|(authority, web)| {
            let proxy = web.get("Handlers")?.get("/")?.get("Proxy")?.as_str()?;
            let name = authority.strip_suffix(&suffix)?;
            (proxy == target && valid_dns(name)).then(|| Node {
                dns_name: name.to_string(),
            })
        });
    Ok(found)
}

/// Remove the matching root proxy, preserving other paths, ports and login.
/// A changed root handler is left alone and reported to the caller.
fn stop(host: &dyn Runner, bind: SocketAddr, rerun: &str) -> Result<(), String> {
    let target = target(bind)?;
    let node = node(host, false, rerun)?;
    let config = config(host)?;
    let Some(handler) = root(&config, &node) else {
        return Ok(());
    };
    if handler.get("Proxy").and_then(Value::as_str) != Some(&target) {
        return Err("Tailscale HTTPS root now serves something else; left it unchanged".into());
    }
    changed(
        host,
        &[
            "serve".into(),
            "--bg".into(),
            format!("--https={TAILSCALE_HTTPS_PORT}"),
            "--set-path=/".into(),
            "off".into(),
        ],
    )
}

#[cfg(test)]
mod tests;
