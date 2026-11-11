//! Adapts the MCP client's own reads and failures to the watch contract.

use futures::future::BoxFuture;
use kkt_agent_client as client;
use kkt_watch as watch;
use std::time::Duration;

/// The program's adapter for watching either MCP endpoint.
pub enum WatchServer {
    Agent(client::Remote),
    Admin(client::AdminRemote),
}

impl watch::Server for WatchServer {
    fn handle(&self) -> BoxFuture<'_, Result<String, watch::Failure>> {
        Box::pin(async move {
            match self {
                Self::Agent(remote) => remote.handle().await,
                Self::Admin(remote) => remote.handle().await,
            }
            .map_err(failure)
        })
    }

    fn channels(&self) -> BoxFuture<'_, Result<Vec<watch::Channel>, watch::Failure>> {
        Box::pin(async move {
            match self {
                Self::Agent(remote) => remote.channels().await,
                Self::Admin(remote) => remote.channels().await,
            }
            .map(|channels| channels.into_iter().map(channel).collect())
            .map_err(failure)
        })
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, watch::Failure>> {
        Box::pin(async move {
            match self {
                Self::Agent(remote) => remote.latest_id().await,
                Self::Admin(remote) => remote.latest_id().await,
            }
            .map_err(failure)
        })
    }

    fn message(&self, id: i64) -> BoxFuture<'_, Result<watch::Message, watch::Failure>> {
        Box::pin(async move {
            match self {
                Self::Agent(remote) => remote.message(id).await,
                Self::Admin(remote) => remote.message(id).await,
            }
            .map(message)
            .map_err(failure)
        })
    }

    fn wait<'a>(
        &'a self,
        after_id: i64,
        channels: Option<&'a [String]>,
        wait: Duration,
    ) -> BoxFuture<'a, Result<watch::Waited, watch::Failure>> {
        Box::pin(async move {
            let waited = match self {
                Self::Agent(remote) => remote.wait(after_id, channels, wait).await,
                Self::Admin(remote) => remote.wait(after_id, channels, wait).await,
            }
            .map_err(failure)?;
            Ok(watch::Waited {
                messages: waited.messages.into_iter().map(message).collect(),
                newer_lane: waited.newer_lane.map(|lane| watch::NewerLane {
                    name: lane.name,
                    version: lane.version,
                    current_version: lane.current_version,
                }),
            })
        })
    }
}

fn channel(channel: client::Channel) -> watch::Channel {
    watch::Channel {
        id: channel.id,
        name: channel.name,
    }
}

fn message(message: client::Message) -> watch::Message {
    watch::Message {
        id: message.id,
        channel_name: message.channel_name,
        agent: message.agent,
        body: message.body,
        cut: message.cut,
    }
}

/// Keeps retryable failures, server refusals and rejected keys distinct.
fn failure(failure: client::Failure) -> watch::Failure {
    match failure {
        client::Failure::Lost(reason) => watch::Failure::Lost(reason),
        client::Failure::Refused { code, message } => watch::Failure::Refused { code, message },
        client::Failure::KeyRefused { key } => watch::Failure::KeyRefused { key },
    }
}

#[cfg(test)]
mod tests {
    use super::failure;
    use kkt_watch::Failure;

    #[test]
    fn a_lost_connection_stays_lost() {
        let client = kkt_agent_client::Failure::Lost("MCP connection failed".into());
        let text = client.to_string();
        let watch = failure(client);
        assert_eq!(watch, Failure::Lost("MCP connection failed".into()));
        assert_eq!(watch.to_string(), text);
    }

    #[test]
    fn a_refusal_keeps_its_code_and_message() {
        let client = kkt_agent_client::Failure::Refused {
            code: "rate_limited".into(),
            message: "slow down".into(),
        };
        let text = client.to_string();
        let watch = failure(client);
        assert_eq!(
            watch,
            Failure::Refused {
                code: "rate_limited".into(),
                message: "slow down".into(),
            }
        );
        assert_eq!(watch.to_string(), text);
    }

    #[test]
    fn a_refused_key_stays_refused_and_keeps_its_name() {
        for key in [Some("the access key in /home/me/key".to_string()), None] {
            let client = kkt_agent_client::Failure::KeyRefused { key: key.clone() };
            let text = client.to_string();
            let watch = failure(client);
            assert_eq!(watch, Failure::KeyRefused { key });
            assert_eq!(watch.to_string(), text);
        }
    }
}
