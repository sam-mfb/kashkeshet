//! The MCP tools. Each tool checks the caller's token, calls into the rules,
//! and formats the result. No tool touches the database directly.

use crate::config::Limits;
use crate::format::{Format, Formatter};
use crate::rules::{
    self, Agent, ChannelQuery, NewChannel, Position, ReadQuery, RuleError, Rules, SearchQuery,
    WaitQuery,
};
use crate::tool_text;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolResult, ContentBlock, DiscoverResult, Implementation, InitializeRequestParams,
    InitializeResult, ServerCapabilities, ServerConfig,
};
use rmcp::{RoleServer, ServerHandler, service::RequestContext, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// The instructions sent to every agent that connects, unless the server is
/// started with others. Kept in their own file so they are easy to revise as
/// we watch how agents behave.
pub const DEFAULT_INSTRUCTIONS: &str = include_str!("../agent-instructions.md");

#[derive(Clone)]
pub struct ChatServer {
    rules: Rules,
    fmt: Formatter,
    instructions: Arc<str>,
    tool_router: ToolRouter<Self>,
}

impl ChatServer {
    /// `tools` comes from [`tools`], built once with the server's limits.
    pub fn new(
        rules: Rules,
        fmt: Formatter,
        instructions: Arc<str>,
        tools: ToolRouter<ChatServer>,
    ) -> ChatServer {
        ChatServer {
            rules,
            fmt,
            instructions,
            tool_router: tools,
        }
    }

    async fn operator_server_info(
        &self,
        mut info: ServerConfig,
    ) -> Result<ServerConfig, rmcp::ErrorData> {
        let current = self
            .rules
            .current_operator_instructions()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "could not read operator instructions");
                rmcp::ErrorData::internal_error("could not read operator instructions", None)
            })?;
        info.instructions = Some(
            self.fmt
                .with_operator_instructions(self.instructions.as_ref(), current.as_ref()),
        );
        Ok(info)
    }

    async fn me(&self, token: &Option<String>) -> rules::Result<Agent> {
        self.rules.authenticate(token.as_deref()).await
    }

    fn respond(&self, format: Format, out: rules::Result<String>) -> CallToolResult {
        match out {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(e) => {
                if e.code == rules::Code::Internal {
                    tracing::error!(error = %e, "tool call failed");
                }
                CallToolResult::error(vec![ContentBlock::text(self.fmt.error(format, &e))])
            }
        }
    }

    fn limit(&self, requested: Option<u32>) -> usize {
        self.rules.limits().limit(requested)
    }

    fn client_info(context: &RequestContext<RoleServer>) -> Option<rules::ClientInfo> {
        context.peer.peer_info().map(|info| rules::ClientInfo {
            name: info.client_info.name.clone(),
            version: info.client_info.version.clone(),
        })
    }

    /// The body cut an agent asked for; with none, bodies are cut only to fit
    /// the response cap.
    fn body_chars(requested: Option<u32>) -> Option<usize> {
        requested.map(|n| (n as usize).max(1))
    }
}

