//! Message and channel reads over the shared MCP client.
use super::{Client, Settings};
use crate::{Channel, Failure, Message, NewerLane, Waited, Watched};
use futures::future::BoxFuture;
use kkt_clock::{Clock, Span};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::time::Duration;

/// The name `kkt watch --admin` watches as. No agent can have it: agent
/// names have no parentheses.
const WATCH_ADMIN_NAME: &str = "(admin)";

pub struct Remote {
    client: Client,
    token: String,
    settings: Settings,
}

impl Remote {
    pub fn new(url: String, key: Option<String>, token: String, settings: Settings) -> Self {
        Self {
            client: Client::new(url, key),
            token,
            settings,
        }
    }

    pub fn from_client(client: Client, token: String, settings: Settings) -> Self {
        Self {
            client,
            token,
            settings,
        }
    }

    async fn call(&self, tool: &str, mut arguments: Map<String, Value>) -> Result<Value, Failure> {
        arguments.insert("token".into(), json!(self.token));
        self.client.call(tool, arguments).await
    }
}

impl Watched for Remote {
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        let mut arguments = Map::new();
        arguments.insert("token".into(), json!(self.token));
        Box::pin(all_channels(
            &self.client,
            self.settings.watch_batch,
            arguments,
        ))
    }

    fn handle(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            let value = self.call("whoami", Map::new()).await?;
            value["handle"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| Failure::Lost("whoami gave no handle".into()))
        })
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("after_id".into(), json!(0));
            let value = self.call("activity", arguments).await?;
            // With no messages in any channel there is no `latest_id`.
            Ok(value["latest_id"].as_i64().unwrap_or(0))
        })
    }

    fn message(&self, id: i64) -> BoxFuture<'_, Result<Message, Failure>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("message_id".into(), json!(id));
            let value = self.call("get_message", arguments).await?;
            message(&value).ok_or_else(|| Failure::Lost("get_message gave no message".into()))
        })
    }

    fn wait<'a>(
        &'a self,
        after_id: i64,
        channels: Option<&'a [String]>,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Waited, Failure>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("after_id".into(), json!(after_id));
            arguments.insert("timeout_s".into(), json!(whole_seconds(wait)));
            arguments.insert("limit".into(), json!(self.settings.watch_batch));
            if let Some(channels) = channels {
                arguments.insert("channel_ids".into(), json!(channels));
            }
            let value = self.call("wait_for_messages", arguments).await?;
            let unexpected = || Failure::Lost("wait_for_messages gave no messages list".into());
            let messages = value["messages"]
                .as_array()
                .ok_or_else(unexpected)?
                .iter()
                .map(|m| message(m).ok_or_else(unexpected))
                .collect::<Result<_, _>>()?;
            Ok(Waited {
                messages,
                newer_lane: newer_lane(&value["newer_lane"])?,
            })
        })
    }
}

/// Message and channel reads over the admin endpoint, `/mcp-admin`, for
/// `kkt watch --admin`: every channel, closed and hidden ones included. The
/// admin endpoint does not wait, so `wait` asks again every
/// `settings.watch_admin_poll` on the injected clock until something comes
/// or the wait is over.
pub struct AdminRemote {
    client: Client,
    clock: Arc<dyn Clock>,
    settings: Settings,
}

impl AdminRemote {
    /// `client` must reach the admin endpoint with the admin key.
    pub fn new(client: Client, clock: Arc<dyn Clock>, settings: Settings) -> Self {
        Self {
            client,
            clock,
            settings,
        }
    }

    async fn new_messages(
        &self,
        after_id: i64,
        channels: Option<&[String]>,
    ) -> Result<Vec<Message>, Failure> {
        let mut arguments = Map::new();
        arguments.insert("after_id".into(), json!(after_id));
        arguments.insert("limit".into(), json!(self.settings.watch_batch));
        if let Some(channels) = channels {
            arguments.insert("channel_ids".into(), json!(channels));
        }
        let value = self.client.call("new_messages", arguments).await?;
        let unexpected = || Failure::Lost("new_messages gave no messages list".into());
        value["messages"]
            .as_array()
            .ok_or_else(unexpected)?
            .iter()
            .map(|m| message(m).ok_or_else(unexpected))
            .collect()
    }
}

