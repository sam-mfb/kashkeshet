//! Shared by the integration tests: start a server on a free port and connect
//! an MCP client to it.
//!
//! `start` and `start_with` run the server the way `KKT_TEST_PROGRAM` says:
//! unset, in this process through kkt-server; `kkt`, as `kkt serve` in a
//! process of its own; `kkt-keymode`, as the second program, which must be
//! built first (`cargo build -p kkt-keymode`). A program is given the test's
//! settings as a config file and key files in a scratch folder. The other
//! ways to start run in this process whatever the variable says: a clock or
//! a database can only be handed to a server here, and `start_in_process`
//! is for settings no program's files can say.

// Each test file builds this module on its own, and none uses every helper.
#![allow(dead_code)]

pub mod serving;

use kashkeshet::machine::settings::{
    format_settings, rules_settings, session_settings, web_settings,
};
use kkt_admission::{AdminAdmission, Admission, ServerKey, SharedKey};
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_config::{LogLevel, LogPolicy};
use kkt_entropy_os::OsEntropy;
use kkt_hosts::AllowedHosts;
use kkt_identity::KeyMode;
use kkt_names::{AdminName, MAX_ADMIN_NAME_CHARS};
use kkt_provenance::{Build, RunSettings, ServerRun};
use kkt_server::{self as app, AppConfig, Parts};
use kkt_settings::{Limits, ServeOptions};
use kkt_shutdown::Shutdown;
use kkt_store::Db;
use rmcp::model::ProtocolVersion;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Server {
    pub base: String,
    running: Running,
}

/// What keeps a test server running; dropping it stops the server.
enum Running {
    InProcess(Shutdown),
    /// A program in a process of its own, killed when dropped, and the
    /// folder with its settings and database.
    Process(Box<serving::Server>, kkt_scratch::Dir),
}

impl Drop for Server {
    fn drop(&mut self) {
        match &self.running {
            Running::InProcess(shutdown) => shutdown.stop(),
            // `serving::Server` kills its process when dropped.
            Running::Process(..) => {}
        }
    }
}

/// A test server's settings, written as a program's files would give them:
/// keys as their text, so they can be written to key files.
pub struct TestConfig {
    /// The access key; `None`: `/mcp` is open.
    pub access_key: Option<String>,
    /// The web key; `None`: the web page is open in this process. A program
    /// with an access key always has a web key: it is made up for it here,
    /// as `kkt serve` makes one.
    pub web_key: Option<String>,
    /// The admin key; `None`: `/mcp-admin` is off. Needs an access key.
    pub admin_key: Option<String>,
    /// The name tied to the admin key. `None`: the admin key may only read.
    pub admin_name: Option<String>,
    pub limits: Limits,
    pub allowed_origins: Vec<String>,
    /// Host names allowed besides the loopback ones.
    pub allowed_hosts: Vec<String>,
    /// The instructions file's text; `None`: the built-in instructions.
    pub instructions: Option<String>,
}

/// The settings `start` uses.
pub fn test_config() -> TestConfig {
    TestConfig {
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
        limits: Limits::DEFAULT,
        allowed_origins: vec!["http://localhost:6274".into()],
        allowed_hosts: Vec::new(),
        instructions: Some("test instructions: call register".into()),
    }
}

/// Where a key in a test's settings came from, for the record of the run.
const KEY_SOURCE: &str = "test";

impl TestConfig {
    fn admin_name(&self) -> Option<AdminName> {
        self.admin_name
            .as_ref()
            .map(|name| AdminName::new(name, MAX_ADMIN_NAME_CHARS).expect("admin name"))
    }