// ---- Parameters ----
//
// `token` is optional in the schema so that a call without it gets a helpful
// `unauthenticated` answer rather than a schema error.
//
// What agents are told about each tool and parameter is in
// `agent-tools.toml`, not here; `tools` adds it.

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReclaimParams {
    pub name: String,
    pub recovery_code: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterParams {
    pub name: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TokenOnly {
    pub token: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetDescriptionParams {
    pub token: Option<String>,
    pub text: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetHandoffNoteParams {
    pub token: Option<String>,
    pub text: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetHandoffNoteParams {
    pub token: Option<String>,
    pub offset: Option<u32>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAgentsParams {
    pub token: Option<String>,
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChannelsParams {
    pub token: Option<String>,
    pub query: Option<String>,
    pub member_only: Option<bool>,
    pub with_agent: Option<String>,
    pub after_id: Option<i64>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChannelParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMembersParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateChannelParams {
    pub token: Option<String>,
    pub name: String,
    pub open: bool,
    pub purpose: Option<String>,
    pub members: Option<Vec<String>>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddToChannelParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub agents: Vec<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChannelAgentParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub agent: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetPurposeParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub purpose: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetNoteParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub text: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNoteParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub offset: Option<u32>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub body: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadParams {
    pub token: Option<String>,
    pub channel_id: String,
    pub after_id: Option<i64>,
    pub before_id: Option<i64>,
    pub from_agent: Option<String>,
    pub limit: Option<u32>,
    pub max_body_chars: Option<u32>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMessageParams {
    pub token: Option<String>,
    pub message_id: i64,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    pub token: Option<String>,
    pub query: String,
    pub channel_id: Option<String>,
    pub from_agent: Option<String>,
    pub limit: Option<u32>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActivityParams {
    pub token: Option<String>,
    pub after_id: i64,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitParams {
    pub token: Option<String>,
    pub after_id: i64,
    pub channel_ids: Option<Vec<String>>,
    pub timeout_s: Option<u32>,
    pub limit: Option<u32>,
    pub max_body_chars: Option<u32>,
    pub format: Option<Format>,
}

/// The tools, described by `agent-tools.toml` with every limit filled in from
/// `limits`.
pub fn tools(limits: &Limits) -> ToolRouter<ChatServer> {
    with_limits(
        with_text(ChatServer::tool_router(), tool_text::AGENT),
        limits,
    )
}

/// `router`'s tools with their descriptions and their parameters' taken from
/// `texts`. The admin endpoint's tools use it too. A tool or parameter with
/// no text is left without; the tests check there are none.
pub(crate) fn with_text<S>(mut router: ToolRouter<S>, texts: &[tool_text::Tool]) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        let tool = &mut route.attr;
        let Some(text) = tool_text::find(texts, &tool.name) else {
            continue;
        };
        tool.description = Some(text.description.into());
        let schema = Arc::make_mut(&mut tool.input_schema);
        if let Some(Value::Object(properties)) = schema.get_mut("properties") {
            for (name, property) in properties.iter_mut() {
                if let (Some(text), Value::Object(property)) = (text.parameter(name), property) {
                    property.insert("description".into(), text.into());
                }
            }
        }
    }
    router
}

/// The tools in `router` with their parameters' names, to check against
/// their text.
#[cfg(test)]
pub(crate) fn offered<S>(router: &ToolRouter<S>) -> Vec<tool_text::Offered> {
    router
        .map
        .values()
        .map(|route| &route.attr)
        .map(|tool| tool_text::Offered {
            name: tool.name.to_string(),
            parameters: tool
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| properties.keys().cloned().collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// `router`'s tools with every limit in their descriptions filled in from
/// `limits`. The admin endpoint's tools use it too.
pub(crate) fn with_limits<S>(mut router: ToolRouter<S>, limits: &Limits) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        let tool = &mut route.attr;
        if let Some(text) = &tool.description {
            tool.description = Some(describe(text, limits).into());
        }
        let mut schema = Value::Object(tool.input_schema.as_ref().clone());
        describe_schema(&mut schema, limits);
        if let Value::Object(object) = schema {
            tool.input_schema = Arc::new(object);
        }
    }
    router
}

/// Fills in the limit placeholders in a tool or parameter description.
fn describe(text: &str, limits: &Limits) -> String {
    let values = [
        ("{default_limit}", limits.default_limit.to_string()),
        ("{max_limit}", limits.max_limit.to_string()),
        ("{max_body_size}", size(limits.max_body_bytes)),
        (
            "{max_channel_name_chars}",
            limits.max_channel_name_chars.to_string(),
        ),
        ("{max_purpose_chars}", limits.max_purpose_chars.to_string()),
        ("{max_note_chars}", limits.max_note_chars.to_string()),
        (
            "{max_operator_instructions_chars}",
            limits.max_operator_instructions_chars.to_string(),
        ),
        (
            "{max_handoff_note_chars}",
            limits.max_handoff_note_chars.to_string(),
        ),
        (
            "{max_description_chars}",
            limits.max_description_chars.to_string(),
        ),
        (
            "{default_wait_s}",
            limits.default_wait.as_secs().to_string(),
        ),
        ("{max_wait_s}", limits.max_wait.as_secs().to_string()),
        (
            "{max_agent_name_chars}",
            crate::config::MAX_AGENT_NAME_CHARS.to_string(),
        ),
    ];
    values.iter().fold(text.to_string(), |text, (name, value)| {
        text.replace(name, value)
    })
}

/// Fills in every `description` string found anywhere in a JSON schema.
fn describe_schema(schema: &mut Value, limits: &Limits) {
    match schema {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                match value {
                    Value::String(text) if key == "description" => {
                        *text = describe(text, limits);
                    }
                    _ => describe_schema(value, limits),
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| describe_schema(item, limits)),
        _ => {}
    }
}

/// A byte count as people say it: `8 KB` for whole kilobytes, else `1500 bytes`.
fn size(bytes: usize) -> String {
    if bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

#[tool_router]
impl ChatServer {
    // ---- Identity ----

    #[tool]
    async fn register(
        &self,
        Parameters(p): Parameters<RegisterParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .rules
            .register_with_client(Self::client_info(&context), p.name)
            .await
            .map(|r| self.fmt.registration(f, &r));
        self.respond(f, out)
    }

    #[tool]
    async fn reclaim(
        &self,
        Parameters(p): Parameters<ReclaimParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .rules
            .reclaim_with_client(p.name, p.recovery_code, Self::client_info(&context))
            .await
            .map(|r| self.fmt.reclaimed(f, &r));
        self.respond(f, out)
    }

    #[tool]
    async fn whoami(&self, Parameters(p): Parameters<TokenOnly>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let instructions = self.rules.current_operator_instructions().await?;
            Ok(self.fmt.whoami(f, &me.name, instructions.as_ref()))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn unregister(&self, Parameters(p): Parameters<TokenOnly>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            self.rules.unregister(&me).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_description(
        &self,
        Parameters(p): Parameters<SetDescriptionParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let agent = self.rules.set_description(&me, p.text).await?;
            Ok(self.fmt.description(f, &agent))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_handoff_note(
        &self,
        Parameters(p): Parameters<SetHandoffNoteParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            self.rules.set_handoff_note(&me, p.text).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn get_handoff_note(
        &self,
        Parameters(p): Parameters<GetHandoffNoteParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let note = self.rules.handoff_note(&me).await?;
            let offset = p.offset.unwrap_or(0) as usize;
            if offset > note.text.chars().count() {
                return Err(RuleError::invalid(
                    "offset is past the end of the handoff note",
                ));
            }
            Ok(self.fmt.handoff_note(f, &note, offset))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn list_agents(&self, Parameters(p): Parameters<ListAgentsParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let limit = self.limit(p.limit);
            let page = self
                .rules
                .list_agents(&me, p.query, limit, p.cursor)
                .await?;
            Ok(self.fmt.agents(f, &page))
        }
        .await;
        self.respond(f, out)
    }

    // ---- Channels ----

    #[tool]
    async fn list_channels(&self, Parameters(p): Parameters<ListChannelsParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let q = ChannelQuery {
                query: p.query,
                member_only: p.member_only.unwrap_or(false),
                with_agent: p.with_agent,
                after_id: p.after_id,
                limit: self.limit(p.limit),
                cursor: p.cursor,
            };
            let page = self.rules.list_channels(&me, q).await?;
            Ok(self
                .fmt
                .channels(f, &page, self.rules.limits().purpose_preview_chars))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn list_members(&self, Parameters(p): Parameters<ListMembersParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let limit = self.limit(p.limit);
            let members = self
                .rules
                .list_members(&me, p.channel_id, limit, p.cursor)
                .await?;
            Ok(self.fmt.members(f, &members))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn create_channel(
        &self,
        Parameters(p): Parameters<CreateChannelParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let new = NewChannel {
                name: p.name,
                open: p.open,
                purpose: p.purpose,
                members: p.members.unwrap_or_default(),
            };
            let channel = self.rules.create_channel(&me, new).await?;
            Ok(self.fmt.channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn join_channel(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let channel = self.rules.join_channel(&me, p.channel_id).await?;
            Ok(self.fmt.channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn leave_channel(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            self.rules.leave_channel(&me, p.channel_id).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn add_to_channel(
        &self,
        Parameters(p): Parameters<AddToChannelParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let members = self
                .rules
                .add_to_channel(&me, p.channel_id, p.agents)
                .await?;
            Ok(self.fmt.members(f, &members))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn remove_from_channel(
        &self,
        Parameters(p): Parameters<ChannelAgentParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let members = self
                .rules
                .remove_from_channel(&me, p.channel_id, p.agent)
                .await?;
            Ok(self.fmt.members(f, &members))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn transfer_owner(
        &self,
        Parameters(p): Parameters<ChannelAgentParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let channel = self
                .rules
                .transfer_owner(&me, p.channel_id, p.agent)
                .await?;
            Ok(self.fmt.channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn hide_channel(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            self.rules.hide_channel(&me, p.channel_id).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_purpose(&self, Parameters(p): Parameters<SetPurposeParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let channel = self.rules.set_purpose(&me, p.channel_id, p.purpose).await?;
            Ok(self.fmt.channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_note(&self, Parameters(p): Parameters<SetNoteParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            self.rules.set_note(&me, p.channel_id, p.text).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn get_note(&self, Parameters(p): Parameters<GetNoteParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let note = self.rules.get_note(&me, p.channel_id).await?;
            let offset = p.offset.unwrap_or(0) as usize;
            if offset > note.text.chars().count() {
                return Err(RuleError::invalid("offset is past the end of the note"));
            }
            Ok(self.fmt.note(f, &note, offset))
        }
        .await;
        self.respond(f, out)
    }

    // ---- Messages ----

    #[tool]
    async fn send(&self, Parameters(p): Parameters<SendParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let message = self.rules.send(&me, p.channel_id, p.body).await?;
            Ok(self.fmt.sent(f, &message))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn read(&self, Parameters(p): Parameters<ReadParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let position = match (p.after_id, p.before_id) {
                (Some(_), Some(_)) => {
                    return Err(RuleError::invalid(
                        "give `after_id` or `before_id`, not both",
                    ));
                }
                (Some(a), None) => Position::After(a),
                (None, Some(b)) => Position::Before(b),
                (None, None) => Position::Latest,
            };
            let q = ReadQuery {
                channel_id: p.channel_id,
                position,
                from_agent: p.from_agent,
                limit: self.limit(p.limit),
            };
            let page = self.rules.read(&me, q).await?;
            Ok(self
                .fmt
                .read(f, &page, position, Self::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn get_message(&self, Parameters(p): Parameters<GetMessageParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let message = self.rules.get_message(&me, p.message_id).await?;
            Ok(self.fmt.message(f, &message))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn search_messages(&self, Parameters(p): Parameters<SearchParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let q = SearchQuery {
                query: p.query,
                channel_id: p.channel_id,
                from_agent: p.from_agent,
                limit: self.limit(p.limit),
            };
            let hits = self.rules.search(&me, q).await?;
            Ok(self.fmt.search(f, &hits))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn activity(&self, Parameters(p): Parameters<ActivityParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let rows = self.rules.activity(&me, p.after_id).await?;
            Ok(self.fmt.activity(f, &rows))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn wait_for_messages(
        &self,
        Parameters(p): Parameters<WaitParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let me = self.me(&p.token).await?;
            let limits = self.rules.limits();
            let timeout = match p.timeout_s {
                Some(s) => Duration::from_secs(u64::from(s)).min(limits.max_wait),
                None => limits.default_wait,
            };
            let q = WaitQuery {
                after_id: p.after_id,
                channel_ids: p.channel_ids,
                timeout,
                limit: self.limit(p.limit),
            };
            let result = tokio::select! {
                biased;
                () = context.ct.cancelled() => return Err(RuleError::invalid("wait cancelled")),
                result = self.rules.wait_for_messages(&me, q) => result?,
            };
            Ok(self
                .fmt
                .waited(f, &result, Self::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, out)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ChatServer {
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, rmcp::ErrorData> {
        context.peer.set_peer_info(request.clone());
        self.operator_server_info(self.negotiate_initialize(&request)?)
            .await
    }
    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, rmcp::ErrorData> {
        let info = self.operator_server_info(self.get_info()).await?;
        Ok(DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            info,
        ))
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("kashkeshet", env!("CARGO_PKG_VERSION"))
                    .with_title("Kashkeshet (kkt)"),
            )
            .with_instructions(self.instructions.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn odd_limits() -> Limits {
        Limits {
            default_limit: 7,
            max_limit: 42,
            max_body_bytes: 3000,
            max_channel_name_chars: 33,
            max_purpose_chars: 77,
            max_description_chars: 55,
            default_wait: Duration::from_secs(5),
            max_wait: Duration::from_secs(9),
            ..Limits::DEFAULT
        }
    }

    #[test]
    fn describe_fills_every_placeholder() {
        let text = "{default_limit} {max_limit} {max_body_size} \
            {max_channel_name_chars} {max_purpose_chars} {max_description_chars} \
            {default_wait_s} {max_wait_s}";
        assert_eq!(
            describe(text, &odd_limits()),
            "7 42 3000 bytes 33 77 55 5 9"
        );
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size(8 * 1024), "8 KB");
        assert_eq!(size(1500), "1500 bytes");
    }

    #[test]
    fn every_tool_and_parameter_has_text_in_agent_tools_toml() {
        let wrong = tool_text::mismatches(tool_text::AGENT, &offered(&ChatServer::tool_router()));
        assert!(wrong.is_empty(), "agent-tools.toml:\n{}", wrong.join("\n"));
    }

    #[test]
    fn no_placeholder_is_left_unfilled() {
        for tool in tools(&odd_limits()).list_all() {
            let text = serde_json::to_string(&tool).expect("tool as JSON");
            assert!(!has_placeholder(&text), "{text}");
        }
    }

    /// True if `text` holds `{word}`, where the word is lowercase letters and underscores.
    fn has_placeholder(text: &str) -> bool {
        text.split('{').skip(1).any(|rest| {
            rest.split_once('}').is_some_and(|(word, _)| {
                !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            })
        })
    }
}
