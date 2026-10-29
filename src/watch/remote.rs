//! `Server` over MCP, with rmcp's client. Connects on first use and again
//! after a call fails to reach the server.

use super::{Failure, Message, Server};
use crate::config::WATCH_BATCH;
use futures::future::BoxFuture;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::{Map, Value, json};
use std::time::Duration;
use tokio::sync::Mutex;

type Client = RunningService<RoleClient, ()>;

/// The server at `url`, reached with the access key `key` (if it needs one),
/// acting as the agent whose token is `token`.
pub struct Remote {
    url: String,
    key: Option<String>,
    token: String,
    client: Mutex<Option<Client>>,
}

impl Remote {
    pub fn new(url: String, key: Option<String>, token: String) -> Remote {
        Remote {
            url,
            key,
            token,
            client: Mutex::new(None),
        }
    }

    async fn connect(&self) -> Result<Client, Failure> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.clone());
        if let Some(key) = &self.key {
            config = config.auth_header(key.clone());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        ().serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
            .await
            .map_err(|e| Failure::Lost(e.to_string()))
    }

    /// Calls `tool` with `arguments` plus the token, asking for JSON.
    async fn call(&self, tool: &str, mut arguments: Map<String, Value>) -> Result<Value, Failure> {
        arguments.insert("token".into(), json!(self.token));
        arguments.insert("format".into(), json!("json"));
        let mut slot = self.client.lock().await;
        let client = match slot.take() {
            Some(client) => client,
            None => self.connect().await?,
        };
        let params = CallToolRequestParams::new(tool.to_string()).with_arguments(arguments);
        // A failed call drops the connection, so the next call makes a new one.
        let result = client
            .call_tool(params)
            .await
            .map_err(|e| Failure::Lost(e.to_string()))?;
        *slot = Some(client);
        let text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect();
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| Failure::Lost(format!("{tool} gave an answer that is not JSON")))?;
        if result.is_error != Some(true) {
            return Ok(value);
        }
        let field = |name: &str| value["error"][name].as_str().map(str::to_string);
        match (field("code"), field("message")) {
            (Some(code), Some(message)) => Err(Failure::Refused { code, message }),
            _ => Err(Failure::Lost(format!(
                "{tool} failed without an error code"
            ))),
        }
    }
}

impl Server for Remote {
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
