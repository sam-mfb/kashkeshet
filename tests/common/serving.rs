//! Start `kkt serve` as its own process on a free port.
//!
//! `start` gives the server `--bind 127.0.0.1:0` and reads the port it got
//! from its "bound to" line, so no other process can take the port between
//! choosing it and listening on it. A server that exits before that line
//! fails the test with its exit status and what it printed to standard
//! error.
//!
//! `start_quiet` is for a server whose standard output does not carry that
//! line, because it logs to a file or leaves out info lines. It picks a
//! free port itself and starts the server on it; if another process took
//! the port in between, it tries again on a new one. Any other failure, and
//! running out of ports, fails the test with each server's exit status and
//! output.

// Each test file builds this module on its own, and none uses every item.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader, Lines};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

/// How long a server may take to start before the test fails.
const START_TIMEOUT: Duration = Duration::from_secs(20);

/// The prefix of the line `kkt serve` prints with the address it bound.
const BOUND: &str = "bound to ";

/// A running `kkt serve`.
pub struct Server {
    pub child: Child,
    /// The address it listens on.
    pub addr: SocketAddr,
    /// What it printed to standard output up to and including its "bound
    /// to" line.
    pub log: String,
    /// Reads the rest of standard output, so the server never blocks on a
    /// full pipe, and returns it at the end.
    rest: JoinHandle<String>,
}

impl Server {
    /// Its address as `127.0.0.1:<port>`.
    pub fn bind(&self) -> String {
        self.addr.to_string()
    }

    /// Its base URL, `http://127.0.0.1:<port>`.
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// What it printed to standard output after `log`, once it has exited
    /// or closed standard output.
    pub async fn rest_of_log(&mut self) -> String {
        (&mut self.rest).await.expect("read standard output")
    }
}

/// Starts `command`, a `kkt serve` without `--bind`, on a port the system
/// chooses, and returns once it is listening. Panics, with the server's exit
/// status and standard error, if it exits first.
pub async fn start(command: Command) -> Server {
    start_on(command, "127.0.0.1:0").await
}

/// As `start`, on `bind`, whose port should be 0.
pub async fn start_on(command: Command, bind: &str) -> Server {
    match try_start_on(command, bind).await {
        Ok(server) => server,
        Err(failure) => panic!("{failure}"),
    }
}

/// As `start`, but returns why the server did not start instead of
/// panicking.
pub async fn try_start(command: Command) -> Result<Server, String> {
    try_start_on(command, "127.0.0.1:0").await
}

async fn try_start_on(mut command: Command, bind: &str) -> Result<Server, String> {
    let mut child = command
        .args(["--bind", bind])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("start kkt");
    let stderr = collect(
        BufReader::new(child.stderr.take().expect("stderr")).lines(),
        true,
    );
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let mut log = String::new();
    let addr = tokio::time::timeout(START_TIMEOUT, async {
        while let Some(line) = lines.next_line().await.expect("read stdout") {
            log.push_str(&line);
            log.push('\n');
            if let Some((_, addr)) = line.split_once(BOUND) {
                return Some(addr.trim().parse::<SocketAddr>().expect("bound address"));
            }
        }
        None
    })
    .await;
    let addr = match addr {
        Ok(Some(addr)) => addr,
        Ok(None) => {
            let status = tokio::time::timeout(START_TIMEOUT, child.wait())
                .await
                .expect("kkt closed standard output but did not exit")
                .expect("wait for kkt");
            let stderr = stderr.await.expect("read stderr");
            return Err(format!(
                "kkt exited before listening, with {status}\n\
                 standard output:\n{log}standard error:\n{stderr}"
            ));
        }
        Err(_) => {
            child.kill().await.expect("stop kkt");
            let stderr = stderr.await.expect("read stderr");
            return Err(format!(
                "kkt did not start in time\n\
                 standard output:\n{log}standard error:\n{stderr}"
            ));
        }
    };
    let rest = collect(lines, false);
    Ok(Server {
        child,
        addr,
        log,
        rest,
    })
}

/// How many ports `start_quiet` tries before it gives up.
const QUIET_ATTEMPTS: usize = 5;

/// How long one request asking whether the server answers may take: a port
/// another process holds may accept the connection and never reply.
const ASK_TIMEOUT: Duration = Duration::from_secs(1);

