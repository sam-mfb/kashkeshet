//! Adapts kkt-remote-access-tailscale's reader of Tailscale's status answer
//! to the sharing node kkt-containerized-service reads a shared install's
//! login through.

use kkt_containerized_service::{NodeLogin, NodeStatus, SharingNode};
use kkt_remote_access_tailscale::Status;
use kkt_remote_access_tailscale::Tailscale;

/// The Tailscale container a shared install runs, as Tailscale's own
/// reader reads its status answer.
pub struct TailscaleNode;

impl SharingNode for TailscaleNode {
    fn status_command(&self) -> Vec<String> {
        Tailscale::STATUS_COMMAND.map(String::from).to_vec()
    }

    fn login(&self, answer: &str) -> Option<NodeLogin> {
        let status = Tailscale::read_status(answer).ok()?;
        Some(login(&status))
    }

    fn status(&self, answer: &str) -> Result<NodeStatus, String> {
        let status = Tailscale::read_status(answer).map_err(|e| e.to_string())?;
        Ok(NodeStatus {
            login: login(&status),
            https: !status.cert_domains.is_empty(),
            link: status.auth_url,
            name: status.dns_name,
        })
    }
}

/// Where the node's login stands, by its backend state.
fn login(status: &Status) -> NodeLogin {
    if status.running() {
        NodeLogin::LoggedIn
    } else if status.needs_login() {
        NodeLogin::NeedsLogin
    } else {
        NodeLogin::Other(status.backend_state.clone())
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

    #[test]
    fn tailscales_answer_gives_the_link_the_name_and_whether_https_is_on() {
        let waiting = TailscaleNode
            .status(r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.example/a"}"#)
            .unwrap();
        assert_eq!(
            waiting,
            NodeStatus {
                login: NodeLogin::NeedsLogin,
                link: Some("https://login.example/a".into()),
                name: None,
                https: false,
            }
        );
        let running = TailscaleNode
            .status(
                r#"{"BackendState":"Running","Self":{"DNSName":"kkt-1.tail1.ts.net."},"CertDomains":["kkt-1.tail1.ts.net"]}"#,
            )
            .unwrap();
        assert_eq!(
            running,
            NodeStatus {
                login: NodeLogin::LoggedIn,
                link: None,
                name: Some("kkt-1.tail1.ts.net".into()),
                https: true,
            }
        );
        assert_eq!(
            TailscaleNode.status(r#"{"Self":{}}"#).unwrap_err(),
            "no BackendState"
        );
        assert!(TailscaleNode.status("not json").is_err());
    }
}
