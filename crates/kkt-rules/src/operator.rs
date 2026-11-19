//! The operator as an actor: whoever runs the server, acting through the
//! web page or the admin endpoint. An [`Operator`] carries what it may do,
//! its [`Permission`]s, which the rules check, never which surface asked;
//! and an [`Audit`] record of where the request came from, which the rules
//! store with what it changes and which [`OperatorWording`] turns into the
//! words of a notice. The edges that admit a caller build its `Operator`.

use kkt_api_agent::Audit;

/// The words a notice uses for an operator, such as "The operator" or
/// "The operator (via sentinel)". The rules write notices but do not word
/// the operator: this is given to them.
pub trait OperatorWording: Send + Sync {
    /// How a notice names the operator that made a change, at the start of
    /// a sentence.
    fn operator(&self, audit: &Audit) -> String;
}
