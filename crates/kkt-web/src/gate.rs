//! Who is viewing: the one contract the page declares for itself. The
//! [`Gate`] says whether a key lets a viewer in, and who an admitted viewer
//! is ([`Viewer`]): the swarm they see and the operator they act as. The
//! page's data and changes are the kkt-api crates', which it calls directly
//! in the viewer's swarm. The server provides the gate over admission and
//! identity (`web_real`); kkt-api-sim provides a stand-in (`PreviewGate`).

use crate::sessions::{Held, Session};
use futures::future::BoxFuture;
use kkt_api_agent::{Operator, SwarmId};

/// Who is viewing, as the [`Gate`] decides it: the swarm whose operations
/// the page shows and changes, and the operator they act as there. What
/// the operator may do is the application's to check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub swarm: SwarmId,
    pub operator: Operator,
}

/// Who is viewing and whether they may see the pages: the two questions
/// the page asks of whoever admits viewers. The page keeps the sessions
/// and form tokens itself ([`Sessions`](crate::Sessions)).
pub trait Gate: Send + Sync {
    /// What a session holds: what the gate said of the key its holder
    /// signed in with ([`Gate::sign_in`]).
    type SignedIn: Held;

    /// Who the viewer with this session is, and in which swarm, or why
    /// they may not see the pages. Asked for every request to a page or the
    /// live feed, whether or not sign-in is required: without it, every
    /// viewer is admitted as the gate's default viewer.
    fn admits(&self, session: Session<Self::SignedIn>) -> BoxFuture<'_, Admit<Viewer>>;

    /// Whether the key a viewer typed lets them in, and as whom. The page
    /// starts a session holding the answer.
    fn sign_in(&self, key: &str) -> SignIn<Self::SignedIn>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admit<V> {
    /// Yes, as this viewer.
    Yes(V),
    /// No, for this reason: the viewer has not signed in, or their session
    /// has ended. Where sign-in is required, they are sent to sign in.
    No(String),
    /// No, for this reason, though the viewer was let in: who they are may
    /// not see the pages, and signing in again would not change that.
    Refused(String),
}

/// The answer to a key typed at sign-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignIn<T> {
    /// The key lets the viewer in; their session holds this.
    Admitted(T),
    /// The key was wrong, for this reason.
    Refused(String),
}
