//! Shared by the integration tests: start a server on a free port and connect
//! an MCP client to it.

// Each test file builds this module on its own, and none uses every helper.
#![allow(dead_code)]

use kashkeshet::admission::Admission;
use kashkeshet::app::{self, AppConfig, Parts};
use kashkeshet::clock::{Clock, SystemClock};
use kashkeshet::config::{Limits, LogLevel, LogPolicy};
use kashkeshet::hosts::AllowedHosts;
use kashkeshet::names::OsEntropy;
use kashkeshet::provenance::{Build, RunSettings, ServerRun};
use kashkeshet::shutdown::Shutdown;
use kashkeshet::store::Db;
use rmcp::model::ProtocolVersion;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use std::sync::Arc;

pub struct Server {
    pub base: String,
    shutdown: Shutdown,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.stop();
    }
}

/// The settings `start` uses.
pub fn test_config() -> AppConfig {
    AppConfig {
        admission: Admission::Open,
        web_admission: Admission::Open,
        admin_key: None,
        admin_name: None,
        limits: Limits::DEFAULT,
        allowed_origins: vec!["http://localhost:6274".into()],
        allowed_hosts: AllowedHosts::new(&[]).expect("loopback hosts"),
        instructions: "test instructions: call register".into(),
    }
}

/// What a test server records about its settings: those of `config`, with
/// no config file or log file and the default log levels.
fn run_settings(config: &AppConfig) -> RunSettings {
    RunSettings {
        allow_any_bind: false,
        limits: config.limits.clone(),
        log_level: LogLevel::Info,
        dependency_log_level: LogLevel::Warn,
        allowed_origins: config.allowed_origins.clone(),
        allowed_hosts: config.allowed_hosts.extra().to_vec(),
        config_file: None,
        access_key_file: config.admission.key_source().map(str::to_string),
        web_key_file: config.web_admission.key_source().map(str::to_string),
        admin_key_file: config
            .admin_key
            .as_ref()
            .map(|key| key.source().to_string()),
        admin_name: config.admin_name.as_ref().map(|name| name.to_string()),
        log_file: None,
        log_policy: LogPolicy::DEFAULT,
        instructions: config.instructions.clone(),
    }
}

pub async fn start() -> Server {
    start_with(test_config()).await
}

pub async fn start_with(config: AppConfig) -> Server {
    start_with_clock(config, Arc::new(SystemClock)).await
}

/// Starts a server whose time comes from `clock`.
pub async fn start_with_clock(config: AppConfig, clock: Arc<dyn Clock>) -> Server {
    start_on(config, clock, Db::open_in_memory().expect("db")).await
}

/// Starts a server on `db`, such as a database file an older version made.
pub async fn start_on(config: AppConfig, clock: Arc<dyn Clock>, db: Db) -> Server {
    let shutdown = Shutdown::new();
    let run = ServerRun::new(Build::CURRENT, &run_settings(&config));
    let run_id = app::record_run(&db, &run, clock.now())
        .await
        .expect("record run");
    let parts = Parts::new(db, clock, Arc::new(OsEntropy), &config.limits, run_id);
    let router = app::build(parts, config, shutdown.child());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let ct = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { ct.stopped().await })
            .await
            .expect("serve");
    });
    Server {
        base: format!("http://{addr}"),
        shutdown,
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
