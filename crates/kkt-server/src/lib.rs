//! Puts the parts together: the rules, the MCP endpoint at `/mcp`, the admin
//! endpoint at `/mcp-admin` when there is an admin key, and the web page at
//! `/`. Also stores the record of each server run in the database, and, with
//! the `real` feature, runs the server (`listen`, then
//! `Listening::serve_until`).
//!
//! kkt-web and kkt-mcp hand their axum `Router`s to this crate, which mounts
//! them and runs them; no axum type leaves it.

#[cfg(any(test, feature = "real"))]
mod running;
mod web_real;

#[cfg(any(test, feature = "real"))]
pub use crate::running::{Listening, listen};
pub use crate::web_real::AdmissionGate;

use crate::web_real::RulesSource;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use kkt_admission::{Admission, Credentials, Decision, Sessions, SharedKey, WebAdmission};
use kkt_clock::{self as clock, Clock, Time};
use kkt_config::Limits;
use kkt_events::Events;
use kkt_format::Formatter;
use kkt_hosts::AllowedHosts;
use kkt_identity::{Identify, KeyMode};
use kkt_mcp as mcp;
use kkt_mcp::mcp_admin;
use kkt_mcp::mcp_transport::Transport;
use kkt_names::{AdminName, Entropy};
use kkt_provenance::{Build, ServerRun};
use kkt_rules::{Deps, Rules};
use kkt_shutdown::Shutdown;
use kkt_store::{Db, NewServerRunRecord, StoreResult, SwarmId};
use kkt_web as web;
use std::sync::Arc;

pub struct AppConfig {
    /// Who may call `/mcp` at all. The web page is not covered.
    pub admission: Admission,
    /// Who may sign in to the operator web page.
    pub web_admission: Admission,
    /// The admin key, the only key `/mcp-admin` takes. `None` turns
    /// `/mcp-admin` off: it is not served at all.
    pub admin_key: Option<SharedKey>,
    /// The name tied to the admin key, under which the admin agent's changes
    /// are made and announced. `None`: the admin key may only read.
    pub admin_name: Option<AdminName>,
    pub limits: Limits,
    /// Browser origins allowed to call `/mcp`. Requests with any other
    /// `Origin` header are refused; requests without one (agents) pass.
    pub allowed_origins: Vec<String>,
    /// `Host` names `/mcp` and the web page answer to. Requests naming any
    /// other host are refused.
    pub allowed_hosts: AllowedHosts,
    /// The text of the server's instructions file, if it has one, even when
    /// it is empty. `None`: agents get kkt-mcp's built-in instructions. See
    /// [`AppConfig::agent_instructions`].
    pub instructions: Option<String>,
}

impl AppConfig {
    /// The instructions every agent that connects is sent: those configured,
    /// or the built-in ones. The record of the server run hashes this text.
    pub fn agent_instructions(&self) -> &str {
        mcp::agent_instructions(self.instructions.as_deref())
    }
}

/// The parts the server is built from. Callers outside the crate supply the
/// database, clock, entropy and run through [`Parts::new`], which makes the
/// rest; unit tests inside the crate can replace those too.
pub struct Parts {
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub entropy: Arc<dyn Entropy>,
    pub(crate) events: Events,
    pub(crate) fmt: Formatter,
    /// This server run, from [`record_run`].
    pub run_id: i64,
}

impl Parts {
    /// The usual parts around `db`, `clock` and `entropy`, sized by `limits`,
    /// for the server run `run_id`.
    pub fn new(
        db: Db,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        limits: &Limits,
        run_id: i64,
    ) -> Parts {
        Parts {
            run_id,
            db,
            fmt: Formatter::new(
                limits.max_response_bytes,
                clock.clone(),
                Build::CURRENT.version,
            ),
            clock,
            entropy,
            events: Events::new(),
        }
    }
}

/// Stores `run` in `db` as a new server run that started at `started_at`,
/// and returns its ID. New agents and messages are stamped with that ID.
pub async fn record_run(db: &Db, run: &ServerRun, started_at: Time) -> StoreResult<i64> {
    let record = NewServerRunRecord {
        started_at: clock::to_text(started_at),
        version: run.version.clone(),
        git_commit: run.commit.clone(),
        settings: run.settings.clone(),
        instructions_sha256: run.instructions_sha256.clone(),
    };
    db.write(move |w| w.insert_server_run(&record)).await
}

/// The server, assembled by [`build`]. With the `real` feature, `listen`
/// binds it to an address and runs it.
pub struct App {
    // Read only by `running`, which only `real` and tests build.
    #[cfg_attr(not(any(test, feature = "real")), allow(dead_code))]
    pub(crate) router: Router,
}

