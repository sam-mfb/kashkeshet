//! The client itself, over rmcp and reqwest.

mod http;
pub use http::{Http, HttpReply};

mod watching;
use watching::AdminRemote;
pub use watching::Remote;

use crate::Settings;
use futures::future::BoxFuture;
use kkt_agent_client::{Address, Avatars, Failure, Reclaimed, Registered, Servers, Watched};
use kkt_clock::Clock;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{ClientInitializeError, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::transport::{DynamicTransportError, StreamableHttpClientTransport};
use rmcp::{ClientLifecycleMode, ClientServiceExt, ServiceError};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;

type Session = RunningService<RoleClient, ()>;

/// kkt's servers over MCP: what the program injects as kkt-agent-client's
/// `Servers`. Each client it makes asks with [`Settings::DEFAULT`].
pub struct Mcp;

impl Mcp {
    fn client(at: Address) -> Client {
        let client = Client::new(at.url, at.key);
        match at.key_name {
            Some(name) => client.describe_key(name),
            None => client,
        }
    }
}

impl Servers for Mcp {
    fn avatars(&self, at: Address) -> Box<dyn Avatars> {
        Box::new(Mcp::client(at))
    }
    fn agent(&self, at: Address, token: String) -> Box<dyn Watched> {
        Box::new(Remote::from_client(
            Mcp::client(at),
            token,
            Settings::DEFAULT,
        ))
    }
    fn admin(&self, at: Address, clock: Arc<dyn Clock>) -> Box<dyn Watched> {
        Box::new(AdminRemote::new(Mcp::client(at), clock, Settings::DEFAULT))
    }
}

impl Avatars for Client {
    fn register<'a>(
        &'a self,
        handle: Option<&'a str>,
        lane: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Registered, Failure>> {
        Box::pin(Client::register(self, handle, lane))
    }
    fn reclaim<'a>(
        &'a self,
        handle: &'a str,
        recovery_code: &'a str,
    ) -> BoxFuture<'a, Result<Reclaimed, Failure>> {
        Box::pin(Client::reclaim(self, handle, recovery_code))
    }
    fn describe<'a>(&'a self, token: &'a str, text: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(self.set_description(token, text))
    }
}

/// The server at `url`, reached with the access key `key` (if it needs one),
/// with no agent credentials held in the transport.
pub struct Client {
    url: String,
    key: Option<String>,
    /// What `KeyRefused` calls the key; see `describe_key`.
    key_description: Option<String>,
    client: Mutex<Option<Session>>,
}

impl Client {
    /// Registers a new avatar, with the handle `handle` if given, else one
    /// the server chooses, taking the lane named `lane` if given.
    pub async fn register(
        &self,
        handle: Option<&str>,
        lane: Option<&str>,
    ) -> Result<Registered, Failure> {
        let mut args = Map::new();
        if let Some(handle) = handle {
            args.insert("handle".into(), json!(handle));
        }
        if let Some(lane) = lane {
            args.insert("lane".into(), json!(lane));
        }
        let value = self.call("register", args).await?;
        Ok(Registered {
            handle: field(&value, "handle")?,
            token: field(&value, "token")?,
            recovery_code: field(&value, "recovery_code")?,
        })
    }

