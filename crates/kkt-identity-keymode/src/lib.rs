//! Key mode's identity: [`KeyMode`], which implements kkt-identity's
//! [`Identify`] directly, a shared contract. It decides who a caller is from
//! the key that let it in and the avatar's token, in the one swarm a
//! key-mode server has: an agent call is the avatar its token belongs to;
//! `register` and `reclaim` are always allowed; the operator is decided by
//! which of the server's keys admitted the caller ([`KeyMode::operator`]).

use kkt_admission::{Principal, ServerKey};
use kkt_api_agent::{
    Agent, Api, Audit, Code, Operator, Permission, Permissions, RuleError, SwarmId,
};
use kkt_identity::{Identified, Identify, Identifying};
use kkt_names::AdminName;

/// Key mode: one swarm, `sw_main`; an agent is who its token says; anyone
/// admitted may register or reclaim; the operator is decided by the key
/// that admitted the caller ([`KeyMode::operator`]). A caller admitted as
/// [`Principal::Bearer`] is refused everything: kkt never admits one, so
/// one here means the server was put together with another admission mode.
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
    /// - an unjudged bearer: no operator ([`KeyMode::refuse_bearer`]).
    fn operator_for(&self, principal: &Principal) -> kkt_api_agent::Result<Operator> {
        match principal {
            Principal::Bearer(_) => Err(KeyMode::refuse_bearer()),
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

    /// The refusal for a caller admitted as [`Principal::Bearer`], which key
    /// mode never admits.
    fn refuse_bearer() -> RuleError {
        RuleError::new(
            Code::Unauthenticated,
            "this server is let in to by its keys, not by other bearer tokens",
        )
    }

    /// Refuses a [`Principal::Bearer`], and lets every other principal on.
    fn check_admitted(principal: &Principal) -> kkt_api_agent::Result<()> {
        match principal {
            Principal::Bearer(_) => Err(KeyMode::refuse_bearer()),
            Principal::Anyone | Principal::KeyHolder(_) => Ok(()),
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
        Operator::new(
            permissions,
            Audit::AdminEndpoint {
                key_name: name.map(|n| n.to_string()),
            },
        )
    }
}

impl Identify for KeyMode {
    fn avatar<'a>(
        &'a self,
        api: &'a dyn Api,
        principal: &'a Principal,
        token: Option<&'a str>,
    ) -> Identifying<'a, Identified<Agent>> {
        Box::pin(async move {
            KeyMode::check_admitted(principal)?;
            let actor = api.in_swarm(self.swarm.clone()).authenticate(token).await?;
            Ok(Identified {
                swarm: self.swarm.clone(),
                actor,
            })
        })
    }

    fn newcomer<'a>(&'a self, principal: &'a Principal) -> Identifying<'a, SwarmId> {
        Box::pin(async move {
            KeyMode::check_admitted(principal)?;
            Ok(self.swarm.clone())
        })
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
