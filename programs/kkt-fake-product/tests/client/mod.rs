//! Calling the fake product over HTTP as its callers do: agent tools at
//! `/mcp` and admin tools at `/mcp-admin`, each with a bearer, and
//! the web page with a session from signing in with a bearer.

use futures::StreamExt;
use kkt_agent_client::{Http, HttpReply};
use kkt_fake_product::Product;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
use serde_json::{Value, json};
use std::time::Duration;

/// The longest a test waits for the live feed to bring what it expects.
/// Past it, the test fails instead of hanging.
pub const FEED_WAIT: Duration = Duration::from_secs(60);

/// `work`'s result, or a failure naming `what` if it takes longer than
/// [`FEED_WAIT`] by the system clock.
async fn within<T>(what: impl Fn() -> String, work: impl Future<Output = T>) -> T {
    use kkt_clock::Clock;
    use kkt_clock_system::SystemClock;
    let clock = SystemClock;
    tokio::select! {
        done = work => done,
        () = clock.sleep_until(clock.now() + FEED_WAIT) => panic!("{} in time", what()),
    }
}

/// One tool call's answer: the text the tool returned, and whether it was
/// an error.
#[derive(Clone, Debug)]
pub struct Answer {
    pub text: String,
    pub is_error: bool,
    /// The whole JSON-RPC response, as sent.
    pub raw: String,
}

impl Answer {
    /// The text as JSON, for a call made with `format: "json"`.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text))
    }
}

/// A client for one product.
pub struct Client {
    http: Http,
    base: String,
}

impl Client {
    pub fn new(product: &Product) -> Client {
        Client {
            http: Http::new().expect("an HTTP client"),
            base: product.url(""),
        }
    }

    /// Calls `tool` at `/mcp` with `arguments`, presenting `bearer`.
    pub async fn tool(&self, bearer: &str, tool: &str, arguments: Value) -> Answer {
        self.call("/mcp", bearer, tool, arguments).await
    }

    /// Calls `tool` at `/mcp-admin` with `arguments`, presenting `bearer`.
    pub async fn admin(&self, bearer: &str, tool: &str, arguments: Value) -> Answer {
        self.call("/mcp-admin", bearer, tool, arguments).await
    }

