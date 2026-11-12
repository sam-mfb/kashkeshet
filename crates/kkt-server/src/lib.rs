//! Puts the parts together: the application API, the MCP endpoint at `/mcp`, the admin
//! endpoint at `/mcp-admin` when there is an admin key, and the web page at
//! `/`; or the web page alone ([`page`]). The program gives it the admin API
//! it built for this server run.
//! kkt-server-listen binds and runs the assembled server.
//!
//! kkt-web and kkt-mcp hand their axum `Router`s to this crate, which mounts
//! them. Its App hands the assembled router to kkt-server-listen.
//!
//! It decides nothing about who a caller is: the program passes in
//! [`Identify`] through [`Parts`], and the admission mode (open or keys)
//! through [`AppConfig`]. The web page reads and changes the rules through
//! the kkt-api crates' contracts; the gate that adapts admission and
//! identity to kkt-web's contract ([`AdmissionGate`]) is public, for a
//! product that assembles its own web page.

mod settings;
mod web_real;
pub use settings::{CheckedSettings, ServerSettings, SettingError};

pub use crate::web_real::AdmissionGate;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use kkt_admission::{AdminAdmission, Admission, Credentials, Decision, WebAdmission};
use kkt_api_admin::AdminApi;
use kkt_api_operator::OperatorApi;
use kkt_clock::Clock;
use kkt_entropy::Entropy;
use kkt_hosts::AllowedHosts;
use kkt_identity::Identify;
use kkt_log::Log;
use kkt_mcp as mcp;
use kkt_mcp::mcp_admin;
use kkt_mcp::mcp_transport::Transport;
use kkt_mcp_formatter::Formatter;
use kkt_provenance::Build;
use kkt_shutdown::Shutdown;
use kkt_web as web;
use std::sync::Arc;