    pub async fn reclaim(&self, handle: &str, recovery_code: &str) -> Result<Reclaimed, Failure> {
        let mut args = Map::new();
        args.insert("handle".into(), json!(handle));
        args.insert("recovery_code".into(), json!(recovery_code));
        let value = self.call("reclaim", args).await?;
        Ok(Reclaimed {
            handle: field(&value, "handle")?,
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
            key_description: None,
            client: Mutex::new(None),
        }
    }

    /// Names the key in a `KeyRefused` failure, such as `the access key in
    /// /home/me/.kkt/key`, so its message says which key the server refused.
    pub fn describe_key(self, description: String) -> Client {
        Client {
            key_description: Some(description),
            ..self
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
            .map_err(|e| self.failure(refused_at_start(&e), &connection_failed(start_cause(&e))))
    }

    /// `KeyRefused` if the server refused the key, else `Lost` for `reason`.
    fn failure(&self, key_refused: bool, reason: &str) -> Failure {
        if key_refused {
            Failure::KeyRefused {
                key: self.key_description.clone(),
            }
        } else {
            Failure::Lost(reason.into())
        }
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
            .map_err(|e| self.failure(refused_in_call(&e), &connection_failed(call_cause(&e))))?;
        *slot = Some(client);
        let text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect();
        let value = parse_answer(tool, &text)?;
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

/// Whether connecting failed because the server refused the key.
fn refused_at_start(error: &ClientInitializeError) -> bool {
    matches!(error, ClientInitializeError::TransportError { error, .. } if refused(error))
}

/// Whether a call failed because the server refused the key, as it does
/// once the key changes under a running connection.
fn refused_in_call(error: &ServiceError) -> bool {
    matches!(error, ServiceError::TransportSend(error) if refused(error))
}

/// What to say when the connection fails, with its cause.
fn connection_failed(cause: String) -> String {
    format!("MCP connection failed ({cause})")
}

/// Why starting a session failed, in words of our own: never the error's
/// text, which can quote the server's answer or the address.
fn start_cause(error: &ClientInitializeError) -> String {
    match error {
        ClientInitializeError::TransportError { error, .. } => transport_cause(error),
        ClientInitializeError::ConnectionClosed(_) => "the connection closed".into(),
        _ => "the server's answer was not MCP".into(),
    }
}

/// Why a call failed, in words of our own, as [`start_cause`] says.
fn call_cause(error: &ServiceError) -> String {
    match error {
        ServiceError::TransportSend(error) => transport_cause(error),
        ServiceError::TransportClosed => "the connection closed".into(),
        ServiceError::Timeout { .. } => "the server did not answer in time".into(),
        _ => "the server's answer was not MCP".into(),
    }
}

/// Why the HTTP transport failed: its kind, and an HTTP status when there
/// is one, read from the error's fields and never from its text.
fn transport_cause(error: &DynamicTransportError) -> String {
    let Some(error) = error
        .error
        .downcast_ref::<StreamableHttpError<reqwest::Error>>()
    else {
        return "the transport failed".into();
    };
    match error {
        StreamableHttpError::Client(e) if e.is_connect() => {
            "the server could not be reached".into()
        }
        StreamableHttpError::Client(e) if e.is_timeout() => {
            "the server did not answer in time".into()
        }
        StreamableHttpError::Client(e) => match e.status() {
            Some(status) => format!("the server answered HTTP {}", status.as_u16()),
            None => "the request failed".into(),
        },
        StreamableHttpError::Io(_) => "reading or writing the connection failed".into(),
        StreamableHttpError::UnexpectedEndOfStream
        | StreamableHttpError::TransportChannelClosed => "the connection closed".into(),
        StreamableHttpError::SessionExpired => "the session expired".into(),
        StreamableHttpError::AuthRequired(_) => "the server asked for a key".into(),
        _ => "the server's answer was not MCP".into(),
    }
}

/// Whether the HTTP transport's error is the server's 401 with a
/// `WWW-Authenticate` challenge, which rmcp reports as `AuthRequired`: the
/// status, not the error's text, decides.
fn refused(error: &DynamicTransportError) -> bool {
    matches!(
        error
            .error
            .downcast_ref::<StreamableHttpError<reqwest::Error>>(),
        Some(StreamableHttpError::AuthRequired(_))
    )
}

/// `tool`'s answer `text` as JSON. When it is not JSON, the failure gives the
/// parser's reason: only the kind of error and where, never its message,
/// which can quote the answer, and answers can hold tokens and recovery codes.
fn parse_answer(tool: &str, text: &str) -> Result<Value, Failure> {
    serde_json::from_str(text).map_err(|e| {
        let kind = match e.classify() {
            serde_json::error::Category::Io => "read error",
            serde_json::error::Category::Syntax => "syntax error",
            serde_json::error::Category::Data => "data error",
            serde_json::error::Category::Eof => "unexpected end of input",
        };
        Failure::Lost(format!(
            "{tool} gave an answer that is not JSON: {kind} at line {}, column {}",
            e.line(),
            e.column()
        ))
    })
}

fn field(value: &Value, name: &str) -> Result<String, Failure> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Failure::Lost(format!("the server gave no {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::streamable_http_client::AuthRequiredError;

    fn transport(error: StreamableHttpError<reqwest::Error>) -> DynamicTransportError {
        DynamicTransportError::from_parts("test", std::any::TypeId::of::<()>(), Box::new(error))
    }

    fn challenge() -> StreamableHttpError<reqwest::Error> {
        StreamableHttpError::AuthRequired(AuthRequiredError::new("Bearer".into()))
    }

    /// Not a refusal, though its text names the status.
    fn other() -> StreamableHttpError<reqwest::Error> {
        StreamableHttpError::UnexpectedServerResponse("HTTP 401 Unauthorized: no".into())
    }

    #[test]
    fn a_failed_connection_says_why_in_words_of_its_own() {
        // The server's text, which may quote what it was sent, never shows.
        let said = "HTTP 500: token act_secret";
        let at_start = ClientInitializeError::TransportError {
            error: transport(StreamableHttpError::UnexpectedServerResponse(said.into())),
            context: "send initialize request".into(),
        };
        assert_eq!(start_cause(&at_start), "the server's answer was not MCP");
        assert_eq!(
            start_cause(&ClientInitializeError::ConnectionClosed(said.into())),
            "the connection closed"
        );
        assert_eq!(
            call_cause(&ServiceError::TransportClosed),
            "the connection closed"
        );
        assert_eq!(
            call_cause(&ServiceError::TransportSend(transport(
                StreamableHttpError::SessionExpired
            ))),
            "the session expired"
        );
        assert_eq!(
            call_cause(&ServiceError::TransportSend(transport(
                StreamableHttpError::Io(std::io::Error::other(said))
            ))),
            "reading or writing the connection failed"
        );
        assert_eq!(
            connection_failed("the connection closed".into()),
            "MCP connection failed (the connection closed)"
        );
    }

    #[tokio::test]
    async fn a_server_that_is_not_there_is_said_to_be_unreachable() {
        // A port that was free a moment ago, with nothing listening on it now.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("a free port")
            .port();
        let client = Client::new(format!("http://127.0.0.1:{port}/mcp"), None);
        match client.reclaim("amber-falcon", "code").await {
            Err(Failure::Lost(said)) => assert_eq!(
                said,
                "MCP connection failed (the server could not be reached)"
            ),
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("reclaimed with no server"),
        }
    }

    #[test]
    fn a_challenge_at_start_is_a_refused_key() {
        let at_start = |error| ClientInitializeError::TransportError {
            error: transport(error),
            context: "send initialize request".into(),
        };
        assert!(refused_at_start(&at_start(challenge())));
        assert!(!refused_at_start(&at_start(other())));
        assert!(!refused_at_start(&ClientInitializeError::ConnectionClosed(
            "closed".into()
        )));
    }

    #[test]
    fn a_challenge_in_a_call_is_a_refused_key() {
        assert!(refused_in_call(&ServiceError::TransportSend(transport(
            challenge()
        ))));
        assert!(!refused_in_call(&ServiceError::TransportSend(transport(
            other()
        ))));
        assert!(!refused_in_call(&ServiceError::TransportClosed));
    }

    #[test]
    fn a_refused_key_names_the_key_it_was_given() {
        let client = Client::new("http://127.0.0.1:9/mcp".into(), Some("k".into()));
        assert_eq!(
            client.failure(true, "lost").to_string(),
            "the server refused the access key"
        );
        let client = client.describe_key("the access key in /home/me/key".into());
        assert_eq!(
            client.failure(true, "lost").to_string(),
            "the server refused the access key in /home/me/key"
        );
        assert_eq!(client.failure(false, "lost"), Failure::Lost("lost".into()));
    }

    #[test]
    fn a_not_json_answer_keeps_the_reason() {
        assert_eq!(
            parse_answer("register", "{\"handle\": }"),
            Err(Failure::Lost(
                "register gave an answer that is not JSON: syntax error at line 1, column 12"
                    .into()
            ))
        );
        assert_eq!(
            parse_answer("register", "{\"handle\": \"a\""),
            Err(Failure::Lost(
                "register gave an answer that is not JSON: unexpected end of input at line 1, column 14"
                    .into()
            ))
        );
        assert_eq!(parse_answer("register", "{}"), Ok(json!({})));
    }

    /// Answers can hold tokens and recovery codes, so the reason never
    /// repeats any of the answer's text.
    #[test]
    fn a_not_json_answer_does_not_repeat_the_answer() {
        let marker = "act_marker7q3z";
        let answers = [
            marker.to_string(),
            format!("{{\"token\": \"{marker}\" x}}"),
            format!("{{\"token\": \"{marker}\\q\"}}"),
            format!("{{\"token\": {marker}}}"),
            format!("{{\"{marker}\": 1,}}"),
            format!("[\"{marker}\"] {marker}"),
        ];
        for answer in answers {
            let Err(Failure::Lost(reason)) = parse_answer("register", &answer) else {
                panic!("{answer:?} should not parse");
            };
            assert!(
                reason.starts_with("register gave an answer that is not JSON: "),
                "{reason}"
            );
            assert!(!reason.contains("marker"), "{reason}");
        }
    }
}
