//! Tests that every call to either endpoint acts in the swarm
//! [`Identify`] names, as a product with several swarms would need, and that
//! [`KeyMode`] acts in the one swarm.

use crate::mcp_transport::Transport;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use kkt_admission::{Principal, ServerKey};
use kkt_clock::Time;
use kkt_clock::testing::FixedClock;
use kkt_config::Limits;
use kkt_events::Events;
use kkt_format::Formatter;
use kkt_hosts::AllowedHosts;
use kkt_identity::{Identified, Identify, Identifying, KeyMode};
use kkt_names::testing::SeqEntropy;
use kkt_rules::{Agent, Audit, Operator, Rules, SwarmId};
use kkt_rules::{Code, Deps, OperatorWording, RuleError};
use kkt_shutdown::Shutdown;
use kkt_store::Db;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

/// Sends every call to one swarm that is not the endpoint's own, as a
/// product with several swarms would. Newcomers go there too when
/// `newcomers`; otherwise they are refused.
struct Elsewhere {
    swarm: SwarmId,
    newcomers: bool,
}

impl Identify for Elsewhere {
    fn avatar<'a>(
        &'a self,
        rules: &'a Rules,
        _principal: &'a Principal,
        token: Option<&'a str>,
    ) -> Identifying<'a, Identified<Agent>> {
        Box::pin(async move {
            let actor = rules
                .in_swarm(self.swarm.clone())
                .authenticate(token)
                .await?;
            Ok(Identified {
                swarm: self.swarm.clone(),
                actor,
            })
        })
    }

    fn newcomer<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, SwarmId> {
        Box::pin(async move {
            if self.newcomers {
                return Ok(self.swarm.clone());
            }
            Err(RuleError::new(
                Code::Unauthenticated,
                "avatars are made elsewhere here",
            ))
        })
    }

    fn operator<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, Identified<Operator>> {
        Box::pin(async move {
            Ok(Identified {
                swarm: self.swarm.clone(),
                actor: KeyMode::admin_operator(None),
            })
        })
    }
}

struct Plain;

impl OperatorWording for Plain {
    fn operator(&self, _audit: &Audit) -> String {
        "The operator".into()
    }
}

/// Rules over a new database with two swarms, acting in `sw_main`.
async fn rules() -> Rules {
    let clock = Arc::new(FixedClock::new(
        Time::from_parts(2026, 11, 9, 0, 0, 0).unwrap(),
    ));
    let db = {
        let place = kkt_db_sqlite::MemoryPlace::new().expect("an in-memory database");
        Db::new(
            place
                .open(&[kkt_store::schema()])
                .expect("the core's tables"),
        )
    };
    let run_id = db
        .write(|w| {
            let run_id = w.insert_server_run(&kkt_store::NewServerRunRecord {
                started_at: "2026-11-09T00:00:00Z".into(),
                version: "test".into(),
                git_commit: "test".into(),
                settings: "test".into(),
                instructions_sha256: "test".into(),
            })?;
            w.insert_swarm(&SwarmId::new("sw_other"), "2026-11-09T00:00:00Z", run_id)?;
            Ok::<_, kkt_store::StoreError>(run_id)
        })
        .await
        .expect("a server run and a second swarm");
    Rules::new(Deps {
        db,
        swarm: SwarmId::main(),
        events: Events::new(),
        clock,
        entropy: Arc::new(SeqEntropy::new(1)),
        run_id,
        limits: Limits::DEFAULT,
        wording: Arc::new(Plain),
    })
}

/// Both endpoints over `rules`, identifying calls with `identify`, with
/// `principal` left on every request as admission would.
fn endpoints(rules: &Rules, identify: Arc<dyn Identify>, principal: Option<Principal>) -> Router {
    let fmt = Formatter::new(100_000, Arc::new(FixedClock::new(Time::UNIX_EPOCH)), "test");
    let transport = Transport::new(Vec::new(), &AllowedHosts::any(), &Shutdown::new());
    let router = crate::endpoint(
        rules.clone(),
        identify.clone(),
        fmt.clone(),
        "".into(),
        "test",
        &transport,
    )
    .merge(crate::mcp_admin::endpoint(
        rules.clone(),
        identify,
        fmt,
        "test",
        &transport,
    ));
    match principal {
        Some(principal) => router.layer(axum::Extension(principal)),
        None => router,
    }
}

/// Calls `tool` at `path` with `arguments` in JSON format and returns
/// what it answered, as JSON, or an error's text.
async fn call(router: &Router, path: &str, tool: &str, arguments: Value) -> Value {
    let mut arguments = arguments;
    arguments["format"] = json!("json");
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let request = Request::post(path)
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Body::from(body.to_string()))
        .expect("a request");
    let response = router.clone().oneshot(request).await.expect("a response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    let text = String::from_utf8(bytes.to_vec()).expect("text");
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .unwrap_or_else(|| panic!("no data in {text}"));
    let reply: Value = serde_json::from_str(data).expect("JSON-RPC");
    let said = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text in {reply}"));
    serde_json::from_str(said).unwrap_or_else(|_| json!(said))
}