pub struct AppConfig {
    /// Who may call `/mcp` at all. The web page is not covered.
    pub admission: Admission,
    /// Who may sign in to the operator web page.
    pub web_admission: Admission,
    /// Who may call `/mcp-admin`: kkt's admin key, or another mode that asks
    /// for a credential (there is no open one). `None` turns `/mcp-admin`
    /// off: it is not served at all.
    pub admin_admission: Option<AdminAdmission>,
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
/// admin API it built for this server run, and the
/// clock, entropy, log and identity decision through [`Parts::new`], which
/// makes the rest; unit tests inside the crate can replace those too.
pub struct Parts {
    /// The rules every endpoint and the web page act through, each in the
    /// swarm its caller is in. The program built them
    /// with an events bus that stops when the server does.
    pub rules: Arc<dyn AdminApi>,
    /// The rules' clock and entropy: the web page's sessions are timed and
    /// drawn as the rules are.
    pub clock: Arc<dyn Clock>,
    pub entropy: Arc<dyn Entropy>,
    /// Who each caller is: asked by both MCP endpoints for every call and by
    /// the web page's gate for every request.
    pub identify: Arc<dyn Identify>,
    /// Diagnostic events from both MCP endpoints and the web page.
    pub log: Arc<dyn Log>,
    pub(crate) fmt: Formatter,
    settings: ServerSettings,
    hosts: AllowedHosts,
}

impl Parts {
    /// The usual parts around `rules`, `clock`, `entropy`, `identify` and
    /// `log`, with the checked snapshot returned by `ServerSettings::check`.
    /// `clock` and `entropy` are the
    /// ones the rules were built with.
    pub fn new(
        rules: Arc<dyn AdminApi>,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        identify: Arc<dyn Identify>,
        log: Arc<dyn Log>,
        checked: CheckedSettings,
    ) -> Parts {
        let (settings, hosts) = checked.into_parts();
        Parts {
            rules,
            identify,
            log,
            fmt: Formatter::new(
                settings.formatter.max_response_bytes,
                clock.clone(),
                Build::CURRENT.version,
            ),
            clock,
            entropy,
            settings,
            hosts,
        }
    }
}

/// The server, assembled by [`build`], for kkt-server-listen to bind and run.
pub struct App {
    pub(crate) router: Router,
}

impl App {
    /// The assembled HTTP router, consumed by kkt-server-listen when binding.
    pub fn into_router(self) -> Router {
        self.router
    }
}

/// The server assembled from `parts` and `config`, to run with kkt-server-listen.
/// Its MCP sessions and live page updates end when `shutdown` stops.
pub fn build(parts: Parts, config: AppConfig, shutdown: Shutdown) -> App {
    App {
        router: router(parts, config, shutdown),
    }
}

/// The web page alone, over `api` in each viewer's swarm and behind `gate`,
/// with no MCP endpoints, for a program that serves only the page, such as
/// the preview over simulated data. kkt-server-listen binds and runs it as
/// it does a whole server.
pub fn page<S: web::Held>(
    api: Arc<dyn OperatorApi>,
    hosts: AllowedHosts,
    gate: Arc<dyn web::Gate<SignedIn = S>>,
    sessions: Arc<web::Sessions<S>>,
    log: Arc<dyn Log>,
    settings: web::Settings,
) -> App {
    App {
        router: web::router(api, hosts, gate, sessions, log, settings),
    }
}

fn router(parts: Parts, config: AppConfig, shutdown: Shutdown) -> Router {
    let fmt = parts.fmt;
    let settings = parts.settings;
    let hosts = parts.hosts;
    let instructions: Arc<str> = config.agent_instructions().into();
    let admission = Arc::new(config.admission);
    // The web page's sessions are timed and drawn as the rules are.
    let (web_clock, web_entropy) = (parts.clock.clone(), parts.entropy.clone());
    let identify = parts.identify;
    let log = parts.log;
    // The rules' own swarm decides nothing here: each MCP call and each web
    // viewer acts in the swarm `identify` names, through the API contract.
    let rules = parts.rules;

    // One transport for both endpoints: their sessions end when `shutdown`
    // stops.
    let transport = Transport::new(settings.allowed_origins, &hosts, settings.mcp, &shutdown);
    // The web page asks `identify` too: who a viewer is, and what they may
    // do, is decided there alone.
    let web_admission = WebAdmission::new(Arc::new(config.web_admission));
    // The page keeps its own sessions, for as long as its settings say.
    let web_sessions = Arc::new(web::Sessions::new(
        web_clock,
        web_entropy,
        settings.web.session_lifetime,
        match web_admission.sign_in_required() {
            true => web::Access::SignIn,
            false => web::Access::Open,
        },
    ));
    let web_gate = Arc::new(AdmissionGate::new(web_admission, identify.clone()));
    let mcp = mcp::endpoint(
        rules.clone(),
        identify.clone(),
        log.clone(),
        fmt.clone(),
        instructions,
        Build::CURRENT.version,
        &transport,
    )
    .route_layer(middleware::from_fn_with_state(admission, admit));
    let admin = config.admin_admission.map(|admission| {
        admin_only(
            admission,
            mcp_admin::endpoint(
                rules.clone(),
                identify.clone(),
                log.clone(),
                fmt,
                Build::CURRENT.version,
                &transport,
            ),
        )
    });
    let router = web::router(
        rules,
        hosts.clone(),
        web_gate,
        web_sessions,
        log,
        settings.web,
    )
    .merge(mcp);
    match admin {
        Some(admin) => router.merge(admin),
        None => router,
    }
}

/// `admin`, the `/mcp-admin` endpoint, served only to callers `admission`
/// lets in.
fn admin_only(admission: AdminAdmission, admin: Router) -> Router {
    admin.route_layer(middleware::from_fn_with_state(
        Arc::new(admission.into_admission()),
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
mod assembly_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use kkt_admission::{Principal, ServerKey, SharedKey};
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

#[cfg(test)]
mod record_tests;
