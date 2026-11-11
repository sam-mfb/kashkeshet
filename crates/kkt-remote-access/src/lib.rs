//! Sharing a service-mode server over the host's existing Tailscale login.
//! Commands go through Host; this part never changes login, DNS preferences,
//! kkt's config, or its service. It owns only the HTTPS root proxy it installs.

use serde::Deserialize;
use serde_json::Value;
use serde_json::error::Category;
use std::net::SocketAddr;

/// The host Tailscale Serve listener used by service-mode setup.
const TAILSCALE_HTTPS_PORT: u16 = 443;

/// The host's Tailscale CLI, injected so tests never change a real tailnet.
pub trait Host {
    fn run(&self, args: &[String]) -> Result<Output, String>;
}

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The exact node name, including any suffix assigned by Tailscale.
#[derive(Debug, PartialEq, Eq)]
pub struct Node {
    pub dns_name: String,
}

#[derive(Deserialize)]
struct Status {
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
pub fn prepare(host: &dyn Host) -> Result<Node, String> {
    node(host, true)
}

fn node(host: &dyn Host, https: bool) -> Result<Node, String> {
    let output = host.run(&["status".into(), "--json".into()])?;
    // Tailscale may exit nonzero while logged out but still return status JSON.
    let status: Status = serde_json::from_str(&output.stdout).map_err(|e| {
        format!(
            "could not read Tailscale status; start Tailscale and log in, then rerun `kkt setup` \
             (its output could not be read: {})",
            json_cause(&e)
        )
    })?;
    if status.backend_state != "Running" {
        return Err(format!(
            "Tailscale is {}; open Tailscale and log in (or run `tailscale up`), then rerun `kkt setup`",
            status.backend_state
        ));
    }
    if !output.success {
        return Err(format!(
            "Tailscale status failed; check `tailscale status` before rerunning `kkt setup`{}",
            said(&output)
        ));
    }
    let dns_name = status.own.map(|own| own.dns_name).unwrap_or_default();
    let dns_name = dns_name.trim_end_matches('.');
    if !valid_dns(dns_name) {
        return Err("Tailscale did not report a valid node DNS name; enable MagicDNS in https://login.tailscale.com/admin/dns and rerun `kkt setup`".into());
    }
    if https
        && !status
            .cert_domains
            .unwrap_or_default()
            .iter()
            .any(|domain| domain.trim_end_matches('.').eq_ignore_ascii_case(dns_name))
    {
        return Err("Tailscale HTTPS certificates are not enabled for this node; open https://login.tailscale.com/admin/dns, enable MagicDNS, then under HTTPS Certificates select Enable HTTPS and accept the certificate transparency notice. Rerun `kkt setup` afterwards".into());
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

fn config(host: &dyn Host) -> Result<Value, String> {
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

fn changed(host: &dyn Host, args: &[String]) -> Result<(), String> {
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
pub fn serve(host: &dyn Host, bind: SocketAddr) -> Result<(), String> {
    let target = target(bind)?;
    let node = prepare(host)?;
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
/// only, so it needs no login. A Tailscale that cannot be run serves
/// nothing: `None`.
pub fn served(host: &dyn Host, bind: SocketAddr) -> Result<Option<Node>, String> {
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Ok(None);
    }
    let target = format!("http://{bind}");
    let args = ["serve".into(), "status".into(), "--json".into()];
    let Ok(output) = host.run(&args) else {
        return Ok(None);
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
pub fn stop(host: &dyn Host, bind: SocketAddr) -> Result<(), String> {
    let target = target(bind)?;
    let node = node(host, false)?;
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
