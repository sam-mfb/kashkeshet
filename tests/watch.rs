//! `kkt watch` against a real server: one line per message, `--once`,
//! `--after-id`, `--channel`, `--state-file`, `--timeout`, `--full`, and the
//! errors that end it. The token never appears in arguments or output.

mod common;
#[path = "common/scratch.rs"]
mod scratch;

use kashkeshet::admission::{Admission, SharedKey};
use kashkeshet::watch;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const KEY: &str = "0123456789abcdef0123456789abcdef";
const TIMEOUT: Duration = Duration::from_secs(20);

struct Agent {
    client: common::Client,
    name: String,
    token: String,
}

impl Agent {
    async fn new(server: &common::Server) -> Agent {
        let client = common::connect_new_with_key(server, KEY).await;
        let text = call(&client, "register", json!({})).await;
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}: ")))
                .unwrap_or_else(|| panic!("no {key} in {text}"))
                .to_string()
        };
        Agent {
            name: field("name"),
            token: field("token"),
            client,
        }
    }

    async fn channel(&self, name: &str, members: &[&Agent]) -> String {
        let members: Vec<&str> = members.iter().map(|a| a.name.as_str()).collect();
        let text = self
            .tool(
                "create_channel",
                json!({"name": name, "open": false, "members": members}),
            )
            .await;
        text["channel_id"]
            .as_str()
            .or_else(|| text["id"].as_str())
            .unwrap_or_else(|| panic!("no channel ID in {text}"))
            .to_string()
    }

    async fn send(&self, channel: &str, body: &str) -> i64 {
        self.tool("send", json!({"channel_id": channel, "body": body}))
            .await["message_id"]
            .as_i64()
            .expect("message_id")
    }

    async fn tool(&self, tool: &str, mut args: Value) -> Value {
        args["token"] = json!(self.token);
        args["format"] = json!("json");
        let text = call(&self.client, tool, args).await;
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
    }
}

async fn call(client: &common::Client, tool: &str, args: Value) -> String {
    let Value::Object(map) = args else {
        panic!("arguments must be an object")
    };
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(map))
        .await
        .expect("call_tool");
    assert_ne!(result.is_error, Some(true), "{tool} failed");
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

async fn start() -> common::Server {
    let mut config = common::test_config();
    config.admission = Admission::SharedKey(SharedKey::new(KEY, "test").expect("key"));
    common::start_with(config).await
}

/// A fresh home directory holding the access key, so the command reads no
/// real config file. It is removed when dropped, so each test declares it
/// before the `kkt watch` it starts.
fn home(name: &str) -> scratch::Dir {
    let dir = scratch::Dir::new(&format!("kashkeshet-watch-{name}"));
    std::fs::write(dir.join("key"), KEY).expect("key file");
    dir
}

/// `kkt watch` against `server`, with `token` in the environment.
fn watch(server: &common::Server, home: &Path, token: &str, args: &[&str]) -> Command {
    watch_at(&format!("{}/mcp", server.base), home, token, args)
}