impl Watched for AdminRemote {
    fn handle(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            // Watching asks for the handle first, even when it
            // resumes from a known place, so this read is what turns a wrong
            // admin key or a missing admin endpoint into a startup failure.
            // `list_agents` is asked because every version of the admin
            // endpoint has it, so a newer `kkt watch` still starts against an
            // older server.
            let mut arguments = Map::new();
            arguments.insert("limit".into(), json!(1));
            self.client.call("list_agents", arguments).await?;
            // No agent can have this handle, so no message is skipped as the
            // watcher's own.
            Ok(WATCH_ADMIN_NAME.to_string())
        })
    }

    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        Box::pin(all_channels(
            &self.client,
            self.settings.watch_batch,
            Map::new(),
        ))
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("after_id".into(), json!(0));
            let value = self.client.call("activity", arguments).await?;
            Ok(value["latest_id"].as_i64().unwrap_or(0))
        })
    }

    fn message(&self, id: i64) -> BoxFuture<'_, Result<Message, Failure>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("message_id".into(), json!(id));
            let value = self.client.call("get_message", arguments).await?;
            message(&value).ok_or_else(|| Failure::Lost("get_message gave no message".into()))
        })
    }

    fn wait<'a>(
        &'a self,
        after_id: i64,
        channels: Option<&'a [String]>,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Waited, Failure>> {
        Box::pin(async move {
            let out_of_range = || Failure::Lost("the clock is out of range".into());
            let end = self
                .clock
                .now()
                .checked_add(Span::from_std(wait).map_err(|_| out_of_range())?)
                .ok_or_else(out_of_range)?;
            let poll =
                Span::from_std(self.settings.watch_admin_poll).map_err(|_| out_of_range())?;
            loop {
                let messages = self.new_messages(after_id, channels).await?;
                let now = self.clock.now();
                if !messages.is_empty() || now >= end {
                    // The operator holds no lane.
                    return Ok(Waited {
                        messages,
                        newer_lane: None,
                    });
                }
                self.clock.sleep_until((now + poll).min(end)).await;
            }
        })
    }
}

/// Every channel `list_channels` gives, following its cursors, `batch` at
/// a time. `arguments` go with every call, besides the limit and cursor.
async fn all_channels(
    client: &Client,
    batch: u32,
    arguments: Map<String, Value>,
) -> Result<Vec<Channel>, Failure> {
    let mut channels = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut call = arguments.clone();
        call.insert("limit".into(), json!(batch));
        if let Some(cursor) = &cursor {
            call.insert("cursor".into(), json!(cursor));
        }
        let value = client.call("list_channels", call).await?;
        let unexpected = || Failure::Lost("list_channels gave an invalid channel list".into());
        for channel in value["channels"].as_array().ok_or_else(unexpected)? {
            channels.push(Channel {
                id: channel["id"].as_str().ok_or_else(unexpected)?.into(),
                name: channel["name"].as_str().ok_or_else(unexpected)?.into(),
            });
        }
        match value["more"].as_str() {
            None if value["more"].is_null() => return Ok(channels),
            Some(more) => {
                let next = more.strip_prefix("cursor=").ok_or_else(unexpected)?;
                if next.is_empty() || cursor.as_deref() == Some(next) {
                    return Err(unexpected());
                }
                cursor = Some(next.to_string());
            }
            None => return Err(unexpected()),
        }
    }
}

/// The `newer_lane` of a `wait_for_messages` answer. `null`, or no field
/// from a server older than the field, means none.
fn newer_lane(value: &Value) -> Result<Option<NewerLane>, Failure> {
    if value.is_null() {
        return Ok(None);
    }
    let unexpected = || Failure::Lost("wait_for_messages gave an invalid newer_lane".into());
    Ok(Some(NewerLane {
        name: value["name"].as_str().ok_or_else(unexpected)?.into(),
        version: value["version"].as_i64().ok_or_else(unexpected)?,
        current_version: value["current_version"].as_i64().ok_or_else(unexpected)?,
    }))
}

fn message(value: &Value) -> Option<Message> {
    let text = |name: &str| value[name].as_str().map(str::to_string);
    Some(Message {
        id: value["id"].as_i64()?,
        channel_name: text("channel_name")?,
        agent: text("agent")?,
        body: text("body")?,
        cut: value["cut"].as_bool()?,
    })
}

/// Rounded up, so a wait of under a second still waits rather than checks.
fn whole_seconds(wait: Duration) -> u64 {
    wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::{NewerLane, newer_lane};
    use serde_json::json;

    #[test]
    fn a_newer_lane_is_read_and_its_absence_is_none() {
        let answer = json!({
            "messages": [],
            "newer_lane": {
                "name": "reviewer",
                "version": 1,
                "current_version": 2,
                "out_of_date": true,
                "taken_at": "2026-11-06T00:00:00Z",
            },
        });
        assert_eq!(
            newer_lane(&answer["newer_lane"]),
            Ok(Some(NewerLane {
                name: "reviewer".into(),
                version: 1,
                current_version: 2,
            }))
        );
        assert_eq!(newer_lane(&json!(null)), Ok(None));
        // A server older than the field leaves it out.
        assert_eq!(
            newer_lane(&json!({ "messages": [] })["newer_lane"]),
            Ok(None)
        );
        assert!(newer_lane(&json!({ "name": "reviewer" })).is_err());
    }
}