/// The server assembled from `parts` and `config`, to run with `listen`.
/// Its MCP sessions and live page updates end when `shutdown` stops.
pub fn build(parts: Parts, config: AppConfig, shutdown: Shutdown) -> App {
    App {
        router: router(parts, config, shutdown),
    }
}

fn router(parts: Parts, config: AppConfig, shutdown: Shutdown) -> Router {
    let fmt = parts.fmt;
    let instructions: Arc<str> = config.agent_instructions().into();
    let admission = Arc::new(config.admission);
    let sessions = Sessions::new(
        parts.clock.clone(),
        parts.entropy.clone(),
        config.limits.web_session_lifetime,
    );
    let web_admission = Arc::new(AdmissionGate::new(WebAdmission::new(
        Arc::new(config.web_admission),
        sessions,
    )));
    let rules = Rules::new(Deps {
        db: parts.db,
        // Key mode: the one swarm, which every avatar and channel is in.
        swarm: SwarmId::main(),
        events: parts.events.until_stopped(shutdown.clone()),
        clock: parts.clock,
        entropy: parts.entropy,
        run_id: parts.run_id,
        limits: config.limits,
        wording: Arc::new(kkt_format::NoticeWording),
    });

    // One transport for both endpoints: their sessions end when `shutdown`
    // stops.
    let transport = Transport::new(config.allowed_origins, &config.allowed_hosts, &shutdown);
    // Key mode: each call is the avatar its token names, or the operator
    // the admin key makes, in the one swarm.
    let identify: Arc<dyn Identify> = Arc::new(KeyMode::new(config.admin_name));
    let mcp = mcp::endpoint(
        rules.clone(),
        identify.clone(),
        fmt.clone(),
        instructions,
        Build::CURRENT.version,
        &transport,
    )
    .route_layer(middleware::from_fn_with_state(admission, admit));
    let admin = config.admin_key.map(|key| {
        admin_only(
            key,
            mcp_admin::endpoint(
                rules.clone(),
                identify,
                fmt,
                Build::CURRENT.version,
                &transport,
            ),
        )
    });
    let router = web::router(
        Arc::new(RulesSource::new(rules)),
        config.allowed_hosts.clone(),
        web_admission,
    )
    .merge(mcp);
    match admin {
        Some(admin) => router.merge(admin),
        None => router,
    }
}

/// `admin`, the `/mcp-admin` endpoint, served to callers with the admin key
/// only.
fn admin_only(key: SharedKey, admin: Router) -> Router {
    admin.route_layer(middleware::from_fn_with_state(
        Arc::new(Admission::SharedKey(key)),
        admit,
    ))
}

/// Lets a request through to `/mcp` only if admission lets its caller in,
/// keeping the [`Principal`](kkt_admission::Principal) it was admitted as in
/// the request's extensions for the endpoint to read. Refusals get `401`
/// with `WWW-Authenticate: Bearer` and the reason.
async fn admit(
    State(admission): State<Arc<Admission>>,
    mut request: Request,
    next: Next,
) -> Response {
    let credentials =
        Credentials::from_authorization(request.headers().get(AUTHORIZATION).map(|v| v.as_bytes()));
    match admission.decide(&credentials) {
        Decision::Admitted(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Decision::Refused(refusal) => (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, "Bearer")],
            refusal.to_string(),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use kkt_admission::{Principal, ServerKey};
    use tower::ServiceExt;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    /// A route behind `admit` that answers with the principal it finds.
    fn behind(admission: Admission) -> Router {
        Router::new()
            .route(
                "/",
                get(|request: Request| async move {
                    format!("{:?}", request.extensions().get::<Principal>())
                }),
            )
            .route_layer(middleware::from_fn_with_state(Arc::new(admission), admit))
    }

    async fn call(router: Router, bearer: Option<&str>) -> (StatusCode, String) {
        let mut request = Request::builder().uri("/");
        if let Some(key) = bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {key}"));
        }
        let response = router
            .oneshot(request.body(Body::empty()).expect("a request"))
            .await
            .expect("a response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body");
        (status, String::from_utf8(body.to_vec()).expect("text"))
    }

    #[tokio::test]
    async fn an_admitted_caller_reaches_the_endpoint_with_its_principal() {
        let keyed =
            || Admission::SharedKey(SharedKey::new(ServerKey::Access, KEY, "test").expect("key"));
        assert_eq!(
            call(behind(keyed()), Some(KEY)).await,
            (StatusCode::OK, "Some(KeyHolder(Access))".to_string())
        );
        assert_eq!(
            call(behind(Admission::Open), None).await,
            (StatusCode::OK, "Some(Anyone)".to_string())
        );
        let (status, body) = call(behind(keyed()), Some("wrong")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, "the access key is wrong");
    }
}