    /// kkt-server's settings, for a server in this process.
    fn app_config(&self) -> AppConfig {
        let admission = |which, key: &Option<String>| match key {
            Some(text) => {
                Admission::SharedKey(SharedKey::new(which, text, KEY_SOURCE).expect("key"))
            }
            None => Admission::Open,
        };
        AppConfig {
            admission: admission(ServerKey::Access, &self.access_key),
            web_admission: admission(ServerKey::Web, &self.web_key),
            admin_admission: self.admin_key.as_ref().map(|text| {
                AdminAdmission::SharedKey(
                    SharedKey::new(ServerKey::Admin, text, KEY_SOURCE).expect("key"),
                )
            }),
            rules: rules_settings(&self.limits),
            web: web_settings(&self.limits),
            sessions: session_settings(&self.limits),
            allowed_origins: self.allowed_origins.clone(),
            allowed_hosts: AllowedHosts::new(&self.allowed_hosts).expect("allowed hosts"),
            instructions: self.instructions.clone(),
        }
    }
}

/// What a test server records about its settings: `limits` and those of
/// `config`, with no config file or log file and the default log levels.
fn run_settings(
    limits: &Limits,
    config: &AppConfig,
    admin_name: Option<&AdminName>,
) -> RunSettings {
    RunSettings {
        allow_any_bind: false,
        limits: limits.clone(),
        log_level: LogLevel::Info,
        dependency_log_level: LogLevel::Warn,
        allowed_origins: config.allowed_origins.clone(),
        allowed_hosts: config.allowed_hosts.extra().to_vec(),
        config_file: None,
        access_key_file: config.admission.key_source().map(str::to_string),
        web_key_file: config.web_admission.key_source().map(str::to_string),
        admin_key_file: config
            .admin_admission
            .as_ref()
            .and_then(|admission| admission.key_source())
            .map(str::to_string),
        admin_name: admin_name.map(|name| name.to_string()),
        log_file: None,
        log_policy: LogPolicy::DEFAULT,
        instructions: config.agent_instructions().to_string(),
    }
}

/// The program `KKT_TEST_PROGRAM` names, or `None` to run in this process.
pub fn program() -> Option<&'static str> {
    match std::env::var("KKT_TEST_PROGRAM") {
        Err(std::env::VarError::NotPresent) => None,
        Ok(name) if name.is_empty() => None,
        Ok(name) if name == "kkt" => Some("kkt"),
        Ok(name) if name == "kkt-keymode" => Some("kkt-keymode"),
        other => panic!("KKT_TEST_PROGRAM must be kkt or kkt-keymode, not {other:?}"),
    }
}

pub async fn start() -> Server {
    start_with(test_config()).await
}

/// Starts a server with `config`, in the program `KKT_TEST_PROGRAM` names.
pub async fn start_with(config: TestConfig) -> Server {
    match program() {
        None => start_in_process(config).await,
        Some(program) => start_program(program, config).await,
    }
}

/// Starts a server in this process, whatever `KKT_TEST_PROGRAM` says.
pub async fn start_in_process(config: TestConfig) -> Server {
    start_with_clock(config, Arc::new(SystemClock)).await
}

/// Starts a server in this process whose time comes from `clock`.
pub async fn start_with_clock(config: TestConfig, clock: Arc<dyn Clock>) -> Server {
    start_on(config, clock, {
        let place = kkt_db_sqlite::MemoryPlace::new().expect("an in-memory database");
        Db::new(
            place
                .open(&[kkt_store::schema()])
                .expect("the core's tables"),
        )
    })
    .await
}

/// Starts a server in this process on `db`, such as a database file an
/// older version made.
pub async fn start_on(config: TestConfig, clock: Arc<dyn Clock>, db: Db) -> Server {
    let shutdown = Shutdown::new();
    let admin_name = config.admin_name();
    let limits = config.limits.clone();
    let config = config.app_config();
    let run = ServerRun::new(
        Build::CURRENT,
        &run_settings(&limits, &config, admin_name.as_ref()),
    );
    let run_id = app::record_run(&db, &run, clock.now())
        .await
        .expect("record run");
    let parts = Parts::new(
        db,
        clock,
        Arc::new(OsEntropy),
        Arc::new(KeyMode::new(admin_name)),
        &format_settings(&limits),
        &kkt_events::Settings::DEFAULT,
        run_id,
    );
    let server = app::build(parts, config, shutdown.child());
    let listening = app::listen(server, "127.0.0.1:0".parse().expect("address"))
        .await
        .expect("bind");
    let addr = listening.address().expect("addr");
    let ct = shutdown.clone();
    tokio::spawn(async move {
        listening
            .serve_until(async move { ct.stopped().await }, |e| {
                panic!("could not set TCP_NODELAY on a connection: {e}")
            })
            .await
            .expect("serve");
    });
    Server {
        base: format!("http://{addr}"),
        running: Running::InProcess(shutdown),
    }
}