fn watch_at(url: &str, home: &Path, token: &str, args: &[&str]) -> Command {
    let mut command = Command::new(KKT);
    command
        .arg("watch")
        .args(["--url", url])
        .arg("--key-file")
        .arg(home.join("key"))
        .args(args)
        .env("HOME", home)
        .env("KKT_TOKEN", token)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn finish(mut command: Command) -> Output {
    tokio::time::timeout(TIMEOUT, command.output())
        .await
        .expect("kkt watch finished")
        .expect("run kkt watch")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_no_secrets(output: &Output, token: &str) {
    for stream in [&output.stdout, &output.stderr] {
        let stream = text(stream);
        assert!(!stream.contains(token), "token printed: {stream}");
        assert!(!stream.contains(KEY), "key printed: {stream}");
    }
}

#[tokio::test]
async fn keeps_running_and_prints_one_line_per_message_from_others() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    other.send(&channel, "before watching").await;
    let home = home("stream");
    let mut child = watch(&server, &home, &me.token, &[])
        .spawn()
        .expect("start");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let mut errors = BufReader::new(child.stderr.take().expect("stderr")).lines();
    // Wait until it has found its start, so the messages below are new.
    tokio::time::timeout(TIMEOUT, async {
        while let Some(line) = errors.next_line().await.expect("stderr") {
            if line.contains("watching as") {
                return;
            }
        }
        panic!("kkt watch stopped before it started watching");
    })
    .await
    .expect("watch started");

    let first = other.send(&channel, "hello\nworld").await;
    me.send(&channel, "my own message").await;
    let second = other.send(&channel, "again").await;
    let mut next = async || {
        tokio::time::timeout(TIMEOUT, lines.next_line())
            .await
            .expect("a line")
            .expect("stdout")
            .expect("still running")
    };
    assert_eq!(
        next().await,
        format!("[{first}] #talk {}: hello / world", other.name)
    );
    assert_eq!(
        next().await,
        format!("[{second}] #talk {}: again", other.name)
    );
    assert!(child.try_wait().expect("status").is_none(), "it stopped");

    // The token was never an argument.
    #[cfg(target_os = "linux")]
    {
        let pid = child.id().expect("pid");
        let args = std::fs::read(format!("/proc/{pid}/cmdline")).expect("cmdline");
        assert!(!text(&args).contains(&me.token));
    }
    child.kill().await.expect("stop");
}

#[tokio::test]
async fn once_prints_messages_after_the_given_id_and_exits() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    let old = other.send(&channel, "old").await;
    let first = other.send(&channel, "new one").await;
    let second = other.send(&channel, "new two").await;
    let home = home("once");
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &["--once", "--after-id", &old.to_string()],
    ))
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!(
            "[{first}] #talk {0}: new one\n[{second}] #talk {0}: new two\n",
            other.name
        )
    );
    assert_no_secrets(&output, &me.token);
}

#[tokio::test]
async fn once_waits_for_a_message_in_the_given_channels_only() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let quiet = me.channel("quiet", &[&other]).await;
    let busy = me.channel("busy", &[&other]).await;
    let home = home("channels");
    let child = watch(&server, &home, &me.token, &["--once", "--channel", &busy])
        .spawn()
        .expect("start");
    // Messages keep coming until the watch sees one in `busy`; the ones in
    // `quiet` must not end it.
    let sender = async {
        loop {
            other.send(&quiet, "not watched").await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            other.send(&busy, "watched").await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    };
    let output = tokio::select! {
        output = tokio::time::timeout(TIMEOUT, child.wait_with_output()) => {
            output.expect("finished").expect("output")
        }
        () = sender => unreachable!(),
    };
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains("#busy") && !stdout.contains("#quiet"),
        "{stdout}"
    );
}

#[tokio::test]
async fn a_state_file_neither_repeats_nor_misses_messages() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    let latest = other.send(&channel, "history").await;
    let home = home("state");
    let state = home.join("state");
    let state_arg = state.to_str().expect("path");

    // With no state file it starts after the latest message.
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &["--once", "--timeout", "1", "--state-file", state_arg],
    ))
    .await;
    assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(
        std::fs::read_to_string(&state).expect("state"),
        format!("{latest}\n")
    );

    // A message sent while nothing watches is printed by the next run.
    let missed = other.send(&channel, "while away").await;
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &["--once", "--state-file", state_arg],
    ))
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!("[{missed}] #talk {}: while away\n", other.name)
    );
    assert_eq!(
        std::fs::read_to_string(&state).expect("state"),
        format!("{missed}\n")
    );

    // And not again by the one after.
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &["--once", "--timeout", "1", "--state-file", state_arg],
    ))
    .await;
    assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
}

