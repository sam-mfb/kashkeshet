//! The open mode: a keyless install, where every caller is let in.

use crate::{Credentials, Decision, Mode, Principal};

/// Lets every caller in, whatever it presents, as [`Principal::Anyone`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Open;

impl Mode for Open {
    fn admit(&self, _credentials: &Credentials) -> Decision {
        Decision::Admitted(Principal::Anyone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_caller_is_let_in_as_anyone() {
        for header in [None, Some(&b"Bearer nope"[..]), Some(&b"Basic eA=="[..])] {
            assert_eq!(
                Open.admit(&Credentials::from_authorization(header)),
                Decision::Admitted(Principal::Anyone)
            );
        }
    }
}
