//! How long a quick MCP call takes on a connection kept open, as agents'
//! clients keep theirs. An MCP reply goes out as two writes, its headers
//! and then its event stream. Unless the server turns off Nagle's
//! algorithm (`TCP_NODELAY`, in kkt-server's `serve_until`), the second
//! write waits for the client to acknowledge the first, and a client
//! delays that acknowledgment: about 40 ms on Linux, on every call after a
//! connection's first. With it off, a call takes well under a millisecond.

mod common;

use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How many calls are timed.
const CALLS: usize = 20;
/// The slowest median that passes: far above a call without the delay
/// (about 0.3 ms), far below one with it (about 40 ms).
const MEDIAN_LIMIT: Duration = Duration::from_millis(10);

/// One `tools/call` to `/mcp`, written in one piece so the client side
/// adds no delay of its own.
fn request(host: &str, tool: &str, arguments: Value) -> Vec<u8> {
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    })
    .to_string();
    format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\ncontent-type: application/json\r\n\
         accept: application/json, text/event-stream\r\nmcp-protocol-version: 2025-06-18\r\n\
         content-length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Sends `request` on `stream` and reads its whole reply: headers, then a
/// chunked body to its last, empty chunk. Returns the reply as text.
async fn exchange(stream: &mut TcpStream, request: &[u8]) -> String {
    stream.write_all(request).await.expect("the request");
    let mut reply = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    while !reply.ends_with(b"\r\n0\r\n\r\n") {
        let n = stream.read(&mut buffer).await.expect("the reply");
        assert!(n > 0, "the server closed the connection");
        reply.extend_from_slice(&buffer[..n]);
    }
    String::from_utf8(reply).expect("text")
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test measures how long calls take on the real network stack"
)]
async fn quick_calls_on_a_kept_alive_connection_take_no_ack_delay() {
    let server = common::start().await;
    let host = server
        .base
        .strip_prefix("http://")
        .expect("an http base")
        .trim_end_matches('/')
        .to_string();
    let mut stream = TcpStream::connect(&host).await.expect("connected");

    // The connection's first call: a new connection acknowledges at once,
    // so it is fast either way.
    let registered = exchange(&mut stream, &request(&host, "register", json!({}))).await;
    let token = registered
        .split("act_")
        .nth(1)
        .map(|rest| {
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(rest.len());
            format!("act_{}", &rest[..end])
        })
        .unwrap_or_else(|| panic!("a token in: {registered}"));

    let whoami = request(&host, "whoami", json!({ "token": token }));
    let mut times = Vec::new();
    for _ in 0..CALLS {
        let started = Instant::now();
        let reply = exchange(&mut stream, &whoami).await;
        times.push(started.elapsed());
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(!reply.contains("\"isError\":true"), "{reply}");
    }
    times.sort();
    let median = times[CALLS / 2];
    assert!(
        median <= MEDIAN_LIMIT,
        "the median of {CALLS} calls on one connection is {median:?}, over {MEDIAN_LIMIT:?}: \
         is TCP_NODELAY set on accepted connections? times: {times:?}"
    );
}
