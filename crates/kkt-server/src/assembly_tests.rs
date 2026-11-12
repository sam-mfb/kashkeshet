//! The `Identify` given through `Parts` is the one both surfaces ask: an
//! agent call on `/mcp` and every request to the web page. A stand-in that
//! refuses everyone, with a reason of its own, shows each surface asked it
//! and nothing else.

use crate::{AppConfig, Parts, build};
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use kkt_admission::{AdminAdmission, Admission, Principal};
use kkt_api_agent::SwarmId;
use kkt_api_agent::{Agent, Operator, RuleError};
use kkt_clock::testing::FixedClock;
use kkt_entropy::testing::SeqEntropy;
use kkt_hosts::AllowedHosts;
use kkt_identity::{Identified, Identify, Identifying};
use kkt_shutdown::Shutdown;
use kkt_web::testing::start_time;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

/// The reason the stand-in gives.
const REFUSED: &str = "refused by the identity given to this server";

struct RefusesEveryone;

impl Identify for RefusesEveryone {
    fn avatar<'a>(
        &'a self,
        _api: &'a dyn kkt_api_agent::Api,
        _principal: &'a Principal,
        _token: Option<&'a str>,
    ) -> Identifying<'a, Identified<Agent>> {
        Box::pin(async { Err(RuleError::invalid(REFUSED)) })
    }

    fn newcomer<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, SwarmId> {
        Box::pin(async { Err(RuleError::invalid(REFUSED)) })
    }

    fn operator<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, Identified<Operator>> {
        Box::pin(async { Err(RuleError::invalid(REFUSED)) })
    }
}

/// An open server, with no keys, built with the stand-in.
async fn router(shutdown: &Shutdown) -> axum::Router {
    router_with(shutdown, None).await
}

/// An open server built with the stand-in, whose `/mcp-admin` admits by
/// `admin`, or is not served.
async fn router_with(shutdown: &Shutdown, admin: Option<AdminAdmission>) -> axum::Router {
    let clock = Arc::new(FixedClock::new(start_time()));
    let entropy = Arc::new(SeqEntropy::new(1));
    let rules = kkt_rules::testing::memory(kkt_rules::testing::Memory {
        events: kkt_events::Events::new(kkt_events::Settings::DEFAULT)
            .until_stopped(shutdown.clone()),
        ..kkt_rules::testing::Memory::new(clock.clone(), entropy.clone())
    })
    .await
    .expect("rules over a new database");
    let parts = Parts::new(
        Arc::new(rules),
        clock,
        entropy,
        Arc::new(RefusesEveryone),
        Arc::new(kkt_log::testing::CapturedLog::default()),
        &crate::ServerSettings::DEFAULT,
        AllowedHosts::new(&[]).expect("loopback hosts"),
    );
    let config = AppConfig {
        admission: Admission::Open,
        web_admission: Admission::Open,
        admin_admission: admin,
        instructions: Some("test instructions".into()),
    };
    build(parts, config, shutdown.child()).router
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response = router.clone().oneshot(request).await.expect("a response");
    let (status, headers) = (response.status(), response.headers().clone());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    (
        status,
        headers,
        String::from_utf8(body.to_vec()).expect("text"),
    )
}

/// A JSON-RPC message to `/mcp`, which keeps no sessions.
fn mcp(message: Value) -> Request<Body> {
    mcp_at("/mcp", message, None)
}

/// A JSON-RPC message to `path`, presenting `bearer` if given.
fn mcp_at(path: &str, message: Value, bearer: Option<&str>) -> Request<Body> {
    let request = Request::post(path);
    let request = match bearer {
        Some(bearer) => request.header("authorization", format!("Bearer {bearer}")),
        None => request,
    };
    request
        .header("host", "127.0.0.1")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18")
        .body(Body::from(message.to_string()))
        .expect("a request")
}

#[tokio::test]
async fn an_mcp_call_asks_the_identify_it_was_given() {
    let shutdown = Shutdown::new();
    let router = router(&shutdown).await;
    let register = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "register", "arguments": {} }
    });
    let (status, _, body) = send(&router, mcp(register)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains(REFUSED), "{body}");
    shutdown.stop();
}

#[tokio::test]
async fn the_web_page_asks_the_identify_it_was_given() {
    let shutdown = Shutdown::new();
    let router = router(&shutdown).await;
    let request = Request::get("/")
        .header("host", "127.0.0.1")
        .body(Body::empty())
        .expect("a request");
    let (status, _, body) = send(&router, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains(REFUSED), "{body}");
    shutdown.stop();
}

/// The program chooses `/mcp-admin`'s admission. None serves nothing; any
/// mode it can choose asks for a credential, so a caller without one never
/// reaches the endpoint, and one with a bearer reaches the `Identify` the
/// program gave.
#[tokio::test]
async fn the_admin_endpoint_admits_as_the_program_chose() {
    let list = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_channels", "arguments": {} }
    });
    let shutdown = Shutdown::new();

    let off = router_with(&shutdown, None).await;
    let (status, _, _) = send(&off, mcp_at("/mcp-admin", list.clone(), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let bearer = router_with(&shutdown, Some(AdminAdmission::Bearer)).await;
    let (status, headers, body) = send(&bearer, mcp_at("/mcp-admin", list.clone(), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(
        headers.get("www-authenticate").map(|v| v.as_bytes()),
        Some(&b"Bearer"[..])
    );
    let (status, _, body) = send(&bearer, mcp_at("/mcp-admin", list, Some("made-up"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains(REFUSED), "{body}");
    shutdown.stop();
}

/// The page alone serves the web page over the application and gate it is
/// given, and nothing else: no MCP endpoint.
#[tokio::test]
async fn the_page_alone_serves_the_page_and_no_endpoint() {
    let clock = Arc::new(FixedClock::new(start_time()));
    let shutdown = Shutdown::new();
    let preview = kkt_api_sim::Preview::new(
        clock.clone(),
        Arc::new(SeqEntropy::new(1)),
        kkt_api_sim::PreviewSettings::DEFAULT,
        &shutdown,
    )
    .expect("the preview's constants are valid");
    let gate = kkt_api_sim::PreviewGate::open();
    let sessions = kkt_web::Sessions::new(
        clock,
        Arc::new(SeqEntropy::new(2)),
        kkt_web::Settings::DEFAULT.session_lifetime,
        gate.access(),
    );
    let router = crate::page(
        Arc::new(preview),
        AllowedHosts::new(&[]).expect("loopback hosts"),
        Arc::new(gate),
        Arc::new(sessions),
        Arc::new(kkt_log::testing::CapturedLog::default()),
        kkt_web::Settings::DEFAULT,
    )
    .router;
    let page = Request::get("/")
        .header("host", "127.0.0.1")
        .body(Body::empty())
        .expect("a request");
    let (status, _, body) = send(&router, page).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("general"), "the simulated channels: {body}");
    let (status, _, _) = send(
        &router,
        mcp(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    shutdown.stop();
}