/// Starts `program` with `config` written as its files.
async fn start_program(program: &str, config: TestConfig) -> Server {
    // Each server in this test program gets a folder of its own.
    static SERVERS: AtomicU64 = AtomicU64::new(0);
    let n = SERVERS.fetch_add(1, Ordering::Relaxed);
    let dir = kkt_scratch::Dir::new(&format!("{program}-server-{n}"));
    let config_file = write_settings(dir.path(), &config);
    let path = program_path(program);
    let mut command = tokio::process::Command::new(&path);
    if program == "kkt" {
        command.arg("serve");
        command.env("HOME", dir.path().join("home"));
    }
    command.arg("--config").arg(&config_file);
    let server = serving::start(command).await;
    Server {
        base: server.base(),
        running: Running::Process(Box::new(server), dir),
    }
}

/// The path of `program`'s binary, which sits beside `kkt`'s.
fn program_path(program: &str) -> PathBuf {
    let kkt = PathBuf::from(env!("CARGO_BIN_EXE_kkt"));
    if program == "kkt" {
        return kkt;
    }
    let path = kkt.with_file_name(format!("{program}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "{} is missing: build it first with `cargo build -p {program}`",
        path.display()
    );
    path
}

/// Writes `config` in `dir` as a config file, key files, an admin name file
/// and an instructions file, and returns the config file's path. Panics if
/// a program's files cannot say it: a limit the config file has no setting
/// for, or an admin key without an access key.
fn write_settings(dir: &Path, config: &TestConfig) -> PathBuf {
    let write =
        |path: &Path, text: &str| std::fs::write(path, text).expect("write a settings file");
    let quoted = |path: &Path| toml_string(&path.display().to_string());
    let mut toml = format!("db = {}\n", quoted(&dir.join("kkt.db")));
    let mut options = ServeOptions::NONE;
    match &config.access_key {
        None => {
            assert!(
                config.admin_key.is_none() && config.web_key.is_none(),
                "a program has a web or admin key only beside an access key"
            );
            toml.push_str("no_access_key = true\n");
            options.no_access_key = Some(true);
        }
        Some(access) => {
            let access_path = dir.join("access.key");
            write(&access_path, access);
            toml.push_str(&format!("access_key_file = {}\n", quoted(&access_path)));
            options.access_key_file = Some(access_path.clone());
            // `kkt serve` makes a web key when there is none.
            let web = config
                .web_key
                .clone()
                .unwrap_or_else(|| SharedKey::generate(&OsEntropy));
            write(&kkt_settings::web_key_path(&access_path), &web);
            if let Some(admin) = &config.admin_key {
                let admin_path =
                    kkt_settings::admin_key_path(&access_path).expect("an admin key path");
                write(&admin_path, admin);
                if let Some(name) = config.admin_name() {
                    write(
                        &kkt_settings::admin_name_path(&admin_path),
                        &name.bound_to(admin),
                    );
                }
            }
        }
    }
    if let Some(text) = &config.instructions {
        let path = dir.join("instructions.md");
        write(&path, text);
        toml.push_str(&format!("instructions = {}\n", quoted(&path)));
    }
    let list = |items: &[String]| {
        let items: Vec<String> = items.iter().map(|item| toml_string(item)).collect();
        format!("[{}]", items.join(", "))
    };
    toml.push_str(&format!(
        "allow_origins = {}\n",
        list(&config.allowed_origins)
    ));
    toml.push_str(&format!("allow_hosts = {}\n", list(&config.allowed_hosts)));
    // The limits a config file can set, where the test's differ from the
    // defaults; then a check that they come out as the test's limits.
    let (given, defaults) = (&config.limits, Limits::DEFAULT);
    let mut count = |name: &str, value: usize, default: usize| {
        if value != default {
            toml.push_str(&format!("{name} = {value}\n"));
        }
        (value != default).then_some(value)
    };
    options.max_response_bytes = count(
        "max_response_bytes",
        given.max_response_bytes,
        defaults.max_response_bytes,
    );
    options.default_limit = count("default_limit", given.default_limit, defaults.default_limit);
    options.max_limit = count("max_limit", given.max_limit, defaults.max_limit);
    options.upgrade_backups_kept = count(
        "upgrade_backups_kept",
        given.upgrade_backups_kept,
        defaults.upgrade_backups_kept,
    );
    options.max_lane_chars = count(
        "max_lane_chars",
        given.max_lane_chars,
        defaults.max_lane_chars,
    );
    options.max_memory_entry_chars = count(
        "max_memory_entry_chars",
        given.max_memory_entry_chars,
        defaults.max_memory_entry_chars,
    );
    options.max_memory_chars = count(
        "max_memory_chars",
        given.max_memory_chars,
        defaults.max_memory_chars,
    );
    options.sends_per_minute = count(
        "sends_per_minute",
        given.sends_per_minute as usize,
        defaults.sends_per_minute as usize,
    )
    .map(|n| u32::try_from(n).expect("sends_per_minute"));
    options.waiting_grace_seconds = count(
        "waiting_grace_seconds",
        given.waiting_grace.as_secs() as usize,
        defaults.waiting_grace.as_secs() as usize,
    )
    .map(|n| u32::try_from(n).expect("waiting_grace_seconds"));
    let resolved = options
        .resolve(dir.join("kkt.db"), None)
        .expect("the test's settings resolve");
    assert_eq!(
        &resolved.limits, given,
        "a test's limits must be ones a config file can set to run in a program"
    );
    let path = dir.join("kkt.toml");
    write(&path, &toml);
    path
}

/// `text` as a TOML string.
fn toml_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string")
}

