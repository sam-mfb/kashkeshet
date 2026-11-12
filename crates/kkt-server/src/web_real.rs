//! The server's side of the web contract: [`AdmissionGate`] is the gate
//! over [`WebAdmission`] and [`Identify`], which decides who each viewer is
//! (kkt-web's [`Viewer`]: a swarm and an operator). It only translates
//! between admission's and identity's types and the web's: who someone is
//! and what they may do is decided by `Identify` alone. The page reads and
//! changes the rules themselves, through the kkt-api crates' contracts.

use futures::future::BoxFuture;
use kkt_admission::{Credentials, Decision, Principal, WebAdmission, WebSession};
use kkt_identity::Identify;
use kkt_web::{Admit, Gate, Session, SignIn, Viewer};
use std::sync::Arc;

/// The web page's gate over the server's [`WebAdmission`] and
/// [`Identify`]. Admission says whether the viewer's session (or, with no
/// sign-in, anyone) is let in and as which principal, and whether a key
/// typed at sign-in is good; `Identify` says which operator that principal
/// is, and in which swarm, exactly as it does for the admin endpoint. The
/// gate only translates their answers into the web's. The sessions, which
/// hold the principal a key admitted, are kkt-web's.
pub struct AdmissionGate {
    admission: WebAdmission,
    identify: Arc<dyn Identify>,
}

impl AdmissionGate {
    pub fn new(admission: WebAdmission, identify: Arc<dyn Identify>) -> AdmissionGate {
        AdmissionGate {
            admission,
            identify,
        }
    }
}

impl Gate for AdmissionGate {
    type SignedIn = Principal;

    fn admits(&self, session: Session<Principal>) -> BoxFuture<'_, Admit<Viewer>> {
        Box::pin(async move {
            let session = match session {
                Session::Absent => WebSession::Absent,
                Session::Ended => WebSession::Ended,
                Session::Live(principal) => WebSession::Live(principal),
            };
            let principal = match self.admission.decide(session) {
                Decision::Admitted(principal) => principal,
                Decision::Refused(refusal) => return Admit::No(refusal.to_string()),
            };
            match self.identify.operator(&principal).await {
                Ok(who) => Admit::Yes(Viewer {
                    swarm: who.swarm,
                    operator: who.actor,
                }),
                Err(e) => Admit::Refused(e.message),
            }
        })
    }

    fn sign_in(&self, key: &str) -> SignIn<Principal> {
        match self.admission.sign_in(&Credentials::from_key(key)) {
            Decision::Admitted(principal) => SignIn::Admitted(principal),
            Decision::Refused(refusal) => SignIn::Refused(refusal.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod swarm_tests;
