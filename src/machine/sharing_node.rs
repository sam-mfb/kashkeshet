//! Adapts kkt-remote-access-tailscale's reader of Tailscale's status answer
//! to the sharing node kkt-containerized-service reads a shared install's
//! login through.

use kkt_containerized_service::{NodeLogin, SharingNode};
use kkt_remote_access_tailscale::Tailscale;

/// The Tailscale container a shared install runs, as Tailscale's own
/// reader reads its status answer.
pub struct TailscaleNode;

impl SharingNode for TailscaleNode {
    fn login(&self, answer: &str) -> Option<NodeLogin> {
        let status = Tailscale::read_status(answer).ok()?;
        Some(if status.running() {
            NodeLogin::LoggedIn
        } else if status.needs_login() {
            NodeLogin::NeedsLogin
        } else {
            NodeLogin::Other(status.backend_state)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscales_answer_says_where_the_login_stands() {
        for (answer, login) in [
            (r#"{"BackendState":"Running"}"#, Some(NodeLogin::LoggedIn)),
            (
                r#"{"BackendState":"NeedsLogin"}"#,
                Some(NodeLogin::NeedsLogin),
            ),
            (
                r#"{"BackendState":"Starting"}"#,
                Some(NodeLogin::Other("Starting".into())),
            ),
            (r#"{"BackendState":3}"#, None),
            ("not json", None),
        ] {
            assert_eq!(TailscaleNode.login(answer), login, "{answer}");
        }
    }
}
