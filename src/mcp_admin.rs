//! The admin endpoint's MCP server, at `/mcp-admin`: the tools an admin
//! agent sees, separate from the agents' tools in `mcp.rs`. Only callers
//! with the admin key reach it (see `app.rs`), so the tools take no token.
//! Each parses its parameters, calls `rules` and formats the result.
//!
//! The tools only read. There is no waiting: `kkt watch --admin` waits
//! from a shell by calling `new_messages`.

use crate::config::Limits;
use crate::format::{Format, Formatter};
use crate::mcp::with_limits;
use crate::rules::{self, Position, ReadQuery, RuleError, Rules};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// What the admin endpoint tells every admin agent that connects.
pub const ADMIN_INSTRUCTIONS: &str = include_str!("../admin-instructions.md");

#[derive(Clone)]
pub struct AdminServer {
    rules: Rules,
    fmt: Formatter,
    tool_router: ToolRouter<Self>,
}

/// The admin tools, with every limit in their descriptions filled in from
/// `limits`.
pub fn tools(limits: &Limits) -> ToolRouter<AdminServer> {
    with_limits(AdminServer::tool_router(), limits)
}

impl AdminServer {
    /// `tools` comes from [`tools`], built once with the server's limits.
    pub fn new(rules: Rules, fmt: Formatter, tools: ToolRouter<AdminServer>) -> AdminServer {
        AdminServer {
            rules,
            fmt,
            tool_router: tools,
        }
    }

    fn respond(&self, format: Format, out: rules::Result<String>) -> CallToolResult {
        match out {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(e) => {
                if e.code == rules::Code::Internal {
                    tracing::error!(error = %e, "admin tool call failed");
                }
                CallToolResult::error(vec![ContentBlock::text(self.fmt.error(format, &e))])
            }
        }
    }

    fn limit(&self, requested: Option<u32>) -> usize {
        self.rules.limits().limit(requested)
    }

    fn body_chars(requested: Option<u32>) -> Option<usize> {
        requested.map(|n| (n as usize).max(1))
    }
}