#[tokio::test]
async fn after_id_seeds_a_missing_or_empty_state_file() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    let first = other.send(&channel, "one").await;
    let second = other.send(&channel, "two").await;
    let home = home("seed");
    let state = home.join("state");
    let state_arg = state.to_str().expect("path");
    let first_arg = first.to_string();
    for empty in [false, true] {
        if empty {
            std::fs::write(&state, "").expect("empty state file");
        } else if state.exists() {
            std::fs::remove_file(&state).expect("remove state file");
        }
        let output = finish(watch(
            &server,
            &home,
            &me.token,
            &[
                "--once",
                "--after-id",
                &first_arg,
                "--state-file",
                state_arg,
            ],
        ))
        .await;
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(
            text(&output.stdout),
            format!("[{second}] #talk {}: two\n", other.name)
        );
        assert_eq!(
            std::fs::read_to_string(&state).expect("state"),
            format!("{second}\n")
        );

        // Now the file holds an ID, and it wins over --after-id.
        let output = finish(watch(
            &server,
            &home,
            &me.token,
            &[
                "--once",
                "--timeout",
                "1",
                "--after-id",
                "0",
                "--state-file",
                state_arg,
            ],
        ))
        .await;
        assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "");
    }
}

/// How `Proxy` holds up the first `wait_for_messages` call.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stall {
    /// Takes the request and never answers it.
    Silent,
    /// Passes the request on and the answer's headers back, then nothing
    /// more, with the connection left open.
    MidStream,
}

/// A proxy in front of a server that passes everything through except the
/// first `wait_for_messages` request, which it holds up as its `Stall`
/// says. `held` is notified once it has. Dropping it closes every
/// connection.
struct Proxy {
    url: String,
    held: std::sync::Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    async fn start(server: &common::Server, stall: Stall) -> Proxy {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let target = server
            .base
            .strip_prefix("http://")
            .expect("server address")
            .to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let held = Arc::new(tokio::sync::Notify::new());
        let unused = Arc::new(AtomicBool::new(true));
        let task = {
            let held = held.clone();
            tokio::spawn(async move {
                // Dropped with this task, which aborts every connection.
                let mut connections = tokio::task::JoinSet::new();
                while let Ok((socket, _)) = listener.accept().await {
                    let upstream = tokio::net::TcpStream::connect(&target)
                        .await
                        .expect("connect to the server");
                    connections.spawn(relay(socket, upstream, stall, unused.clone(), held.clone()));
                }
            })
        };
        Proxy {
            url: format!("http://{address}/mcp"),
            held,
            task,
        }
    }
}

/// Passes bytes both ways between `client` and `upstream`, unless this is
/// the first `wait_for_messages` request while `unused` is set.
async fn relay(
    client: tokio::net::TcpStream,
    upstream: tokio::net::TcpStream,
    stall: Stall,
    unused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    held: std::sync::Arc<tokio::sync::Notify>,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut client_in, mut client_out) = client.into_split();
    let (mut upstream_in, mut upstream_out) = upstream.into_split();
    let stalled = AtomicBool::new(false);
    let found = |seen: &[u8], what: &[u8]| seen.windows(what.len()).any(|w| w == what);
    let up = async {
        let mut seen = Vec::new();
        let mut buffer = [0u8; 16384];
        while let Ok(n) = client_in.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buffer[..n]);
            if !stalled.load(Ordering::SeqCst)
                && found(&seen, b"wait_for_messages")
                && unused.swap(false, Ordering::SeqCst)
            {
                stalled.store(true, Ordering::SeqCst);
                held.notify_one();
                if stall == Stall::Silent {
                    // Keep the connection, read nothing more, answer nothing.
                    std::future::pending::<()>().await;
                }
            }
            if upstream_out.write_all(&buffer[..n]).await.is_err() {
                break;
            }
        }
    };
    let down = async {
        let mut answer = Vec::new();
        let mut headers_sent = false;
        let mut buffer = [0u8; 16384];
        while let Ok(n) = upstream_in.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            if !stalled.load(Ordering::SeqCst) {
                if client_out.write_all(&buffer[..n]).await.is_err() {
                    break;
                }
                continue;
            }
            // Held up: only the headers, once, then the rest is dropped.
            answer.extend_from_slice(&buffer[..n]);
            if headers_sent {
                continue;
            }
            if let Some(end) = answer.windows(4).position(|w| w == b"\r\n\r\n") {
                headers_sent = true;
                if client_out.write_all(&answer[..end + 4]).await.is_err() {
                    break;
                }
            }
        }
        // Leave the client's side open even when the server's closes.
        std::future::pending::<()>().await;
    };
    tokio::join!(up, down);
}