#[tokio::test]
async fn key_mode_acts_in_the_one_swarm() {
    let rules = rules().await;
    let router = endpoints(
        &rules,
        Arc::new(KeyMode::new(None)),
        Some(Principal::Anyone),
    );
    let made = call(&router, "/mcp", "register", json!({})).await;
    let token = made["token"].as_str().expect("a token");
    assert!(rules.authenticate(Some(token)).await.is_ok());
    let me = call(&router, "/mcp", "whoami", json!({ "token": token })).await;
    assert_eq!(me["handle"], made["handle"]);
}

#[tokio::test]
async fn key_mode_lets_the_admin_key_act_at_the_admin_endpoint_and_not_the_access_key() {
    let rules = rules().await;
    let as_admin = endpoints(
        &rules,
        Arc::new(KeyMode::new(None)),
        Some(Principal::KeyHolder(ServerKey::Admin)),
    );
    let listed = call(&as_admin, "/mcp-admin", "list_channels", json!({})).await;
    assert!(listed["channels"].is_array(), "{listed}");

    let as_agent = endpoints(
        &rules,
        Arc::new(KeyMode::new(None)),
        Some(Principal::KeyHolder(ServerKey::Access)),
    );
    let refused = call(&as_agent, "/mcp-admin", "list_channels", json!({})).await;
    assert_eq!(refused["error"]["code"], "unauthenticated", "{refused}");
}

#[tokio::test]
async fn every_call_acts_in_the_swarm_identify_names() {
    let rules = rules().await;
    let other = SwarmId::new("sw_other");
    let theirs = rules.in_swarm(other.clone());
    let reg = theirs.register().await.expect("register");
    let me = theirs.authenticate(Some(&reg.token)).await.expect("auth");
    let router = endpoints(
        &rules,
        Arc::new(Elsewhere {
            swarm: other,
            newcomers: false,
        }),
        Some(Principal::Anyone),
    );

    // The agent's token is found in the other swarm, and what it makes
    // is made there.
    let created = call(
        &router,
        "/mcp",
        "create_channel",
        json!({ "token": reg.token, "name": "away", "open": true }),
    )
    .await;
    let id = created["id"].as_str().expect("a channel ID").to_string();
    let channels = theirs
        .list_channels(
            &me,
            kkt_rules::ChannelQuery {
                query: None,
                member_only: false,
                with_agent: None,
                after_id: None,
                limit: 10,
                cursor: None,
            },
        )
        .await
        .expect("list");
    assert_eq!(channels.items.len(), 1);
    assert_eq!(channels.items[0].id, id);

    // The endpoint's own swarm has none of it.
    let home = rules.register().await.expect("register");
    let home = rules.authenticate(Some(&home.token)).await.expect("auth");
    assert_eq!(
        rules
            .join_channel(&home, id.clone())
            .await
            .expect_err("not there")
            .code,
        Code::NotFound
    );

    // `register` and `reclaim` say what `identify` says.
    let refused = call(&router, "/mcp", "register", json!({})).await;
    assert_eq!(
        refused,
        json!({ "error": { "code": "unauthenticated", "message": "avatars are made elsewhere here" } })
    );

    // The admin endpoint lists the other swarm's channel, not home's.
    rules
        .create_channel(
            &home,
            kkt_rules::NewChannel {
                name: "home".into(),
                open: true,
                purpose: None,
                members: Vec::new(),
            },
        )
        .await
        .expect("create");
    let listed = call(&router, "/mcp-admin", "list_channels", json!({})).await;
    let names: Vec<_> = listed["channels"]
        .as_array()
        .unwrap_or_else(|| panic!("no channels in {listed}"))
        .iter()
        .map(|c| c["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["away"]);
}

#[tokio::test]
async fn a_call_admission_did_not_let_in_is_refused() {
    let rules = rules().await;
    let router = endpoints(&rules, Arc::new(KeyMode::new(None)), None);
    let refused = call(&router, "/mcp", "register", json!({})).await;
    assert_eq!(refused["error"]["code"], "internal");
    let refused = call(&router, "/mcp-admin", "list_channels", json!({})).await;
    assert_eq!(refused["error"]["code"], "internal");
}

#[tokio::test]
async fn register_and_reclaim_act_in_the_swarm_identify_names() {
    let rules = rules().await;
    let other = SwarmId::new("sw_other");
    let router = endpoints(
        &rules,
        Arc::new(Elsewhere {
            swarm: other.clone(),
            newcomers: true,
        }),
        Some(Principal::Anyone),
    );
    let theirs = rules.in_swarm(other);

    let made = call(&router, "/mcp", "register", json!({})).await;
    let token = made["token"].as_str().expect("a token");
    assert!(theirs.authenticate(Some(token)).await.is_ok());
    assert_eq!(
        rules
            .authenticate(Some(token))
            .await
            .expect_err("not home")
            .code,
        Code::Unauthenticated
    );

    let back = call(
        &router,
        "/mcp",
        "reclaim",
        json!({ "handle": made["handle"], "recovery_code": made["recovery_code"] }),
    )
    .await;
    let token = back["token"]
        .as_str()
        .unwrap_or_else(|| panic!("no token in {back}"));
    assert!(theirs.authenticate(Some(token)).await.is_ok());
    assert_eq!(
        rules
            .authenticate(Some(token))
            .await
            .expect_err("not home")
            .code,
        Code::Unauthenticated
    );
}