    /// The HTTP status of a `tools/call` to `path` that presents `bearer`,
    /// or no `Authorization` header with `None`.
    pub async fn mcp_status(&self, path: &str, bearer: Option<&str>) -> StatusCode {
        let message = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "register", "arguments": {} }
        });
        let response = self
            .http
            .post(
                &format!("{}{path}", self.base),
                bearer,
                &[
                    (CONTENT_TYPE.as_str(), "application/json"),
                    ("accept", "application/json, text/event-stream"),
                    ("mcp-protocol-version", "2025-06-18"),
                ],
                message.to_string(),
            )
            .await
            .expect("an answer");
        http_status(&response)
    }

    async fn call(&self, path: &str, bearer: &str, tool: &str, arguments: Value) -> Answer {
        let message = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        });
        let response = self
            .http
            .post(
                &format!("{}{path}", self.base),
                Some(bearer),
                &[
                    (CONTENT_TYPE.as_str(), "application/json"),
                    ("accept", "application/json, text/event-stream"),
                    ("mcp-protocol-version", "2025-06-18"),
                ],
                message.to_string(),
            )
            .await
            .expect("an answer");
        let status = http_status(&response);
        let raw = response.text().await.expect("a body");
        assert_eq!(status, StatusCode::OK, "{tool} at {path}: {raw}");
        let envelope = rpc_message(&raw);
        let result = &envelope["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{tool} at {path} gave no text: {raw}"))
            .to_string();
        Answer {
            text,
            is_error: result["isError"].as_bool() == Some(true),
            raw,
        }
    }

    /// Signs in to the web page with `bearer`, and returns the session's
    /// cookie.
    pub async fn sign_in(&self, bearer: &str) -> String {
        let response = self
            .http
            .post(
                &format!("{}/login", self.base),
                None,
                &[(CONTENT_TYPE.as_str(), "application/x-www-form-urlencoded")],
                format!("key={bearer}"),
            )
            .await
            .expect("an answer");
        assert_eq!(http_status(&response), StatusCode::SEE_OTHER, "signed in");
        assert_eq!(response.header(LOCATION.as_str()), Some(&b"/"[..]));
        let set = response
            .header_text(SET_COOKIE.as_str())
            .expect("a session cookie");
        set.split(';').next().expect("name=value").to_string()
    }

    /// The page at `path`, as the viewer holding `cookie` sees it.
    pub async fn page(&self, cookie: &str, path: &str) -> (StatusCode, String) {
        let response = self
            .http
            .get(
                &format!("{}{path}", self.base),
                None,
                &[(COOKIE.as_str(), cookie)],
            )
            .await
            .expect("an answer");
        let status = http_status(&response);
        (status, response.text().await.expect("a page"))
    }

    /// Posts the form at `path` with `fields`, as the viewer holding
    /// `cookie`, with the form token from the page at `from`.
    pub async fn form(
        &self,
        cookie: &str,
        from: &str,
        path: &str,
        fields: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let (status, page) = self.page(cookie, from).await;
        assert_eq!(status, StatusCode::OK, "{from}");
        let token = form_token_in(&page);
        let mut body = vec![("form_token", token.as_str())];
        body.extend_from_slice(fields);
        let response = self
            .http
            .form(
                &format!("{}{path}", self.base),
                None,
                &[(COOKIE.as_str(), cookie)],
                &body,
            )
            .await
            .expect("an answer");
        let status = http_status(&response);
        (status, response.text().await.expect("a page"))
    }

    /// The live feed `/events`, as the viewer holding `cookie` gets it.
    pub async fn events(&self, cookie: &str) -> Events {
        let response = self
            .http
            .get(
                &format!("{}/events", self.base),
                None,
                &[(COOKIE.as_str(), cookie)],
            )
            .await
            .expect("an answer");
        assert_eq!(http_status(&response), StatusCode::OK, "the live feed");
        Events {
            bytes: response.bytes_stream(),
            pending: Vec::new(),
        }
    }
}

/// The original test-facing status type, without putting it in the provider.
fn http_status(response: &HttpReply) -> StatusCode {
    StatusCode::from_u16(response.status()).expect("an HTTP status")
}

/// The token a page's forms carry.
fn form_token_in(page: &str) -> String {
    let (_, rest) = page
        .split_once(r#"name="form_token" value=""#)
        .expect("a form token");
    rest.split_once('"').expect("closing quote").0.to_string()
}

/// The one JSON-RPC message in `raw`: plain JSON, or one server-sent event.
fn rpc_message(raw: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return value;
    }
    raw.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|data| !data.is_empty())
        .find_map(|data| serde_json::from_str::<Value>(data).ok())
        .unwrap_or_else(|| panic!("no JSON-RPC message in: {raw}"))
}

/// One event from the live feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// The event's name, such as `message` or `lagged`.
    pub name: String,
    /// Its data, as sent.
    pub data: String,
}

/// The live feed, read event by event.
pub struct Events {
    bytes: futures::stream::BoxStream<'static, Result<Vec<u8>, String>>,
    pending: Vec<u8>,
}

impl Events {
    /// The next event, skipping keep-alive comments. `None` when the feed
    /// ends.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(end) = self.pending.windows(2).position(|w| w == b"\n\n") {
                let block: Vec<u8> = self.pending.drain(..end + 2).collect();
                let block = String::from_utf8(block).expect("the feed is text");
                if let Some(event) = parse_event(&block) {
                    return Some(event);
                }
                continue;
            }
            let chunk = self.bytes.next().await?.expect("the feed's bytes");
            self.pending.extend_from_slice(&chunk);
        }
    }

    /// Events up to and including the first whose data contains `until`,
    /// each given to `check` as it comes. Fails if that takes longer than
    /// [`FEED_WAIT`].
    pub async fn until(&mut self, until: &str, check: impl Fn(&Event)) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = within(|| format!("no {until} in the feed: {seen:?}"), self.next())
                .await
                .unwrap_or_else(|| panic!("the feed ended before {until}: {seen:?}"));
            check(&event);
            let done = event.data.contains(until);
            seen.push(event);
            if done {
                return seen;
            }
        }
    }
}

