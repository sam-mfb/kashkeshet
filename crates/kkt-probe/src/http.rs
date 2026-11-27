//! [`HttpProber`]: the requests, with reqwest. reqwest's types stay in this
//! file.

use crate::{Answered, Initialized, Reply, ReplyError, Settings, Status, Tls, Unprobed};
use reqwest::{Client, Response, StatusCode, Url, redirect::Policy};
use serde_json::{Value, json};

/// Probes servers over HTTP, as its settings say.
pub struct HttpProber {
    settings: Settings,
}

impl HttpProber {
    pub fn new(settings: Settings) -> Self {
        Self { settings }
    }

    /// A plain GET of `url`: the status it answered with, or why it did not
    /// answer.
    pub async fn answer(&self, url: &str) -> Result<Answered, Unprobed> {
        let url = parse(url)?;
        let client = client(self.settings)?;
        Ok(match client.get(url).send().await {
            Ok(response) => Ok(status(response.status())),
            Err(e) => Err(e.to_string()),
        })
    }

    /// A plain GET of `url`, an https address: whether anything there
    /// speaks TLS ([`Tls`]). No key is sent.
    pub async fn tls(&self, url: &str) -> Result<Tls, Unprobed> {
        let url = parse(url)?;
        let client = client(self.settings)?;
        Ok(match client.get(url).send().await {
            Ok(response) => Tls::Answered(status(response.status())),
            Err(e) => tls_failure(&e).unwrap_or_else(|| Tls::Failed(e.to_string())),
        })
    }

    /// An MCP `initialize` request to `url`, with `key` as a bearer token
    /// when given: what the server answered, and, for a session it opened,
    /// what closing that session gave.
    pub async fn initialize(&self, url: &str, key: Option<&str>) -> Result<Initialized, Unprobed> {
        let settings = self.settings;
        let url = parse(url)?;
        let client = client(settings)?;
        let request = json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
            "protocolVersion": settings.protocol_version,
            "capabilities": {},
            "clientInfo": {"name": settings.client_name, "version": settings.client_version}
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
            Err(e) => return Ok(Initialized::Failed(e.to_string())),
        };
        if response.status() != StatusCode::OK {
            return Ok(Initialized::Refused(status(response.status())));
        }
        let session = response.headers().get("mcp-session-id").cloned();
        let reply = read_reply(response, settings).await.map(|value| {
            match value.get("result").and_then(|r| r.get("serverInfo")) {
                Some(info) => Reply::ServerInfo {
                    name: text(info, "name"),
                    version: text(info, "version"),
                },
                None => Reply::NoServerInfo,
            }
        });
        let cleanup = match session {
            Some(session) => {
                let mut delete = client
                    .delete(url)
                    .header("mcp-session-id", session)
                    .header("MCP-Protocol-Version", settings.protocol_version);
                if let Some(key) = key {
                    delete = delete.bearer_auth(key);
                }
                Some(match delete.send().await {
                    Ok(response) => Ok(status(response.status())),
                    Err(e) => Err(e.to_string()),
                })
            }
            None => None,
        };
        Ok(Initialized::Replied { reply, cleanup })
    }
}

fn parse(url: &str) -> Result<Url, Unprobed> {
    Url::parse(url).map_err(|_| Unprobed::BadUrl)
}

fn client(settings: Settings) -> Result<Client, Unprobed> {
    Client::builder()
        .timeout(settings.timeout)
        .connect_timeout(settings.timeout)
        .redirect(Policy::none())
        .no_proxy()
        .https_only(false)
        .user_agent(format!(
            "{}/{}",
            settings.client_name, settings.client_version
        ))
        .build()
        .map_err(|e| Unprobed::NoClient(e.to_string()))
}