/// A clock that moves only when told to, for `watch::run`.
struct ManualClock(tokio::sync::watch::Sender<chrono::DateTime<chrono::Utc>>);

impl ManualClock {
    fn new() -> ManualClock {
        ManualClock(tokio::sync::watch::Sender::new(chrono::Utc::now()))
    }

    fn advance(&self, by: chrono::Duration) {
        self.0.send_modify(|now| *now += by);
    }
}

impl kashkeshet::clock::Clock for ManualClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        *self.0.borrow()
    }

    fn sleep_until(
        &self,
        deadline: chrono::DateTime<chrono::Utc>,
    ) -> futures::future::BoxFuture<'_, ()> {
        let mut now = self.0.subscribe();
        Box::pin(async move {
            now.wait_for(|t| *t >= deadline)
                .await
                .expect("the clock outlives its sleeps");
        })
    }
}

#[derive(Default)]
struct Lines {
    lines: Vec<String>,
    notes: Vec<String>,
}

impl watch::Report for Lines {
    fn message(&mut self, line: &str) -> Result<(), String> {
        self.lines.push(line.into());
        Ok(())
    }

    fn note(&mut self, text: &str) {
        self.notes.push(text.into());
    }
}

/// The watch, through the MCP client, against a real server whose first
/// wait is held up by `stall`. Once it is, another agent sends a message and
/// the test clock moves on: the watch must give up on the held call,
/// connect again, and print that message, exactly once.
async fn recovers_from(stall: Stall) {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    let before = other.send(&channel, "before").await;
    let proxy = Proxy::start(&server, stall).await;
    let clock = ManualClock::new();
    let remote = watch::Remote::new(proxy.url.clone(), Some(KEY.into()), me.token.clone());
    let options = watch::Options {
        channels: None,
        channel_names: Vec::new(),
        mentions: false,
        after_id: Some(before),
        once: true,
        timeout: None,
        full: false,
    };
    let mut report = Lines::default();
    let sent = tokio::sync::OnceCell::new();
    let driving = async {
        proxy.held.notified().await;
        sent.set(other.send(&channel, "after the stall").await)
            .expect("sent once");
        // Fast enough to pass the call's limit in a moment, slow enough
        // for a real call to finish well within it.
        loop {
            clock.advance(chrono::Duration::seconds(1));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let outcome = tokio::time::timeout(TIMEOUT, async {
        tokio::select! {
            outcome = watch::run(&remote, &clock, None, &mut report, &options) => outcome,
            () = driving => unreachable!("the clock never stops"),
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{stall:?}: the watch did not recover"));
    assert_eq!(outcome, Ok(watch::Outcome::Received), "{stall:?}");
    let sent = sent.get().expect("a message was sent");
    assert_eq!(
        report.lines,
        vec![format!("[{sent}] #talk {}: after the stall", other.name)],
        "{stall:?}"
    );
    assert!(
        report.notes.iter().any(|n| n.contains("no answer in time")),
        "{stall:?}: {:?}",
        report.notes
    );
}

#[tokio::test]
async fn a_wait_the_server_never_answers_is_retried() {
    recovers_from(Stall::Silent).await;
}

#[tokio::test]
async fn a_wait_whose_answer_stops_partway_is_retried() {
    recovers_from(Stall::MidStream).await;
}

#[tokio::test]
async fn the_timeout_ends_it_with_no_library_lines() {
    // The MCP library can log errors of its own when the timeout drops a
    // call it has under way, depending on timing; they are not the
    // command's to print. (`watch_logs_only_kkts_own_lines` in main.rs
    // checks the filter that keeps them out.)
    let server = start().await;
    let me = Agent::new(&server).await;
    let home = home("quiet-timeout");
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &["--once", "--timeout", "1"],
    ))
    .await;
    assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
    let stderr = text(&output.stderr);
    assert!(
        stderr.lines().all(|line| line.starts_with("kkt watch: ")),
        "{stderr}"
    );
}

#[tokio::test]
async fn makes_the_token_file_private_before_watching() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    let before = other.send(&channel, "hi").await - 1;
    let home = home("token-file");
    let file = home.join("token");
    std::fs::write(&file, format!("{}\n", me.token)).expect("token file");
    let args = [
        "--once",
        "--after-id",
        &before.to_string(),
        "--token-file",
        file.to_str().expect("path"),
    ];

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let mut command = watch(&server, &home, "", &args);
        command.env_remove("KKT_TOKEN");
        let output = finish(command).await;
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert!(text(&output.stdout).contains(": hi"));
        assert!(
            text(&output.stderr).contains("was readable by others and is now private"),
            "{}",
            text(&output.stderr)
        );
        assert!(text(&output.stderr).contains("get a new token with `reclaim`"));
        assert_eq!(
            text(&output.stderr)
                .lines()
                .filter(|line| !line.contains("watching as"))
                .count(),
            1
        );
        assert_eq!(
            std::fs::metadata(&file).expect("file").permissions().mode() & 0o777,
            0o600
        );
        assert_no_secrets(&output, &me.token);
    }

    let mut command = watch(&server, &home, "", &args);
    command.env_remove("KKT_TOKEN");
    let output = finish(command).await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains(": hi"));
    assert!(!text(&output.stderr).contains("is now private"));
    assert_eq!(text(&output.stderr).lines().count(), 1);
    assert_no_secrets(&output, &me.token);
}