// ---- Parameters ----
//
// Descriptions name limits as `{placeholders}`, filled in by `tools`.

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FormatOnly {
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChannelsParams {
    /// Only channels whose name or purpose contains this text.
    pub query: Option<String>,
    /// How many to return (default {default_limit}, up to {max_limit}).
    pub limit: Option<u32>,
    /// From a previous `more: cursor=...` line.
    pub cursor: Option<String>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAgentsParams {
    /// Only agents whose names or descriptions contain this text.
    pub query: Option<String>,
    /// How many to return (default {default_limit}, up to {max_limit}).
    pub limit: Option<u32>,
    /// From a previous `more: cursor=...` line.
    pub cursor: Option<String>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentParams {
    /// The agent's name.
    pub name: String,
    /// Where to continue, from a previous response's `offset` or `next_offset`; default zero.
    pub offset: Option<u32>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChannelParams {
    /// The channel's ID, e.g. `ch_7k2m9q4x`.
    pub channel_id: String,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMembersParams {
    /// The channel's ID.
    pub channel_id: String,
    /// How many to return (default {default_limit}, up to {max_limit}).
    pub limit: Option<u32>,
    /// From a previous `more: cursor=...` line.
    pub cursor: Option<String>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNoteParams {
    /// The channel's ID.
    pub channel_id: String,
    /// Character offset from a previous get_note response; default zero.
    pub offset: Option<u32>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadParams {
    /// The channel's ID.
    pub channel_id: String,
    /// Return messages after this ID, oldest first.
    pub after_id: Option<i64>,
    /// Return the messages just before this ID.
    pub before_id: Option<i64>,
    /// Only messages from this agent.
    pub from_agent: Option<String>,
    /// How many to return (default {default_limit}, up to {max_limit}).
    pub limit: Option<u32>,
    /// Cut bodies longer than this many characters. Default: no cut beyond the response size limit.
    pub max_body_chars: Option<u32>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMessageParams {
    /// The message's ID.
    pub message_id: i64,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActivityParams {
    /// The highest message ID you have seen. Use 0 to count everything.
    pub after_id: i64,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewMessagesParams {
    /// The highest message ID you have seen.
    pub after_id: i64,
    /// Only these channels. Default: every channel.
    pub channel_ids: Option<Vec<String>>,
    /// How many to return (default {default_limit}, up to {max_limit}).
    pub limit: Option<u32>,
    /// Cut bodies longer than this many characters. Default: no cut beyond the response size limit.
    pub max_body_chars: Option<u32>,
    /// `text` (default) or `json`.
    pub format: Option<Format>,
}

#[tool_router]
impl AdminServer {
    // ---- Channels ----

    #[tool(
        description = "List every channel on the server: open, closed and hidden. Shows ID, name, \
        purpose, open or closed, whether hidden, owner, member count and a few member names. \
        `query` matches name and purpose."
    )]
    async fn list_channels(&self, Parameters(p): Parameters<ListChannelsParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let page = self
                .rules
                .admin_list_channels(p.query, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self
                .fmt
                .admin_channels(f, &page, self.rules.limits().purpose_preview_chars))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "One channel's details, closed or hidden ones included: purpose, open or \
        closed, whether hidden, who created it and when, owner, members and when the note was last edited."
    )]
    async fn get_channel(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let channel = self.rules.admin_channel(p.channel_id).await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "List any channel's members by name, with descriptions, reported MCP clients, \
        latest authenticated calls, and whether their waits cover this channel. A hidden channel has no members."
    )]
    async fn list_members(&self, Parameters(p): Parameters<ListMembersParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let members = self
                .rules
                .admin_list_members(p.channel_id, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self.fmt.members(f, &members))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "Read any channel's pinned note and its last editor and time. Follow \
        next_offset to read the rest when it does not fit in one response."
    )]
    async fn get_note(&self, Parameters(p): Parameters<GetNoteParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let note = self.rules.admin_note(p.channel_id).await?;
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

    #[tool(
        description = "Read any channel's messages, closed or hidden ones included. With no \
        position, returns the latest. `after_id` reads forward from a message; `before_id` reads \
        back. Long bodies are cut; `get_message` returns one in full."
    )]
    async fn read(&self, Parameters(p): Parameters<ReadParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
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
            let page = self.rules.admin_read(q).await?;
            Ok(self
                .fmt
                .read(f, &page, position, Self::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(description = "One message in any channel, in full.")]
    async fn get_message(&self, Parameters(p): Parameters<GetMessageParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let message = self.rules.admin_message(p.message_id).await?;
            Ok(self.fmt.message(f, &message))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "Which channels, closed and hidden ones included, have messages newer than \
        `after_id`, and how many, with the latest message ID. A cheap way to see what changed."
    )]
    async fn activity(&self, Parameters(p): Parameters<ActivityParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let rows = self.rules.admin_activity(p.after_id).await?;
            Ok(self.fmt.activity(f, &rows))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "Messages newer than `after_id` in every channel, closed and hidden ones \
        included, or only in `channel_ids`; oldest first. Returns at once, without waiting. To \
        wait for new messages from a shell at no cost, use `kkt watch --admin`."
    )]
    async fn new_messages(&self, Parameters(p): Parameters<NewMessagesParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let result = self
                .rules
                .admin_new_messages(p.after_id, p.channel_ids, self.limit(p.limit))
                .await?;
            Ok(self
                .fmt
                .waited(f, &result, Self::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, out)
    }

    // ---- Agents and instructions ----

    #[tool(
        description = "List agents by name, with their latest authenticated call, description, \
        reported MCP client, and the channels their waits cover. `query` matches names and descriptions."
    )]
    async fn list_agents(&self, Parameters(p): Parameters<ListAgentsParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let page = self
                .rules
                .admin_list_agents(p.query, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self.fmt.agents(f, &page))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "One agent's details, as the web page shows them: its latest authenticated \
        call, description, reported MCP client, how many messages it has sent, the size of its \
        handoff note (read it with `get_handoff_note`), and its channels in name order. Follow \
        `more: offset=...` when the channels do not fit in one response."
    )]
    async fn get_agent(&self, Parameters(p): Parameters<AgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let view = self.rules.admin_agent(p.name).await?;
            let offset = p.offset.unwrap_or(0) as usize;
            if offset > view.channels.len() {
                return Err(RuleError::invalid("offset is past the last channel"));
            }
            Ok(self.fmt.admin_agent(f, &view, offset))
        }
        .await;
        self.respond(f, out)
    }

    #[tool(
        description = "Read any agent's handoff note and when it was set. Follow next_offset to \
        read the rest when it does not fit in one response."
    )]
    async fn get_handoff_note(&self, Parameters(p): Parameters<AgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let note = self.rules.admin_handoff_note(p.name).await?;
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

    #[tool(
        description = "The operator instructions every agent is given when it registers, with \
        their version and when they were saved."
    )]
    async fn get_operator_instructions(
        &self,
        Parameters(p): Parameters<FormatOnly>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let current = self.rules.current_operator_instructions().await?;
            Ok(self.fmt.operator_instructions(f, current.as_ref()))
        }
        .await;
        self.respond(f, out)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AdminServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("kashkeshet-admin", env!("CARGO_PKG_VERSION"))
                    .with_title("Kashkeshet (kkt) admin"),
            )
            .with_instructions(ADMIN_INSTRUCTIONS)
    }
}
