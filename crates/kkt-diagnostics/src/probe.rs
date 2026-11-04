//! [`OsProber`]: what touches the machine, reading key files and probing a
//! server over HTTP. reqwest's types stay in this file.

use super::{Check, KEY_CHECK, KeyFile, Prober, Probing, SERVER_CHECK};
use kkt_config::{DOCTOR_MAX_RESPONSE_BYTES, DOCTOR_PROTOCOL_VERSION, DOCTOR_TIMEOUT};
use reqwest::{Client, Response, Url, redirect::Policy};
use serde_json::{Value, json};
use std::path::Path;

/// The real [`Prober`]: reads files with `std::fs` and probes over HTTP.
pub struct OsProber;

impl Prober for OsProber {
    fn key_file(&self, path: &Path) -> Result<KeyFile, String> {
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        Ok(KeyFile {
            mode: mode(&metadata),
            text: std::fs::read_to_string(path).map_err(|e| e.to_string()),
        })
    }

    fn probe<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a> {
        Box::pin(probe(url, key))
    }
}

/// A file's permission bits.
#[cfg(unix)]
fn mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

/// No permission bits: this system does not have them.
#[cfg(not(unix))]
fn mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

fn initialized(text: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(text).ok()?;
    let candidates =
        std::iter::once(text).chain(text.lines().filter_map(|line| line.strip_prefix("data:")));
    candidates
        .filter_map(|candidate| serde_json::from_str::<Value>(candidate).ok())
        .find(|value| value.get("result").is_some() || value.get("error").is_some())
}

async fn initialize_response(mut response: Response) -> Result<Value, String> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("could not read MCP response: {e}"))?
    {
        if body.len() + chunk.len() > DOCTOR_MAX_RESPONSE_BYTES {
            return Err("MCP response exceeded the diagnostic size limit".into());
        }
        body.extend_from_slice(&chunk);
        if let Some(value) = initialized(&body) {
            return Ok(value);
        }
    }
    Err("the endpoint did not return an MCP initialize result".into())
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(DOCTOR_TIMEOUT)
        .connect_timeout(DOCTOR_TIMEOUT)
        .redirect(Policy::none())
        .no_proxy()
        .https_only(false)
        .user_agent(concat!("kkt-doctor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("could not construct the diagnostic HTTP client: {e}"))
}

/// Adds whether the server answers at `url` and accepts its host name to
/// `checks`. Returns whether both did.
async fn reach(client: &Client, url: &Url, checks: &mut Vec<Check>) -> bool {
    let response = match client.get(url.clone()).send().await {
        Ok(response) => response,
        Err(e) => {
            checks.push(Check::problem(
                SERVER_CHECK,
                e.to_string(),
                "run `kkt service start` or `kkt serve`, and check --url and the bind setting",
            ));
            return false;
        }
    };
    checks.push(Check::ok(SERVER_CHECK, format!("reachable at {url}")));
    if response.status().as_u16() == 403 {
        checks.push(Check::problem(
            "Host",
            "the server refused this host name",
            format!(
                "add {:?} to allow_hosts in the serve config, then restart",
                url.host_str().unwrap_or("")
            ),
        ));
        return false;
    }
    checks.push(Check::ok("Host", "accepted"));
    true
}

/// The server checks for `url`, using `key`.
async fn probe(url: &str, key: Option<&str>) -> Result<Vec<Check>, String> {
    let mut checks = Vec::new();
    let client = http_client()?;
    let url = Url::parse(url).map_err(|_| "invalid diagnostic URL")?;
    if !reach(&client, &url, &mut checks).await {
        return Ok(checks);
    }
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
        "protocolVersion": DOCTOR_PROTOCOL_VERSION,
        "capabilities": {}, "clientInfo":{"name":"kkt-doctor", "version":env!("CARGO_PKG_VERSION")}
    }});
    let mut post = client
        .post(url.clone())
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(request.to_string());
    if let Some(key) = key {
        post = post.bearer_auth(key);
    }
    let response = match post.send().await {
        Ok(response) => response,
        Err(e) => {
            checks.push(Check::problem(
                "MCP",
                e.to_string(),
                "check the MCP URL and server logs",
            ));
            return Ok(checks);
        }
    };
    if response.status().as_u16() == 401 {
        checks.push(Check::problem(KEY_CHECK, "server returned 401", "pass --key-file with the key used by the running server, or restart it after changing keys"));
        return Ok(checks);
    }
    if response.status().as_u16() != 200 {
        checks.push(Check::problem(
            "MCP",
            format!("server returned {}", response.status()),
            "check that --url points at the server's /mcp endpoint",
        ));
        return Ok(checks);
    }
    let session = response.headers().get("mcp-session-id").cloned();
    let initialized = initialize_response(response).await;
    if let Some(session) = session {
        let mut delete = client
            .delete(url)
            .header("mcp-session-id", session)
            .header("MCP-Protocol-Version", DOCTOR_PROTOCOL_VERSION);
        if let Some(key) = key {
            delete = delete.bearer_auth(key);
        }
        match delete.send().await {
            Ok(response) if response.status().is_success() => (),
            Ok(response) => checks.push(Check::problem(
                "probe cleanup",
                format!("server returned {}", response.status()),
                "check the server logs",
            )),
            Err(e) => checks.push(Check::problem(
                "probe cleanup",
                e.to_string(),
                "check the server logs",
            )),
        }
    }
    match initialized {
        Ok(value) => match value.get("result").and_then(|r| r.get("serverInfo")) {
            Some(info) => {
                checks.push(Check::ok(KEY_CHECK, "MCP initialize returned 200"));
                let version = info
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("not reported");
                checks.push(Check::ok("server build", format!("version {version}; commit is not exposed by MCP (see the web page's server runs)")));
            }
            None => checks.push(Check::problem(
                "MCP",
                "initialize did not return serverInfo",
                "check --url and the server's MCP implementation",
            )),
        },
        Err(error) => checks.push(Check::problem(
            "MCP",
            error,
            "check --url and the server logs",
        )),
    }
    Ok(checks)
}