/// A port that was free a moment ago: bound and released at once, so
/// another process may take it before it is used.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("address")
        .port()
}

/// Starts the `kkt serve` that `make` builds for an address such as
/// `127.0.0.1:7420`, without `--bind`, on a free port. Returns it and its
/// address once it answers. If another process took the port first, tries
/// again on a new one. Panics, with the server's exit status and standard
/// error, if it fails for any other reason, or with those of every server
/// it tried if all the ports were taken.
pub async fn start_quiet(make: impl FnMut(&str) -> Command) -> (Child, String) {
    let ports = std::iter::repeat_with(free_port).take(QUIET_ATTEMPTS);
    match try_start_quiet(make, ports).await {
        Ok(started) => started,
        Err(failure) => panic!("{failure}"),
    }
}

/// As `start_quiet`, trying `ports` in turn, but returns why the server did
/// not start instead of panicking.
pub async fn try_start_quiet(
    mut make: impl FnMut(&str) -> Command,
    ports: impl IntoIterator<Item = u16>,
) -> Result<(Child, String), String> {
    let mut taken = Vec::new();
    for port in ports {
        let bind = format!("127.0.0.1:{port}");
        match start_quiet_on(make(&bind), &bind).await {
            Quiet::Started(child) => return Ok((child, bind)),
            Quiet::Taken(why) => taken.push(why),
            Quiet::Failed(why) => return Err(why),
        }
    }
    Err(format!(
        "kkt could not listen on any port it tried, each held by another process:\n{}",
        taken.join("\n")
    ))
}

/// How one try of `start_quiet_on` ended.
enum Quiet {
    /// The server answers.
    Started(Child),
    /// The server said it could not listen, and another process holds the
    /// port; with why, as for `Failed`.
    Taken(String),
    /// The server failed for another reason; with its exit status and what
    /// it printed.
    Failed(String),
}

/// Starts `command` on `bind` and returns it once it answers.
async fn start_quiet_on(mut command: Command, bind: &str) -> Quiet {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // A name only this server allows, so that another server on the port,
    // which refuses it, is not taken for this one.
    let host = format!(
        "kkt-test-{}-{}.example",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut child = command
        .args(["--bind", bind, "--allow-host", &host])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("start kkt");
    let http = reqwest::Client::builder()
        .timeout(ASK_TIMEOUT)
        .build()
        .expect("HTTP client");
    let answered = tokio::time::timeout(START_TIMEOUT, async {
        loop {
            if child.try_wait().expect("check on kkt").is_some() {
                return false;
            }
            let reply = http
                .get(format!("http://{bind}/"))
                .header("host", &host)
                .send()
                .await;
            if reply.is_ok_and(|reply| reply.status().is_success()) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if matches!(answered, Ok(true)) {
        return Quiet::Started(child);
    }
    if answered.is_err() {
        child.kill().await.expect("stop kkt");
    }
    let out = child.wait_with_output().await.expect("wait for kkt");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let what = match answered {
        Ok(_) => format!("kkt exited before listening on {bind}, with {}", out.status),
        Err(_) => format!("kkt did not start on {bind} in time"),
    };
    let why = format!("{what}\nstandard output:\n{stdout}standard error:\n{stderr}");
    // The port counts as taken only if the server exited saying it could
    // not listen there and another process does hold it. kkt serve says so
    // in its log on standard output, or, when it logs to a file, also on
    // standard error. Any other failure is reported, even on a port another
    // process holds.
    let said = format!("could not listen on {bind}:");
    let could_not_listen = stdout.contains(&said) || stderr.contains(&said);
    if answered.is_ok() && could_not_listen && std::net::TcpListener::bind(bind).is_err() {
        eprintln!("kkt could not listen on {bind}, which another process holds");
        return Quiet::Taken(why);
    }
    Quiet::Failed(why)
}

/// Reads `lines` to the end in the background and returns what it read;
/// with `echo`, also passes each line on to the test's standard error, so a
/// failing test shows it as before.
fn collect<R>(mut lines: Lines<BufReader<R>>, echo: bool) -> JoinHandle<String>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut text = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if echo {
                eprintln!("{line}");
            }
            text.push_str(&line);
            text.push('\n');
        }
        text
    })
}
