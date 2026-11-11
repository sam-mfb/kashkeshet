//! A fake product with two swarms, for tests only: the shared crates
//! assembled the way a product other than kkt would assemble them, with
//! the real server (`kkt_server::listen`) in the test's process.
//!
//! Callers are let in by any bearer token (`Admission::Bearer`), and
//! [`FakeIdentify`] decides who each made-up bearer is: a swarm's agents,
//! its operator on the web page, or its admin at `/mcp-admin`, which also
//! takes any bearer (`AdminAdmission::Bearer`). The tests in `tests/` seed
//! both swarms over HTTP and check that nothing of one reaches the other.
//!
//! The tests start it over an in-memory database; the `kkt-fake-product`
//! program serves it over a database file, to look at by hand. Never part
//! of a release; see `[package.metadata.kkt]` in Cargo.toml.

pub mod client;

use kkt_admission::{AdminAdmission, Admission, Principal};
use kkt_clock::Clock;
use kkt_entropy::Entropy;
use kkt_hosts::AllowedHosts;
use kkt_identity::{Identified, Identify, Identifying, KeyMode};
use kkt_names::AdminName;
use kkt_rules::{
    self as rules, Agent, Audit, Code, Operator, Permission, Permissions, RuleError, Rules, SwarmId,
};
use kkt_rules::{Db, NewServerRunRecord};
use kkt_server::{AppConfig, Parts};
use kkt_shutdown::Shutdown;
use std::net::SocketAddr;
use std::sync::Arc;

/// The first swarm: kkt's own, `sw_main`.
pub fn main() -> SwarmId {
    SwarmId::main()
}

/// The second swarm, which the product makes at startup.
pub fn other() -> SwarmId {
    SwarmId::new("sw_other")
}

/// The made-up bearers [`FakeIdentify`] knows, three for each swarm.
pub struct Bearers;

impl Bearers {
    pub const MAIN_AGENTS: &'static str = "fake-agents-of-main";
    pub const MAIN_OPERATOR: &'static str = "fake-operator-of-main";
    pub const MAIN_ADMIN: &'static str = "fake-admin-of-main";
    pub const OTHER_AGENTS: &'static str = "fake-agents-of-other";
    pub const OTHER_OPERATOR: &'static str = "fake-operator-of-other";
    pub const OTHER_ADMIN: &'static str = "fake-admin-of-other";

    /// The bearer that lets `swarm`'s agents in.
    pub fn agents(swarm: &SwarmId) -> &'static str {
        if *swarm == main() {
            Bearers::MAIN_AGENTS
        } else {
            Bearers::OTHER_AGENTS
        }
    }

    /// The bearer `swarm`'s operator signs in to the web page with.
    pub fn operator(swarm: &SwarmId) -> &'static str {
        if *swarm == main() {
            Bearers::MAIN_OPERATOR
        } else {
            Bearers::OTHER_OPERATOR
        }
    }

    /// The bearer `swarm`'s admin presents at `/mcp-admin`.
    pub fn admin(swarm: &SwarmId) -> &'static str {
        if *swarm == main() {
            Bearers::MAIN_ADMIN
        } else {
            Bearers::OTHER_ADMIN
        }
    }
}

/// The name an admin's changes are announced under.
pub const ADMIN_NAME: &str = "fake";

/// What a made-up bearer is.
enum Role {
    Agents,
    Operator,
    Admin,
}

/// Who each caller is, in the fake product: a made-up bearer names a swarm
/// and a role. Every other caller is refused.
pub struct FakeIdentify;

impl FakeIdentify {
    fn bearer(principal: &Principal) -> rules::Result<(SwarmId, Role)> {
        let refused = || RuleError::new(Code::Unauthenticated, "not a bearer the fake knows");
        let Principal::Bearer(token) = principal else {
            return Err(refused());
        };
        match token.expose() {
            Bearers::MAIN_AGENTS => Ok((main(), Role::Agents)),
            Bearers::MAIN_OPERATOR => Ok((main(), Role::Operator)),
            Bearers::MAIN_ADMIN => Ok((main(), Role::Admin)),
            Bearers::OTHER_AGENTS => Ok((other(), Role::Agents)),
            Bearers::OTHER_OPERATOR => Ok((other(), Role::Operator)),
            Bearers::OTHER_ADMIN => Ok((other(), Role::Admin)),
            _ => Err(refused()),
        }
    }

    fn agents_swarm(principal: &Principal) -> rules::Result<SwarmId> {
        match FakeIdentify::bearer(principal)? {
            (swarm, Role::Agents) => Ok(swarm),
            (_, Role::Operator | Role::Admin) => Err(RuleError::new(
                Code::Unauthenticated,
                "an operator's or admin's bearer does not let agents in",
            )),
        }
    }
}