impl Server {
    /// What the program printed up to its "bound to" line; `None` for a
    /// server in this process.
    pub fn program_log(&self) -> Option<&str> {
        match &self.running {
            Running::Process(server, _) => Some(&server.log),
            Running::InProcess(_) => None,
        }
    }

    /// Stops the program this server runs in and waits for it to exit.
    /// Panics if the server runs in this process.
    pub async fn kill_program(&mut self) {
        match &mut self.running {
            Running::Process(server, _) => {
                server.child.kill().await.expect("stop the program");
            }
            Running::InProcess(_) => panic!("this server runs in the test's process"),
        }
    }
}

pub type Client = RunningService<RoleClient, ()>;

pub async fn connect(server: &Server, lifecycle: ClientLifecycleMode) -> Client {
    connect_with(
        StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", server.base)),
        lifecycle,
    )
    .await
}

async fn connect_with(
    config: StreamableHttpClientTransportConfig,
    lifecycle: ClientLifecycleMode,
) -> Client {
    let transport = StreamableHttpClientTransport::from_config(config);
    ().serve_with_lifecycle(transport, lifecycle)
        .await
        .expect("connect")
}

/// Connects as `connect_new` does, sending `Authorization: Bearer <key>`
/// with every request.
pub async fn connect_new_with_key(server: &Server, key: &str) -> Client {
    connect_with(
        StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", server.base))
            .auth_header(key),
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await
}

/// Connects to the admin endpoint, `/mcp-admin`, sending
/// `Authorization: Bearer <key>` with every request.
pub async fn connect_admin_with_key(server: &Server, key: &str) -> Client {
    connect_admin_at(&server.base, key).await
}

/// Connects as `connect_admin_with_key` does, to a server at `base` that
/// this process did not start.
pub async fn connect_admin_at(base: &str, key: &str) -> Client {
    connect_with(
        StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp-admin")).auth_header(key),
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await
}

pub async fn connect_new(server: &Server) -> Client {
    connect_new_at(&server.base).await
}

/// Connects as `connect_new` does, to a server at `base` (such as
/// `http://127.0.0.1:7420`) that this process did not start.
pub async fn connect_new_at(base: &str) -> Client {
    connect_with(
        StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp")),
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await
}