#[tokio::test]
async fn a_bad_token_key_or_address_ends_it_with_an_error() {
    let server = start().await;
    let me = Agent::new(&server).await;

    let home = home("bad-token");
    let output = finish(watch(&server, &home, "act_not_a_real_token", &["--once"])).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("unauthenticated"),
        "{}",
        text(&output.stderr)
    );

    let home = self::home("bad-key");
    std::fs::write(home.join("key"), "fedcba9876543210fedcba9876543210").expect("key");
    let output = finish(watch(&server, &home, &me.token, &["--once"])).await;
    assert_eq!(output.status.code(), Some(1));
    assert_no_secrets(&output, &me.token);

    let home = self::home("no-token");
    let mut command = watch(&server, &home, "", &["--once"]);
    command.env_remove("KKT_TOKEN");
    let output = finish(command).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("KKT_TOKEN"),
        "{}",
        text(&output.stderr)
    );

    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("address");
    let home = self::home("bad-url");
    let url = format!("http://{free}/mcp");
    let output = finish(watch_at(&url, &home, &me.token, &["--once"])).await;
    assert_eq!(output.status.code(), Some(1));
    assert_no_secrets(&output, &me.token);
}

#[tokio::test]
async fn the_timeout_ends_it_when_the_server_never_answers() {
    // Accepts connections and reads nothing, answers nothing.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let home = home("silent");
    let url = format!("http://{address}/mcp");
    let started = std::time::Instant::now();
    let output = finish(watch_at(
        &url,
        &home,
        "act_unused",
        &["--once", "--timeout", "1"],
    ))
    .await;
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("before the timeout"),
        "{}",
        text(&output.stderr)
    );
    silent.abort();
}

#[tokio::test]
async fn a_timeout_too_long_for_the_date_is_an_error() {
    let home = home("long-timeout");
    let output = finish(watch_at(
        "http://127.0.0.1:1/mcp",
        &home,
        "act_unused",
        &["--once", "--timeout", "1000000000000000"],
    ))
    .await;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    assert!(
        text(&output.stderr).contains("timeout of 1000000000000000 seconds is too long"),
        "{}",
        text(&output.stderr)
    );
}

#[tokio::test]
async fn full_prints_a_body_too_long_for_the_server_answer() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("talk", &[&other]).await;
    // Fits the body limit but not, with the rest of the answer, the
    // response cap, so the wait itself cuts it.
    let body = "z".repeat(common::test_config().limits.max_response_bytes);
    let id = other.send(&channel, &body).await;
    let waited = me
        .tool(
            "wait_for_messages",
            json!({"after_id": id - 1, "timeout_s": 0}),
        )
        .await;
    assert_eq!(waited["messages"][0]["cut"], json!(true), "{waited}");
    let after = (id - 1).to_string();
    let home = home("full");

    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &["--once", "--full", "--after-id", &after],
    ))
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!("[{id}] #talk {}: {body}\n", other.name)
    );
}

