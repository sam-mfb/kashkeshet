//! Puts the parts together: the rules, the MCP endpoint at `/mcp`, the admin
//! endpoint at `/mcp-admin` when there is an admin key, and the web page at
//! `/`. Also stores the record of each server run in the database, and runs
//! the server ([`listen`], then [`Listening::serve_until`]).
//!
//! kkt-web and kkt-mcp hand their axum `Router`s to this crate, which mounts
//! them and runs them; no axum type leaves it.

mod running;
mod web_real;

pub use crate::running::{App, Listening, listen};
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
use kkt_mcp as mcp;
use kkt_mcp::mcp_admin;
use kkt_mcp::mcp_transport::Transport;
use kkt_names::{AdminName, Entropy};
use kkt_provenance::{Build, ServerRun};
use kkt_rules::{Deps, Rules};
use kkt_shutdown::Shutdown;
use kkt_store::{Db, NewServerRunRecord, StoreResult};
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
    /// Sent to every agent that connects. See [`kkt_mcp::DEFAULT_INSTRUCTIONS`].
    pub instructions: String,
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

/// The server assembled from `parts` and `config`, to run with [`listen`].
/// Its MCP sessions and live page updates end when `shutdown` stops.
pub fn build(parts: Parts, config: AppConfig, shutdown: Shutdown) -> App {
    App {
        router: router(parts, config, shutdown),
    }
}

fn router(parts: Parts, config: AppConfig, shutdown: Shutdown) -> Router {
    let fmt = parts.fmt;
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
        events: parts.events.until_stopped(shutdown.clone()),
        clock: parts.clock,
        entropy: parts.entropy,
        run_id: parts.run_id,
        limits: config.limits,
    });

    // One transport for both endpoints: their sessions end when `shutdown`
    // stops.
    let transport = Transport::new(config.allowed_origins, &config.allowed_hosts, &shutdown);
    let mcp = mcp::endpoint(
        rules.clone(),
        fmt.clone(),
        config.instructions.into(),
        Build::CURRENT.version,
        &transport,
    )
    .route_layer(middleware::from_fn_with_state(admission, admit));
    let admin = config.admin_key.map(|key| {
        admin_only(
            key,
            mcp_admin::endpoint(
                rules.clone(),
                fmt,
                config.admin_name,
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

/// Lets a request through to `/mcp` only if admission lets its caller in.
/// Refusals get `401` with `WWW-Authenticate: Bearer` and the reason.
async fn admit(State(admission): State<Arc<Admission>>, request: Request, next: Next) -> Response {
    let credentials =
        Credentials::from_authorization(request.headers().get(AUTHORIZATION).map(|v| v.as_bytes()));
    match admission.decide(&credentials) {
        Decision::Admitted(_) => next.run(request).await,
        Decision::Refused(refusal) => (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, "Bearer")],
            refusal.to_string(),
        )
            .into_response(),
    }
}
