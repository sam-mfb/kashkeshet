//! Who a caller is: the [`Identify`] contract, the one place that decides
//! who someone is and what they may do, for the MCP endpoints and the web
//! page alike. Admission has already let the caller in, and hands on the
//! [`Principal`] it was admitted as; `Identify` takes it from there, and a
//! caller becomes the swarm it acts in and its actor, or a refusal.
//!
//! [`KeyMode`] is the one implementation in kashkeshet: an agent call is
//! the avatar its token belongs to, in the one swarm; `register` and
//! `reclaim` are always allowed there; the operator is decided by which of
//! the server's keys admitted the caller ([`KeyMode::operator`]).
//!
//! This crate knows nothing of HTTP: each surface reads the principal off
//! its own requests and passes it in.

use kkt_admission::{Principal, ServerKey};
use kkt_names::AdminName;
use kkt_rules::{
    self as rules, Agent, Audit, Code, Operator, Permission, Permissions, RuleError, Rules, SwarmId,
};
use std::future::Future;
use std::pin::Pin;

/// A future an [`Identify`] implementation returns.
pub type Identifying<'a, T> = Pin<Box<dyn Future<Output = rules::Result<T>> + Send + 'a>>;

/// What a caller was identified as: the swarm it acts in, and who acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identified<A> {
    pub swarm: SwarmId,
    pub actor: A,
}

/// How each caller becomes (swarm, actor), or a refusal. A refusal is a
/// [`rules::RuleError`]; each surface gives it to the caller in its own way.
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

    /// The operator a caller admitted as `principal` acts as, and in which
    /// swarm, or why it is not an operator. Asked by the admin endpoint for
    /// each call, and by the web page for each request.
    fn operator<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, Identified<Operator>>;
}

/// Key mode: one swarm, `sw_main`; an agent is who its token says; anyone
/// admitted may register or reclaim; the operator is decided by the key
/// that admitted the caller ([`KeyMode::operator`]).
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

    /// The operator a caller admitted as `principal` is, in key mode:
    ///
    /// - the admin key: [`KeyMode::admin_operator`] under the admin key's
    ///   name;
    /// - the web key, or no key at all (a server with none set): the human
    ///   on the web page, who may read, change and issue recovery codes,
    ///   audited as the web page. The admin endpoint is served only with an
    ///   admin key, so a caller with no key never reaches it;
    /// - the access key: no operator. It lets agents in, nothing more.
    fn operator_for(&self, principal: &Principal) -> rules::Result<Operator> {
        match principal {
            Principal::KeyHolder(ServerKey::Admin) => {
                Ok(KeyMode::admin_operator(self.admin_name.clone()))
            }
            Principal::KeyHolder(ServerKey::Web) | Principal::Anyone => Ok(Operator::new(
                Permissions::of(&[
                    Permission::Read,
                    Permission::Change,
                    Permission::IssueRecoveryCodes,
                ]),
                Audit::WebPage,
            )),
            Principal::KeyHolder(ServerKey::Access) => Err(RuleError::new(
                Code::Unauthenticated,
                "the access key lets agents in; it does not make an operator",
            )),
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

    fn operator<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, Identified<Operator>> {
        Box::pin(async move {
            Ok(Identified {
                swarm: self.swarm.clone(),
                actor: self.operator_for(principal)?,
            })
        })
    }
}

#[cfg(test)]
mod tests;
