//! Admission: whether an HTTP caller is let in to `/mcp` or the web page at
//! all. This comes before agent tokens (`rules::auth`) and channel access
//! (`rules::access`), and knows nothing about either.
//!
//! For `/mcp`, the HTTP layer turns a request into [`Credentials`], asks
//! [`Admission::decide`], and then acts on the [`Decision`]: for a caller let
//! in, the [`Principal`] it was admitted as. Every mode of admitting callers
//! provides the same thing, [`Mode::admit`]; there are three, [`Open`],
//! [`SharedKey`] and [`AnyBearer`], and [`Admission`] is the one a server
//! uses. kkt itself uses only the first two.
//!
//! The web page asks [`WebAdmission`] instead. A viewer signs in once with
//! credentials [`Admission`] accepts and gets a session, which the page
//! keeps; later requests are let in by the session alone.

mod any_bearer;
pub mod server_keys;
mod shared_key;

pub use any_bearer::{AnyBearer, BearerToken};
pub use server_keys::key_refusal;
pub use shared_key::SharedKey;

mod open;
pub use open::Open;

use std::sync::Arc;

/// A way of admitting callers: what every mode provides. A mode turns the
/// credential a caller presents into who was admitted, a [`Principal`], or a
/// refusal, and says nothing about agent tokens or channels.
pub trait Mode: Send + Sync {
    /// Decides whether the caller that presented `credentials` is let in.
    fn admit(&self, credentials: &Credentials) -> Decision;
}

/// How a server admits callers: one [`Mode`].
pub enum Admission {
    /// Everyone is let in. The server's behavior before admission existed.
    Open,
    /// Callers must present one shared key as a bearer token.
    SharedKey(SharedKey),
    /// Every caller that presents a bearer token is let in, as
    /// [`Principal::Bearer`], for what comes after admission to judge.
    /// kkt never sets this mode: it is for a program whose `Identify`
    /// decides who a bearer is.
    Bearer,
}

/// How a server admits callers to its admin endpoint, `/mcp-admin`: one of
/// the modes that ask for a credential. It has no open mode, so no server
/// can let a caller without one reach an admin tool; a server with no
/// admin admission does not serve the endpoint at all.
pub enum AdminAdmission {
    /// Callers must present the admin key. kkt's mode. The key must be one
    /// made as [`ServerKey::Admin`]: its holder is admitted as that kind of
    /// key, and `KeyMode` decides what each kind may do, so a web key here
    /// would make the web page's operator at `/mcp-admin`. kkt's programs
    /// pass the key `server_keys::check` made for the admin key.
    SharedKey(SharedKey),
    /// Every caller with a bearer token is let in as
    /// [`Principal::Bearer`], for the program's `Identify` to judge; as
    /// [`Admission::Bearer`].
    Bearer,
}

impl AdminAdmission {
    /// The same mode, as an [`Admission`].
    pub fn into_admission(self) -> Admission {
        match self {
            AdminAdmission::SharedKey(key) => Admission::SharedKey(key),
            AdminAdmission::Bearer => Admission::Bearer,
        }
    }

    /// Where the admin key came from, for the record of a server run.
    /// `None` for the bearer mode. Never the key itself.
    pub fn key_source(&self) -> Option<&str> {
        match self {
            AdminAdmission::SharedKey(key) => Some(key.source()),
            AdminAdmission::Bearer => None,
        }
    }
}

/// What a caller presented to be let in, read from its `Authorization`
/// header. Holds a secret, so it has no `Debug`.
pub struct Credentials {
    presented: Presented,
}

enum Presented {
    Nothing,
    Bearer(Vec<u8>),
    /// A header that is not a bearer token, such as `Basic ...`.
    Other,
}

impl Credentials {
    /// Reads the value of an `Authorization` header, if the request had one.
    /// The scheme name `Bearer` is matched without regard to case.
    pub fn from_authorization(header: Option<&[u8]>) -> Credentials {
        const SCHEME: &[u8] = b"bearer ";
        let presented = match header {
            None => Presented::Nothing,
            Some(value)
                if value.len() > SCHEME.len()
                    && value[..SCHEME.len()].eq_ignore_ascii_case(SCHEME) =>
            {
                Presented::Bearer(value[SCHEME.len()..].trim_ascii().to_vec())
            }
            Some(_) => Presented::Other,
        };
        Credentials { presented }
    }

