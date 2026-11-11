//! The bearer mode: every caller that presents a bearer token is let in,
//! carrying the token on as its [`Principal`], for the program's `Identify`
//! to judge. kkt never sets it; it is for a program that decides who a
//! bearer is after admission.

use crate::{Credentials, Decision, Mode, Principal, Refusal};
use std::fmt;

/// Lets in every caller that presents a bearer token, as
/// [`Principal::Bearer`]. A caller that presents none, or a header that is
/// not a bearer token, is refused as by [`SharedKey`](crate::SharedKey). So
/// is an empty token, or one that is not UTF-8 text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnyBearer;

impl Mode for AnyBearer {
    fn admit(&self, credentials: &Credentials) -> Decision {
        match credentials.bearer() {
            Err(refusal) => Decision::Refused(refusal),
            // An empty sign-in form presents nothing.
            Ok([]) => Decision::Refused(Refusal::NoCredentials),
            Ok(token) => match std::str::from_utf8(token) {
                Ok(text) => Decision::Admitted(Principal::Bearer(BearerToken(text.to_owned()))),
                Err(_) => Decision::Refused(Refusal::NotBearer),
            },
        }
    }
}

/// A bearer token a caller presented, not judged by admission. It may be a
/// secret, so its `Debug` does not show it.
#[derive(Clone, PartialEq, Eq)]
pub struct BearerToken(String);

impl BearerToken {
    /// A token with `text`, as admission would carry it on: for an
    /// `Identify` implementation's tests.
    pub fn new(text: impl Into<String>) -> BearerToken {
        BearerToken(text.into())
    }

    /// The token's text, for `Identify` to judge. Do not log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BearerToken(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Admission;

    fn decide(header: Option<&[u8]>) -> Decision {
        Admission::Bearer.decide(&Credentials::from_authorization(header))
    }

    fn bearer(text: &str) -> Decision {
        Decision::Admitted(Principal::Bearer(BearerToken(text.to_owned())))
    }

    #[test]
    fn any_bearer_token_is_let_in_and_carried_on() {
        assert_eq!(decide(Some(b"Bearer swarm-a")), bearer("swarm-a"));
        assert_eq!(decide(Some(b"bearer   swarm-b ")), bearer("swarm-b"));
        let Decision::Admitted(Principal::Bearer(token)) = decide(Some(b"Bearer x y")) else {
            panic!("admitted as a bearer");
        };
        assert_eq!(token.expose(), "x y");
    }

    #[test]
    fn no_bearer_token_is_refused_as_with_a_shared_key() {
        assert_eq!(decide(None), Decision::Refused(Refusal::NoCredentials));
        assert_eq!(
            decide(Some(b"Basic eA==")),
            Decision::Refused(Refusal::NotBearer)
        );
        assert_eq!(
            decide(Some(b"Bearer ")),
            Decision::Refused(Refusal::NotBearer)
        );
        assert_eq!(
            decide(Some(b"Bearer \xff\xfe")),
            Decision::Refused(Refusal::NotBearer)
        );
    }

    #[test]
    fn the_token_is_not_shown_in_debug_output() {
        let shown = format!("{:?}", bearer("sekrit-token"));
        assert!(!shown.contains("sekrit"), "{shown}");
        assert!(shown.contains("Bearer"), "{shown}");
        assert_eq!(Admission::Bearer.key_source(), None);
    }

    #[test]
    fn the_web_page_signs_in_a_bearer_with_its_token() {
        use crate::{WebAdmission, WebSession};
        use std::sync::Arc;

        let web = WebAdmission::new(Arc::new(Admission::Bearer));
        assert!(web.sign_in_required());
        let Decision::Admitted(principal) = web.sign_in(&Credentials::from_key(" swarm-a\n"))
        else {
            panic!("signed in");
        };
        assert_eq!(web.decide(WebSession::Live(principal)), bearer("swarm-a"));
        assert_eq!(
            web.sign_in(&Credentials::from_key(" ")),
            Decision::Refused(Refusal::NoCredentials)
        );
    }
}
