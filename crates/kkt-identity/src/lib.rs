//! Who a caller is: the [`Identify`] contract, the one place that decides
//! who someone is and what they may do, for the MCP endpoints and the web
//! page alike. Admission has already let the caller in, and hands on the
//! [`Principal`] it was admitted as; `Identify` takes it from there, and a
//! caller becomes the swarm it acts in and its actor, or a refusal.
//!
//! It is a contract: each product brings its own implementation, as
//! kkt-identity-keymode's `KeyMode` is kashkeshet's. A stand-in for tests,
//! [`testing::FixedIdentify`], is behind the `testing` feature.
//!
//! This crate knows nothing of HTTP: each surface reads the principal off
//! its own requests and passes it in.

use kkt_admission::Principal;
use kkt_api_agent::{Agent, Api, Operator, SwarmId};
use std::future::Future;
use std::pin::Pin;

/// A future an [`Identify`] implementation returns.
pub type Identifying<'a, T> = Pin<Box<dyn Future<Output = kkt_api_agent::Result<T>> + Send + 'a>>;

/// What a caller was identified as: the swarm it acts in, and who acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identified<A> {
    pub swarm: SwarmId,
    pub actor: A,
}

/// How each caller becomes (swarm, actor), or a refusal. A refusal is a
/// [`kkt_api_agent::RuleError`]; each surface gives it to the caller in its own way.
pub trait Identify: Send + Sync {
    /// An agent tool call carrying `token`, from a caller admitted as
    /// `principal`: the avatar it acts as, and in which swarm. `api` is
    /// the endpoint's operations, to look the token up with, in whichever
    /// swarm the implementation chooses ([`Api::in_swarm`]). The swarm
    /// returned must be the one the avatar was found in: the call acts
    /// there.
    fn avatar<'a>(
        &'a self,
        api: &'a dyn Api,
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

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::*;
    use kkt_api_agent::{Code, RuleError};

    /// A stand-in [`Identify`] for tests: the avatar, swarm and operator
    /// each call is given, set up front. Every caller acts in the one
    /// swarm; an agent call is the avatar given, whatever its token; a call
    /// for an avatar or operator that was not given is refused as
    /// unauthenticated.
    pub struct FixedIdentify {
        pub swarm: SwarmId,
        pub avatar: Option<Agent>,
        pub operator: Option<Operator>,
    }

    fn refused(what: &str) -> RuleError {
        RuleError::new(
            Code::Unauthenticated,
            format!("this test identity has no {what}"),
        )
    }

    impl Identify for FixedIdentify {
        fn avatar<'a>(
            &'a self,
            _api: &'a dyn Api,
            _principal: &'a Principal,
            _token: Option<&'a str>,
        ) -> Identifying<'a, Identified<Agent>> {
            Box::pin(async move {
                let actor = self.avatar.clone().ok_or_else(|| refused("avatar"))?;
                Ok(Identified {
                    swarm: self.swarm.clone(),
                    actor,
                })
            })
        }

        fn newcomer<'a>(&'a self, _principal: &'a Principal) -> Identifying<'a, SwarmId> {
            Box::pin(async move { Ok(self.swarm.clone()) })
        }

        fn operator<'a>(
            &'a self,
            _principal: &'a Principal,
        ) -> Identifying<'a, Identified<Operator>> {
            Box::pin(async move {
                let actor = self.operator.clone().ok_or_else(|| refused("operator"))?;
                Ok(Identified {
                    swarm: self.swarm.clone(),
                    actor,
                })
            })
        }
    }
}

#[cfg(test)]
mod tests;