    /// A key typed into a sign-in form, without surrounding whitespace.
    pub fn from_key(key: &str) -> Credentials {
        Credentials {
            presented: Presented::Bearer(key.trim().as_bytes().to_vec()),
        }
    }

    /// The bearer token presented, or the refusal for presenting none.
    fn bearer(&self) -> Result<&[u8], Refusal> {
        match &self.presented {
            Presented::Nothing => Err(Refusal::NoCredentials),
            Presented::Other => Err(Refusal::NotBearer),
            Presented::Bearer(token) => Ok(token),
        }
    }
}

/// The answer for one caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Admitted(Principal),
    Refused(Refusal),
}

/// Who an admitted caller is, as far as admission can tell. The server keeps
/// it with the request, for what comes after admission to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// No admission check is set, so the caller is not known.
    Anyone,
    /// The caller presented one of the server's keys: which one.
    KeyHolder(ServerKey),
    /// The caller presented this bearer token, which admission has not
    /// judged ([`Admission::Bearer`]).
    Bearer(BearerToken),
}

/// Which of the server's keys a [`SharedKey`] is. Each is a different key
/// (`server_keys::check`), so a caller that presents one is known to hold
/// that one. Admission only records it; what each key may do is decided
/// after admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerKey {
    /// The access key, which lets agents in to `/mcp`.
    Access,
    /// The web key, with which the human signs in to the web page.
    Web,
    /// The admin key, which lets admin agents in to `/mcp-admin`.
    Admin,
}

/// Why a caller was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("an access key is required: send Authorization: Bearer <key>")]
    NoCredentials,
    #[error("the Authorization header must be a bearer token: Bearer <key>")]
    NotBearer,
    #[error("the access key is wrong")]
    WrongKey,
    #[error("sign in first")]
    NoSession,
    #[error("the session has ended; sign in again")]
    SessionEnded,
}

impl Admission {
    /// Decides whether the caller with `credentials` is let in, by this
    /// server's mode.
    pub fn decide(&self, credentials: &Credentials) -> Decision {
        self.mode().admit(credentials)
    }

    fn mode(&self) -> &dyn Mode {
        match self {
            Admission::Open => &Open,
            Admission::SharedKey(key) => key,
            Admission::Bearer => &AnyBearer,
        }
    }

    /// Where the shared key came from, for the record of a server run.
    /// `None` when no key is set. Never the key itself.
    pub fn key_source(&self) -> Option<&str> {
        match self {
            Admission::Open | Admission::Bearer => None,
            Admission::SharedKey(key) => Some(key.source()),
        }
    }
}

/// Admission for the web page: whether sign-in is needed, whether a key is
/// good and whom it admits, and the decision for an open page or a
/// session's holder. With [`Admission::Open`], everyone sees it. Otherwise
/// a viewer needs a live session. The sessions are kkt-web's.
pub struct WebAdmission {
    admission: Arc<Admission>,
}

/// The session a web request presents, as the page found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebSession {
    /// The request carries no session.
    Absent,
    /// The session was never issued, was signed out, or has expired.
    Ended,
    /// A live session, started for this principal.
    Live(Principal),
}

impl WebAdmission {
    pub fn new(admission: Arc<Admission>) -> WebAdmission {
        WebAdmission { admission }
    }

    /// Whether viewers must sign in. When false, `decide` admits everyone.
    pub fn sign_in_required(&self) -> bool {
        !matches!(*self.admission, Admission::Open)
    }

    /// Decides whether the viewer presenting `session` is let in.
    pub fn decide(&self, session: WebSession) -> Decision {
        if !self.sign_in_required() {
            return Decision::Admitted(Principal::Anyone);
        }
        match session {
            WebSession::Absent => Decision::Refused(Refusal::NoSession),
            WebSession::Live(principal) => Decision::Admitted(principal),
            WebSession::Ended => Decision::Refused(Refusal::SessionEnded),
        }
    }