#[tokio::test]
async fn channel_names_mix_with_ids_and_repeat() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let first = me.channel("first", &[&other]).await;
    let second = me.channel("second", &[&other]).await;
    let third = me.channel("third", &[&other]).await;
    let skipped = me.channel("skipped", &[&other]).await;
    let before = other.send(&skipped, "skip").await;
    other.send(&first, "one").await;
    other.send(&second, "two").await;
    other.send(&third, "three").await;
    other.send(&skipped, "skip").await;
    let home = home("channel-names");
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &[
            "--once",
            "--after-id",
            &before.to_string(),
            "--channel-name",
            "first",
            "--channel-name",
            "second",
            "--channel",
            &third,
        ],
    ))
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("#first"), "{stdout}");
    assert!(stdout.contains("#second"), "{stdout}");
    assert!(stdout.contains("#third"), "{stdout}");
    assert!(!stdout.contains("#skipped"), "{stdout}");
}

#[tokio::test]
async fn channel_names_reject_missing_and_ambiguous_names_across_pages() {
    let server = start().await;
    let me = Agent::new(&server).await;
    let first = me.channel("z-duplicate", &[]).await;
    // Enough metadata to force list_channels to return several pages.
    for i in 0..30 {
        me.channel(&format!("filler-{i}"), &[]).await;
    }
    let second = me.channel("z-duplicate", &[]).await;
    let home = home("ambiguous-names");
    for (name, expected) in [
        ("absent", "no visible channel"),
        ("z-duplicate", "several visible channels"),
    ] {
        let output = finish(watch(
            &server,
            &home,
            &me.token,
            &["--once", "--channel-name", name],
        ))
        .await;
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        let stderr = text(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        if name == "z-duplicate" {
            assert!(stderr.contains(&first), "{stderr}");
            assert!(stderr.contains(&second), "{stderr}");
        }
        assert!(output.stdout.is_empty());
    }
}

#[tokio::test]
async fn mentions_fetch_the_whole_body_and_save_skipped_messages() {
    let server = start().await;
    let (me, other) = (Agent::new(&server).await, Agent::new(&server).await);
    let channel = me.channel("mentions", &[&other]).await;
    let home = home("mentions");
    let state = home.join("state");
    std::fs::write(&state, "0\n").unwrap();
    other.send(&channel, "nothing here").await;
    other.send(&channel, &format!("{}-helper", me.name)).await;
    let body = format!(
        "{} @{}",
        "z".repeat(common::test_config().limits.max_response_bytes - me.name.len() - 2),
        me.name
    );
    let mentioned = other.send(&channel, &body).await;
    other.send(&channel, "also skipped").await;
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &[
            "--once",
            "--mentions",
            "--state-file",
            state.to_str().unwrap(),
        ],
    ))
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.starts_with(&format!("[{mentioned}]")), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        format!("{mentioned}\n")
    );

    let last = other.send(&channel, "still no mention").await;
    let output = finish(watch(
        &server,
        &home,
        &me.token,
        &[
            "--once",
            "--mentions",
            "--timeout",
            "1",
            "--state-file",
            state.to_str().unwrap(),
        ],
    ))
    .await;
    assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
    assert!(output.stdout.is_empty());
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        format!("{last}\n")
    );
}

// ---- --admin ----

const ADMIN_KEY: &str = "admin-0123456789abcdef0123456789abcdef";

async fn start_with_admin() -> common::Server {
    let mut config = common::test_config();
    config.admission = Admission::SharedKey(SharedKey::new(KEY, "test").expect("key"));
    config.admin_key = Some(SharedKey::new(ADMIN_KEY, "test").expect("admin key"));
    common::start_with(config).await
}

