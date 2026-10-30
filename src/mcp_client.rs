//! Shared MCP client for identity commands and watching. Connects on first use and again
//! after a call fails to reach the server.

use std::fmt;

mod watching;
pub use watching::{AdminRemote, Remote};

/// Why a call to the server failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The server could not be reached, the connection broke, or its answer
    /// made no sense.
    Lost(String),
    /// The server answered with one of its error codes, such as
    /// `unauthenticated`.
    Refused { code: String, message: String },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Lost(reason) => write!(f, "could not reach the server: {reason}"),
            Failure::Refused { code, message } => write!(f, "{code}: {message}"),
        }
    }
}
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

type Session = RunningService<RoleClient, ()>;

/// Credentials returned once by registration. Never format these in output.
pub struct Registered {
    pub name: String,
    pub token: String,
    pub recovery_code: String,
}

/// A replacement token for an existing identity.
pub struct Reclaimed {
    pub name: String,
    pub token: String,
}

/// The server at `url`, reached with the access key `key` (if it needs one),
/// with no agent credentials held in the transport.
pub struct Client {
    url: String,
    key: Option<String>,
    client: Mutex<Option<Session>>,
}

impl Client {
    /// Registers a new identity, named `name` if given, else by the server.
    pub async fn register(&self, name: Option<&str>) -> Result<Registered, Failure> {
        let mut args = Map::new();
        if let Some(name) = name {
            args.insert("name".into(), json!(name));
        }
        let value = self.call("register", args).await?;
        Ok(Registered {
            name: field(&value, "name")?,
            token: field(&value, "token")?,
            recovery_code: field(&value, "recovery_code")?,
        })
    }

    pub async fn reclaim(&self, name: &str, recovery_code: &str) -> Result<Reclaimed, Failure> {
        let mut args = Map::new();
        args.insert("name".into(), json!(name));
        args.insert("recovery_code".into(), json!(recovery_code));
        let value = self.call("reclaim", args).await?;
        Ok(Reclaimed {
            name: field(&value, "name")?,
            token: field(&value, "token")?,
        })
    }

    pub async fn set_description(&self, token: &str, description: &str) -> Result<(), Failure> {
        let mut args = Map::new();
        args.insert("token".into(), json!(token));
        args.insert("text".into(), json!(description));
        self.call("set_description", args).await?;
        Ok(())
    }

    pub fn new(url: String, key: Option<String>) -> Client {
        Client {
            url,
            key,
            client: Mutex::new(None),
        }
    }

    async fn connect(&self) -> Result<Session, Failure> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.clone());
        if let Some(key) = &self.key {
            config = config.auth_header(key.clone());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        ().serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
            .await
            .map_err(|_| Failure::Lost("MCP connection failed".into()))
    }

    /// Calls `tool`, asking for JSON.
    async fn call(&self, tool: &str, mut arguments: Map<String, Value>) -> Result<Value, Failure> {
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
            .map_err(|_| Failure::Lost("MCP connection failed".into()))?;
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

fn field(value: &Value, name: &str) -> Result<String, Failure> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Failure::Lost(format!("the server gave no {name}")))
}