impl Identify for FakeIdentify {
    fn avatar<'a>(
        &'a self,
        rules: &'a Rules,
        principal: &'a Principal,
        token: Option<&'a str>,
    ) -> Identifying<'a, Identified<Agent>> {
        Box::pin(async move {
            let swarm = FakeIdentify::agents_swarm(principal)?;
            let actor = rules.in_swarm(swarm.clone()).authenticate(token).await?;
            Ok(Identified { swarm, actor })
        })
    }

    fn newcomer<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, SwarmId> {
        Box::pin(async move { FakeIdentify::agents_swarm(principal) })
    }

    fn operator<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, Identified<Operator>> {
        Box::pin(async move {
            match FakeIdentify::bearer(principal)? {
                (swarm, Role::Operator) => Ok(Identified {
                    swarm,
                    actor: Operator::new(
                        Permissions::of(&[
                            Permission::Read,
                            Permission::Change,
                            Permission::IssueRecoveryCodes,
                        ]),
                        Audit::WebPage,
                    ),
                }),
                (swarm, Role::Admin) => {
                    let name = AdminName::new(ADMIN_NAME, kkt_names::MAX_ADMIN_NAME_CHARS)
                        .map_err(|e| RuleError::new(Code::Internal, e.to_string()))?;
                    Ok(Identified {
                        swarm,
                        actor: KeyMode::admin_operator(Some(name)),
                    })
                }
                (_, Role::Agents) => Err(RuleError::new(
                    Code::Unauthenticated,
                    "an agents' bearer does not make an operator",
                )),
            }
        })
    }
}

/// Each part's settings, as the product gives them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// What the rules enforce and fill in.
    pub rules: kkt_rules::Settings,
    /// How large a response may be.
    pub fmt: kkt_format::Settings,
    /// How much the web page shows at once.
    pub web: kkt_web::Settings,
    /// How the MCP endpoints' event streams keep in touch with clients.
    pub mcp: kkt_mcp::Settings,
    /// How long a web page sign-in lasts.
    pub sessions: kkt_admission::Settings,
    /// How many notices the events bus keeps for a slow subscriber.
    pub events: kkt_events::Settings,
}

impl Settings {
    /// Every part's defaults.
    pub const DEFAULT: Settings = Settings {
        rules: kkt_rules::Settings::DEFAULT,
        fmt: kkt_format::Settings::DEFAULT,
        web: kkt_web::Settings::DEFAULT,
        mcp: kkt_mcp::Settings::DEFAULT,
        sessions: kkt_admission::Settings::DEFAULT,
        events: kkt_events::Settings::DEFAULT,
    };
}

/// The fake product, serving on a loopback port until dropped or stopped.
pub struct Product {
    pub address: SocketAddr,
    shutdown: Shutdown,
}

impl Product {
    /// Starts the product with `settings`, over `db`, a new database with
    /// the core's tables, in which it makes the second swarm. Its time comes
    /// from `clock` and its randomness from `entropy`: the system's, or a
    /// test's fixed clock and seeded entropy, so two runs can be compared.
    pub async fn start(
        db: Db,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        settings: Settings,
    ) -> Product {
        let now = kkt_clock::to_text(clock.now());
        let run_id = db
            .write(move |w| {
                let run_id = w.insert_server_run(&NewServerRunRecord {
                    started_at: now.clone(),
                    version: "fake".into(),
                    git_commit: "fake".into(),
                    settings: "fake".into(),
                    instructions_sha256: "fake".into(),
                })?;
                w.insert_swarm(&other(), &now, run_id)?;
                Ok::<_, kkt_rules::StoreError>(run_id)
            })
            .await
            .expect("a server run and the second swarm");
        let parts = Parts::new(
            db,
            clock,
            entropy,
            Arc::new(FakeIdentify),
            &settings.fmt,
            &settings.events,
            run_id,
        );
        let config = AppConfig {
            admission: Admission::Bearer,
            web_admission: Admission::Bearer,
            admin_admission: Some(AdminAdmission::Bearer),
            rules: settings.rules,
            web: settings.web,
            mcp: settings.mcp,
            sessions: settings.sessions,
            allowed_origins: Vec::new(),
            allowed_hosts: AllowedHosts::new(&[]).expect("loopback hosts"),
            instructions: None,
        };
        Product::serve(parts, config, Shutdown::new()).await
    }

    /// Serves the product assembled from `parts` and `config` on a loopback
    /// port until `shutdown` stops.
    async fn serve(parts: Parts, config: AppConfig, shutdown: Shutdown) -> Product {
        let app = kkt_server::build(parts, config, shutdown.child());
        let listening = kkt_server::listen(app, SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("a loopback port");
        let address = listening.address().expect("the bound address");
        let stop = shutdown.clone();
        tokio::spawn(async move {
            listening
                .serve_until(async move { stop.stopped().await }, |e| {
                    eprintln!("kkt-fake-product: could not set TCP_NODELAY on a connection: {e}")
                })
                .await
                .expect("serving")
        });
        Product { address, shutdown }
    }

    /// Stops the product.
    pub fn stop(&self) {
        self.shutdown.stop();
    }

    /// Waits until the product is stopped.
    pub async fn stopped(&self) {
        self.shutdown.stopped().await;
    }

    /// The product's stop signal, to stop it from elsewhere.
    pub fn shutdown(&self) -> Shutdown {
        self.shutdown.clone()
    }

    /// `path` on the product, as a URL.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
}

impl Drop for Product {
    fn drop(&mut self) {
        self.shutdown.stop();
    }
}
