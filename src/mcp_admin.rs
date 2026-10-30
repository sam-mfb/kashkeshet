//! The admin endpoint's MCP server, at `/mcp-admin`: the tools an admin
//! agent sees, separate from the agents' tools in `mcp.rs`. Only callers
//! with the admin key reach it (see `app.rs`), so the tools take no token.
//! Each parses its parameters, calls `rules` and formats the result.
//!
//! The tools only read. There is no waiting: `kkt watch --admin` waits
//! from a shell by calling `new_messages`.

use crate::config::Limits;
use crate::format::{Format, Formatter};
use crate::mcp::{with_limits, with_text};
use crate::rules::{self, Position, ReadQuery, RuleError, Rules};
use crate::tool_text;
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

/// The admin tools, described by `admin-tools.toml` with every limit filled
/// in from `limits`.
pub fn tools(limits: &Limits) -> ToolRouter<AdminServer> {
    with_limits(
        with_text(AdminServer::tool_router(), tool_text::ADMIN),
        limits,
    )
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
// What admin agents are told about each tool and parameter is in
// `admin-tools.toml`, not here; `tools` adds it.

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FormatOnly {
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChannelsParams {
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAgentsParams {
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentParams {
    pub name: String,
    pub offset: Option<u32>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChannelParams {
    pub channel_id: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMembersParams {
    pub channel_id: String,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNoteParams {
    pub channel_id: String,
    pub offset: Option<u32>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadParams {
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
    pub message_id: i64,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActivityParams {
    pub after_id: i64,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewMessagesParams {
    pub after_id: i64,
    pub channel_ids: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub max_body_chars: Option<u32>,
    pub format: Option<Format>,
}

#[tool_router]
impl AdminServer {
    // ---- Channels ----

    #[tool]
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

    #[tool]
    async fn get_channel(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let channel = self.rules.admin_channel(p.channel_id).await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
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

    #[tool]
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

    #[tool]
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

    #[tool]
    async fn get_message(&self, Parameters(p): Parameters<GetMessageParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let message = self.rules.admin_message(p.message_id).await?;
            Ok(self.fmt.message(f, &message))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn activity(&self, Parameters(p): Parameters<ActivityParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let rows = self.rules.admin_activity(p.after_id).await?;
            Ok(self.fmt.activity(f, &rows))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
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

    #[tool]
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

    #[tool]
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

    #[tool]
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

    #[tool]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_and_parameter_has_text_in_admin_tools_toml() {
        let wrong = tool_text::mismatches(
            tool_text::ADMIN,
            &crate::mcp::offered(&AdminServer::tool_router()),
        );
        assert!(wrong.is_empty(), "admin-tools.toml:\n{}", wrong.join("\n"));
    }
}