    /// Whether `credentials` match the web page's own key, and whom they
    /// admit: the principal a session started now holds.
    pub fn sign_in(&self, credentials: &Credentials) -> Decision {
        self.admission.decide(credentials)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    fn keyed() -> Admission {
        Admission::SharedKey(SharedKey::new(ServerKey::Access, KEY, "/keys/kkt").expect("key"))
    }

    fn decide(admission: &Admission, header: Option<&str>) -> Decision {
        admission.decide(&Credentials::from_authorization(header.map(str::as_bytes)))
    }

    #[test]
    fn open_admits_everyone() {
        let open = Admission::Open;
        for header in [None, Some("Bearer nope"), Some("Basic eA==")] {
            assert_eq!(decide(&open, header), Decision::Admitted(Principal::Anyone));
        }
        assert_eq!(open.key_source(), None);
    }

    #[test]
    fn the_right_key_is_admitted() {
        let bearer = format!("Bearer {KEY}");
        assert_eq!(
            decide(&keyed(), Some(&bearer)),
            Decision::Admitted(Principal::KeyHolder(ServerKey::Access))
        );
        let lower = format!("bearer  {KEY} ");
        assert_eq!(
            decide(&keyed(), Some(&lower)),
            Decision::Admitted(Principal::KeyHolder(ServerKey::Access))
        );
    }

    #[test]
    fn a_missing_or_wrong_key_is_refused() {
        let admission = keyed();
        assert_eq!(
            decide(&admission, None),
            Decision::Refused(Refusal::NoCredentials)
        );
        assert_eq!(
            decide(&admission, Some("Bearer 0123456789abcdef0123456789abcdeX")),
            Decision::Refused(Refusal::WrongKey)
        );
        assert_eq!(
            decide(&admission, Some("Bearer ")),
            Decision::Refused(Refusal::NotBearer)
        );
        let prefix = format!("Bearer {}", &KEY[..10]);
        assert_eq!(
            decide(&admission, Some(&prefix)),
            Decision::Refused(Refusal::WrongKey)
        );
        let basic = format!("Basic {KEY}");
        assert_eq!(
            decide(&admission, Some(&basic)),
            Decision::Refused(Refusal::NotBearer)
        );
    }

    #[test]
    fn an_admin_admission_always_asks_for_a_credential() {
        let key = SharedKey::new(ServerKey::Admin, KEY, "/keys/admin").expect("key");
        let keyed = AdminAdmission::SharedKey(key);
        assert_eq!(keyed.key_source(), Some("/keys/admin"));
        assert_eq!(AdminAdmission::Bearer.key_source(), None);
        for admission in [
            keyed.into_admission(),
            AdminAdmission::Bearer.into_admission(),
        ] {
            assert!(!matches!(admission, Admission::Open));
            assert_eq!(
                decide(&admission, None),
                Decision::Refused(Refusal::NoCredentials)
            );
            assert_eq!(
                decide(&admission, Some("Basic eA==")),
                Decision::Refused(Refusal::NotBearer)
            );
        }
    }

    #[test]
    fn the_key_source_is_recorded_but_not_the_key() {
        assert_eq!(keyed().key_source(), Some("/keys/kkt"));
    }

    fn web(admission: Admission) -> WebAdmission {
        WebAdmission::new(Arc::new(admission))
    }

    #[test]
    fn an_open_web_page_needs_no_session() {
        let web = web(Admission::Open);
        assert!(!web.sign_in_required());
        for session in [
            WebSession::Absent,
            WebSession::Ended,
            WebSession::Live(Principal::KeyHolder(ServerKey::Web)),
        ] {
            assert_eq!(web.decide(session), Decision::Admitted(Principal::Anyone));
        }
    }

    #[test]
    fn a_keyed_web_page_admits_a_live_sessions_principal() {
        let web = web(keyed());
        assert!(web.sign_in_required());
        assert_eq!(
            web.decide(WebSession::Absent),
            Decision::Refused(Refusal::NoSession)
        );
        assert_eq!(
            web.decide(WebSession::Ended),
            Decision::Refused(Refusal::SessionEnded)
        );
        assert_eq!(
            web.decide(WebSession::Live(Principal::KeyHolder(ServerKey::Access))),
            Decision::Admitted(Principal::KeyHolder(ServerKey::Access))
        );
    }

    #[test]
    fn signing_in_checks_the_key_typed() {
        let web = web(keyed());
        assert_eq!(
            web.sign_in(&Credentials::from_key(&format!(" {KEY}\n"))),
            Decision::Admitted(Principal::KeyHolder(ServerKey::Access))
        );
        assert_eq!(
            web.sign_in(&Credentials::from_key("0123456789abcdef0123456789abcdeX")),
            Decision::Refused(Refusal::WrongKey)
        );
    }
}
