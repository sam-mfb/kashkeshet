//! The watch contract over the shared MCP client.
use super::Client;
use crate::clock::Clock;
use crate::config::{WATCH_ADMIN_NAME, WATCH_ADMIN_POLL, WATCH_BATCH};
use crate::watch::{Channel, Failure, Message, Server};
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::time::Duration;

pub struct Remote {
    client: Client,
    token: String,
}

impl Remote {
    pub fn new(url: String, key: Option<String>, token: String) -> Self {
        Self {
            client: Client::new(url, key),
            token,
        }
    }

    pub fn from_client(client: Client, token: String) -> Self {
        Self { client, token }
    }

    async fn call(&self, tool: &str, mut arguments: Map<String, Value>) -> Result<Value, Failure> {
        arguments.insert("token".into(), json!(self.token));
        self.client.call(tool, arguments).await
    }
}

impl Server for Remote {
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        let mut arguments = Map::new();
        arguments.insert("token".into(), json!(self.token));
        Box::pin(all_channels(&self.client, arguments))
    }

    fn name(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            let value = self.call("whoami", Map::new()).await?;
            value["name"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| Failure::Lost("whoami gave no name".into()))
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
    ) -> BoxFuture<'a, Result<Vec<Message>, Failure>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("after_id".into(), json!(after_id));
            arguments.insert("timeout_s".into(), json!(whole_seconds(wait)));
            arguments.insert("limit".into(), json!(WATCH_BATCH));
            if let Some(channels) = channels {
                arguments.insert("channel_ids".into(), json!(channels));
            }
            let value = self.call("wait_for_messages", arguments).await?;
            let unexpected = || Failure::Lost("wait_for_messages gave no messages list".into());
            value["messages"]
                .as_array()
                .ok_or_else(unexpected)?
                .iter()
                .map(|m| message(m).ok_or_else(unexpected))
                .collect()
        })
    }
}

/// The watch contract over the admin endpoint, `/mcp-admin`, for
/// `kkt watch --admin`: every channel, closed and hidden ones included. The
/// admin endpoint does not wait, so `wait` asks again every
/// `WATCH_ADMIN_POLL` on the injected clock until something comes or the
/// wait is over.
pub struct AdminRemote {
    client: Client,
    clock: Arc<dyn Clock>,
}

impl AdminRemote {
    /// `client` must reach the admin endpoint with the admin key.
    pub fn new(client: Client, clock: Arc<dyn Clock>) -> Self {
        Self { client, clock }
    }

    async fn new_messages(
        &self,
        after_id: i64,
        channels: Option<&[String]>,
    ) -> Result<Vec<Message>, Failure> {
        let mut arguments = Map::new();
        arguments.insert("after_id".into(), json!(after_id));
        arguments.insert("limit".into(), json!(WATCH_BATCH));
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

impl Server for AdminRemote {
    fn name(&self) -> BoxFuture<'_, Result<String, Failure>> {
        // No agent can have this name, so no message is skipped as the
        // watcher's own.
        Box::pin(async { Ok(WATCH_ADMIN_NAME.to_string()) })
    }

    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        Box::pin(all_channels(&self.client, Map::new()))
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
    ) -> BoxFuture<'a, Result<Vec<Message>, Failure>> {
        Box::pin(async move {
            let out_of_range = || Failure::Lost("the clock is out of range".into());
            let end = self
                .clock
                .now()
                .checked_add_signed(chrono::Duration::from_std(wait).map_err(|_| out_of_range())?)
                .ok_or_else(out_of_range)?;
            let poll = chrono::Duration::from_std(WATCH_ADMIN_POLL).map_err(|_| out_of_range())?;
            loop {
                let messages = self.new_messages(after_id, channels).await?;
                let now = self.clock.now();
                if !messages.is_empty() || now >= end {
                    return Ok(messages);
                }
                self.clock.sleep_until((now + poll).min(end)).await;
            }
        })
    }
}

/// Every channel `list_channels` gives, following its cursors. `arguments`
/// go with every call, besides the limit and cursor.
async fn all_channels(
    client: &Client,
    arguments: Map<String, Value>,
) -> Result<Vec<Channel>, Failure> {
    let mut channels = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut call = arguments.clone();
        call.insert("limit".into(), json!(WATCH_BATCH));
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