/// `kkt watch --admin` against `server`, with the admin key in `home` and
/// no token anywhere.
fn watch_admin(server: &common::Server, home: &Path, args: &[&str]) -> Command {
    std::fs::write(home.join("admin-key"), ADMIN_KEY).expect("admin key file");
    let mut command = Command::new(KKT);
    command
        .args(["watch", "--admin", "--url", &format!("{}/mcp", server.base)])
        .arg("--key-file")
        .arg(home.join("admin-key"))
        .args(args)
        .env("HOME", home)
        .env_remove("KKT_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

#[tokio::test]
async fn admin_watch_sees_a_closed_channel_it_is_not_in() {
    let server = start_with_admin().await;
    let (a, b) = (Agent::new(&server).await, Agent::new(&server).await);
    let closed = a.channel("private", &[&b]).await;
    let home = home("admin-closed");
    let child = watch_admin(&server, &home, &["--once", "--timeout", "15"])
        .spawn()
        .expect("start");
    // It starts after the latest message, so keep sending until it sees one.
    let sender = async {
        loop {
            b.send(&closed, "between the two of us").await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    let output = tokio::select! {
        output = tokio::time::timeout(TIMEOUT, child.wait_with_output()) => {
            output.expect("finished").expect("output")
        }
        () = sender => unreachable!(),
    };
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(&format!("#private {}: between the two of us", b.name)),
        "{stdout}"
    );
    assert!(text(&output.stderr).contains("watching as (admin)"));
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !text(stream).contains(ADMIN_KEY),
            "the admin key was printed"
        );
    }
}

#[tokio::test]
async fn admin_watch_sees_messages_in_a_hidden_channel() {
    let server = start_with_admin().await;
    let a = Agent::new(&server).await;
    let gone = a.channel("gone", &[]).await;
    let kept = a.send(&gone, "said before hiding").await;
    a.tool("hide_channel", json!({ "channel_id": gone })).await;
    let home = home("admin-hidden");
    let output = finish(watch_admin(
        &server,
        &home,
        &["--once", "--after-id", &(kept - 1).to_string()],
    ))
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).starts_with(&format!("[{kept}] #gone {}: said before hiding", a.name)),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn admin_watch_refuses_other_keys_and_agent_options() {
    let server = start_with_admin().await;
    let home = home("admin-refused");
    std::fs::write(home.join("admin-key"), KEY).expect("key");
    let mut command = Command::new(KKT);
    command
        .args(["watch", "--admin", "--once", "--timeout", "5"])
        .args(["--url", &format!("{}/mcp", server.base)])
        .arg("--key-file")
        .arg(home.join("admin-key"))
        .env("HOME", home.path())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = finish(command).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(!text(&output.stdout).contains(KEY));
    assert!(!text(&output.stderr).contains(KEY));

    for other in ["--mentions", "--token-file=/nowhere"] {
        let output = finish(watch_admin(&server, &home, &["--once", other])).await;
        assert_eq!(output.status.code(), Some(2), "{other}");
    }
}

/// Resuming from `--after-id` or a saved place skips the call for the
/// latest message, so a wrong admin key, or a server with no admin
/// endpoint, must still be found before watching starts, not retried.
#[tokio::test]
async fn admin_watch_checks_the_key_when_it_resumes() {
    let with_admin = start_with_admin().await;
    let mut config = common::test_config();
    config.admission = Admission::SharedKey(SharedKey::new(KEY, "test").expect("key"));
    let without_admin = common::start_with(config).await;
    let home = home("admin-resume");
    let state = home.join("state");
    std::fs::write(&state, "0\n").expect("state file");
    let state_arg = format!("--state-file={}", state.display());
    for (server, wrong_key) in [(&with_admin, true), (&without_admin, false)] {
        for resume in [vec!["--after-id", "0"], vec![state_arg.as_str()]] {
            let mut command = watch_admin(server, &home, &["--once", "--timeout", "5"]);
            command.args(&resume);
            if wrong_key {
                std::fs::write(home.join("admin-key"), KEY).expect("wrong key");
            }
            let output = finish(command).await;
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{resume:?}: {stderr}");
            assert!(!stderr.contains("watching as"), "{resume:?}: {stderr}");
            assert!(!stderr.contains(KEY) && !stderr.contains(ADMIN_KEY));
        }
    }
}
