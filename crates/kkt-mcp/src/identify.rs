//! Who a call comes from: the [`Identify`] contract both endpoints ask
//! before any tool acts. A request becomes the swarm it acts in and its
//! actor, or a refusal. Admission has already let the caller in, and hands
//! on the [`Principal`] it was admitted as; `Identify` takes it from there.
//!
//! [`KeyMode`] is the one implementation in kashkeshet: an agent call is
//! the avatar its token belongs to, in the one swarm; `register` and
//! `reclaim` are always allowed there; an admin call is the operator the
//! admin key makes.

use kkt_admission::Principal;
use kkt_names::AdminName;
use kkt_rules::{self as rules, Agent, Audit, Operator, Permission, Permissions, Rules, SwarmId};
use std::future::Future;
use std::pin::Pin;

/// A future an [`Identify`] implementation returns.
pub type Identifying<'a, T> = Pin<Box<dyn Future<Output = rules::Result<T>> + Send + 'a>>;

/// What a call was identified as: the swarm it acts in, and who acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identified<A> {
    pub swarm: SwarmId,
    pub actor: A,
}

/// How each call to an endpoint becomes (swarm, actor), or a refusal. A
/// refusal is a [`rules::RuleError`], given to the caller as the tool's
/// error.
pub trait Identify: Send + Sync {
    /// An agent tool call carrying `token`, from a caller admitted as
    /// `principal`: the avatar it acts as, and in which swarm. `rules` are
    /// the endpoint's, to look the token up with, in whichever swarm the
    /// implementation chooses ([`Rules::in_swarm`]). The swarm returned
    /// must be the one the avatar was found in: the call acts there.
    fn avatar<'a>(
        &'a self,
        rules: &'a Rules,
        principal: &'a Principal,
        token: Option<&'a str>,
    ) -> Identifying<'a, Identified<Agent>>;

    /// `register` or `reclaim`, from a caller admitted as `principal`: the
    /// swarm the avatar is registered or reclaimed in, or why neither may
    /// be done.
    fn newcomer<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, SwarmId>;

    /// An admin tool call, from a caller admitted as `principal`: the
    /// operator it acts as, and in which swarm.
    fn operator<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, Identified<Operator>>;
}

/// Key mode: one swarm, `sw_main`; an agent is who its token says; anyone
/// admitted may register or reclaim; an admin call is the operator the
/// admin key makes ([`KeyMode::admin_operator`]). The principal adds
/// nothing: in key mode admission lets every caller in as the same one.
pub struct KeyMode {
    swarm: SwarmId,
    admin_name: Option<AdminName>,
}

impl KeyMode {
    /// Key mode for a server whose admin key has `admin_name`, if it has an
    /// admin key with a name.
    pub fn new(admin_name: Option<AdminName>) -> KeyMode {
        KeyMode {
            swarm: SwarmId::main(),
            admin_name,
        }
    }

    /// The operator an admin key makes: it may read, and change too when
    /// the key has a name, `name`; its changes are recorded and announced
    /// under that name. It never issues recovery codes. An admin key with
    /// no name never has [`Permission::Change`], so it is never named in a
    /// notice.
    pub fn admin_operator(name: Option<AdminName>) -> Operator {
        let permissions = match name {
            Some(_) => Permissions::of(&[Permission::Read, Permission::Change]),
            None => Permissions::of(&[Permission::Read]),
        };
        Operator::new(permissions, Audit::AdminEndpoint { key_name: name })
    }
}

impl Identify for KeyMode {
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
        Box::pin(async move { Ok(self.swarm.clone()) })
    }

    fn operator<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, Identified<Operator>> {
        Box::pin(async move {
            Ok(Identified {
                swarm: self.swarm.clone(),
                actor: KeyMode::admin_operator(self.admin_name.clone()),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_transport::Transport;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use kkt_clock::Time;
    use kkt_clock::testing::FixedClock;
    use kkt_config::Limits;
    use kkt_events::Events;
    use kkt_format::Formatter;
    use kkt_hosts::AllowedHosts;
    use kkt_names::testing::SeqEntropy;
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

        fn operator<'a>(
            &'a self,
            _principal: &'a Principal,
        ) -> Identifying<'a, Identified<Operator>> {
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
        let db = Db::open_in_memory().expect("db");
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
    fn endpoints(
        rules: &Rules,
        identify: Arc<dyn Identify>,
        principal: Option<Principal>,
    ) -> Router {
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
}