/// The event in one block of the feed, or `None` for a comment.
pub fn parse_event(block: &str) -> Option<Event> {
    let mut name = None;
    let mut data = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
        }
    }
    Some(Event {
        name: name.or_else(|| (!data.is_empty()).then(|| "message".to_string()))?,
        data: data.join("\n"),
    })
}

/// The live feed over a socket that takes in only a few kilobytes at a
/// time and is not read until asked: a subscriber too slow to keep up.
/// Once the server has filled the socket, it stops reading the feed's
/// events, which wait for it in the events bus.
pub struct SlowFeed {
    stream: tokio::net::TcpStream,
    /// What has come in after the response's headers, still chunked.
    received: Vec<u8>,
}

impl SlowFeed {
    /// Opens `/events` on `product` as the viewer holding `cookie`, and
    /// waits for the response's headers, so the feed has started.
    pub async fn open(product: &Product, cookie: &str) -> SlowFeed {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
        socket
            .set_recv_buffer_size(4096)
            .expect("a small receive buffer");
        let mut stream = socket.connect(product.address).await.expect("connected");
        let request = format!(
            "GET /events HTTP/1.1\r\nHost: {}\r\nCookie: {cookie}\r\nAccept: text/event-stream\r\n\r\n",
            product.address
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("the request");
        let mut received = Vec::new();
        let mut buffer = [0u8; 1024];
        let end = loop {
            if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                break end;
            }
            let n = stream.read(&mut buffer).await.expect("the response");
            assert!(n > 0, "the feed closed before its headers");
            received.extend_from_slice(&buffer[..n]);
        };
        let headers = String::from_utf8_lossy(&received[..end]).to_string();
        assert!(headers.starts_with("HTTP/1.1 200"), "{headers}");
        assert!(
            headers
                .to_lowercase()
                .contains("transfer-encoding: chunked"),
            "{headers}"
        );
        received.drain(..end + 4);
        SlowFeed { stream, received }
    }

    /// Reads the feed until its events include `until`, and returns them.
    /// Fails if a read waits longer than [`FEED_WAIT`].
    pub async fn read_until(&mut self, until: &str) -> Vec<Event> {
        use tokio::io::AsyncReadExt;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let body = dechunk(&self.received);
            let text = String::from_utf8_lossy(&body);
            if let Some(at) = text.find(until) {
                // Up to the end of the event that holds `until`.
                let end = text[at..].find("\n\n").map_or(text.len(), |e| at + e + 2);
                return text[..end].split("\n\n").filter_map(parse_event).collect();
            }
            let n = within(
                || format!("no {until} in the feed"),
                self.stream.read(&mut buffer),
            )
            .await
            .expect("the feed");
            assert!(n > 0, "the feed ended before {until}");
            self.received.extend_from_slice(&buffer[..n]);
        }
    }
}

/// The data of the whole chunks in `chunked`, an HTTP/1.1 chunked body.
fn dechunk(chunked: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut rest = chunked;
    while let Some(line_end) = rest.windows(2).position(|w| w == b"\r\n") {
        let size_text = String::from_utf8_lossy(&rest[..line_end]).to_string();
        let size_text = size_text.split(';').next().unwrap_or("").trim().to_string();
        let size = usize::from_str_radix(&size_text, 16)
            .unwrap_or_else(|_| panic!("a chunk size, not {size_text:?}"));
        let start = line_end + 2;
        if size == 0 || rest.len() < start + size + 2 {
            break;
        }
        body.extend_from_slice(&rest[start..start + size]);
        rest = &rest[start + size + 2..];
    }
    body
}