/// What `error` says of TLS at the address, where it says something: a
/// refused connection, or a reply that is not TLS. Each layer under reqwest
/// (hyper, the TLS stream) hands on the one below as an I/O error, either
/// as its source or wrapped inside it, so both are followed.
fn tls_failure(error: &(dyn std::error::Error + 'static)) -> Option<Tls> {
    if let Some(io) = error.downcast_ref::<std::io::Error>() {
        if io.kind() == std::io::ErrorKind::ConnectionRefused {
            return Some(Tls::NoListener);
        }
        if let Some(inner) = io.get_ref()
            && let Some(found) = tls_failure(inner)
        {
            return Some(found);
        }
    }
    if let Some(rustls::Error::InvalidMessage(rustls::InvalidMessage::InvalidContentType)) =
        error.downcast_ref::<rustls::Error>()
    {
        return Some(Tls::NotTls);
    }
    error.source().and_then(tls_failure)
}

fn status(code: StatusCode) -> Status {
    Status {
        code: code.as_u16(),
        reason: code.canonical_reason().map(str::to_string),
    }
}

fn text(info: &Value, key: &str) -> Option<String> {
    info.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The first JSON value in `text` that is an `initialize` result or error:
/// the whole body, or one of its `data:` lines when it is a stream.
fn initialized(text: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(text).ok()?;
    let candidates =
        std::iter::once(text).chain(text.lines().filter_map(|line| line.strip_prefix("data:")));
    candidates
        .filter_map(|candidate| serde_json::from_str::<Value>(candidate).ok())
        .find(|value| value.get("result").is_some() || value.get("error").is_some())
}

async fn read_reply(mut response: Response, settings: Settings) -> Result<Value, ReplyError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ReplyError::Read(e.to_string()))?
    {
        if body.len() + chunk.len() > settings.max_response_bytes {
            return Err(ReplyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
        if let Some(value) = initialized(&body) {
            return Ok(value);
        }
    }
    Err(ReplyError::NoResult)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_is_its_code_and_standard_reason() {
        assert_eq!(
            status(StatusCode::NOT_FOUND),
            Status {
                code: 404,
                reason: Some("Not Found".into())
            }
        );
        let odd = StatusCode::from_u16(599).expect("a status");
        assert_eq!(
            status(odd),
            Status {
                code: 599,
                reason: None
            }
        );
    }

    #[test]
    fn an_initialize_result_is_found_whole_or_in_a_stream() {
        let whole = br#"{"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"s"}}}"#;
        assert!(initialized(whole).is_some());
        let stream = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{}}\n\n";
        assert!(initialized(stream).is_some());
        assert!(initialized(b"data: {\"other\":1}\n").is_none());
        assert!(initialized(b"not json").is_none());
    }

    #[test]
    fn a_bad_url_is_refused_before_any_request() {
        let prober = HttpProber::new(Settings {
            timeout: std::time::Duration::from_secs(1),
            max_response_bytes: 1024,
            protocol_version: "2025-06-18",
            client_name: "probe-test",
            client_version: "1",
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        assert_eq!(
            runtime.block_on(prober.answer("not a url")).err(),
            Some(Unprobed::BadUrl)
        );
        assert_eq!(
            runtime.block_on(prober.initialize("not a url", None)).err(),
            Some(Unprobed::BadUrl)
        );
    }

    /// A listener on a free local port that answers each connection with
    /// `reply` and closes it: its address.
    fn listener(reply: &'static [u8]) -> std::net::SocketAddr {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = [0u8; 1024];
                // What the client sent, and the reply, matter only to the
                // client, which the test reads.
                let _ = stream.read(&mut request);
                let _ = stream.write_all(reply);
            }
        });
        address
    }

    fn prober() -> HttpProber {
        HttpProber::new(Settings {
            timeout: std::time::Duration::from_secs(5),
            max_response_bytes: 1024,
            protocol_version: "2025-06-18",
            client_name: "probe-test",
            client_version: "0",
        })
    }

    #[tokio::test]
    async fn tls_says_when_a_plain_http_server_answers() {
        let address = listener(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n");
        let found = prober().tls(&format!("https://{address}/mcp")).await;
        assert_eq!(found, Ok(Tls::NotTls));
    }

    #[tokio::test]
    async fn tls_says_when_nothing_listens() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = free.local_addr().expect("address");
        drop(free);
        let found = prober().tls(&format!("https://{address}/mcp")).await;
        assert_eq!(found, Ok(Tls::NoListener));
    }

    /// A connection closed during the handshake could be a TLS server that
    /// refused this client as well as anything else: it is a failure, not
    /// a server without TLS.
    #[tokio::test]
    async fn tls_calls_a_closed_handshake_a_failure() {
        let address = listener(b"");
        let found = prober().tls(&format!("https://{address}/mcp")).await;
        assert!(matches!(found, Ok(Tls::Failed(_))), "{found:?}");
    }

    #[tokio::test]
    async fn tls_refuses_an_address_that_is_not_a_url() {
        assert_eq!(prober().tls("not a url").await, Err(Unprobed::BadUrl));
    }
}
