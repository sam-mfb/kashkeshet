//! The admin endpoint's MCP server, at `/mcp-admin`: the tools an admin
//! agent sees, separate from the agents' tools in `mcp.rs`. Only callers
//! with the admin key reach it (see `app.rs`), so the tools take no token.
//! Each parses its parameters, calls `rules` and formats the result.
//!
//! The read tools work with any admin key. The change tools act as
//! [`Actor::Admin`] under the name tied to the key, and refuse when the key
//! has none. Issuing recovery codes and rotating keys have no tool: they
//! stay with the human. There is no waiting: `kkt watch --admin` waits from
//! a shell by calling `new_messages`.

use crate::config::Limits;
use crate::format::{Format, Formatter};
use crate::mcp::{with_limits, with_text};
use crate::names::AdminName;
use crate::rules::{
    self, Actor, ChannelChange, Code, NewChannel, Position, ReadQuery, RuleError, Rules,
};
use crate::tool_text;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::time::Duration;

/// What the admin endpoint tells every admin agent that connects.
pub const ADMIN_INSTRUCTIONS: &str = include_str!("../admin-instructions.md");

#[derive(Clone)]
pub struct AdminServer {
    rules: Rules,
    fmt: Formatter,
    /// The name tied to the admin key, if it was given one. Without it the
    /// change tools refuse.
    name: Option<AdminName>,
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
    /// `name` is the name tied to the admin key, if it has one.
    pub fn new(
        rules: Rules,
        fmt: Formatter,
        tools: ToolRouter<AdminServer>,
        name: Option<AdminName>,
    ) -> AdminServer {
        AdminServer {
            rules,
            fmt,
            name,
            tool_router: tools,
        }
    }

    /// Who makes a change: the admin agent, under its key's name. A key
    /// made without a name may only read.
    fn actor(&self) -> rules::Result<Actor> {
        self.name.clone().map(Actor::Admin).ok_or_else(|| {
            RuleError::new(
                Code::Unauthenticated,
                "this admin key has no name, so it may only read. The operator names it by \
                 making the key again with `kkt key create --admin --force --name <name>` \
                 and restarting the server.",
            )
        })
    }

    /// Makes `change` to `channel_id` as the admin agent, then shows the
    /// channel as `get_channel` does.
    async fn change(
        &self,
        f: Format,
        channel_id: String,
        change: ChannelChange,
    ) -> rules::Result<String> {
        self.rules
            .change_channel(self.actor()?, channel_id.clone(), change)
            .await?;
        let channel = self.rules.admin_channel(channel_id).await?;
        Ok(self.fmt.admin_channel(f, &channel))
    }

    /// `hours` as a quiet period for stale agents. Zero is refused by
    /// `rules`.
    fn idle(hours: u32) -> Duration {
        Duration::from_secs(u64::from(hours) * 3600)
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateChannelParams {
    pub name: String,
    pub open: bool,
    pub purpose: Option<String>,
    pub members: Option<Vec<String>>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetPurposeParams {
    pub channel_id: String,
    pub purpose: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetOpenParams {
    pub channel_id: String,
    pub open: bool,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChannelAgentParams {
    pub channel_id: String,
    pub name: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NameParams {
    pub name: String,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StaleParams {
    pub idle_hours: u32,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HideStaleParams {
    pub idle_hours: u32,
    pub names: Vec<String>,
    pub format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InstructionsParams {
    pub text: String,
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

    // ---- Changes, as the operator via this key's name ----

    #[tool]
    async fn create_channel(
        &self,
        Parameters(p): Parameters<CreateChannelParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let new = NewChannel {
                name: p.name,
                purpose: p.purpose,
                open: p.open,
                members: p.members.unwrap_or_default(),
            };
            let created = self
                .rules
                .create_channel_as_operator(self.actor()?, new)
                .await?;
            let channel = self.rules.admin_channel(created.id).await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_purpose(&self, Parameters(p): Parameters<SetPurposeParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            self.rules
                .set_purpose(self.actor()?, p.channel_id.clone(), p.purpose)
                .await?;
            let channel = self.rules.admin_channel(p.channel_id).await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_open(&self, Parameters(p): Parameters<SetOpenParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::SetOpen(p.open))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn add_member(&self, Parameters(p): Parameters<ChannelAgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::AddMember(p.name))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn remove_member(&self, Parameters(p): Parameters<ChannelAgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::RemoveMember(p.name))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_owner(&self, Parameters(p): Parameters<ChannelAgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::SetOwner(Some(p.name)))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn clear_owner(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::SetOwner(None))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn hide_channel(&self, Parameters(p): Parameters<ChannelParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self.change(f, p.channel_id, ChannelChange::Hide).await;
        self.respond(f, out)
    }

    #[tool]
    async fn hide_agent(&self, Parameters(p): Parameters<NameParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            self.rules.hide_agent(self.actor()?, p.name).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn list_stale_agents(&self, Parameters(p): Parameters<StaleParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let stale = self.rules.stale_agents(Self::idle(p.idle_hours)).await?;
            Ok(self.fmt.stale_agents(f, p.idle_hours, &stale))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn hide_stale_agents(
        &self,
        Parameters(p): Parameters<HideStaleParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let outcome = self
                .rules
                .hide_stale_agents(self.actor()?, Self::idle(p.idle_hours), p.names)
                .await?;
            Ok(self.fmt.stale_hidden(f, &outcome))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_operator_instructions(
        &self,
        Parameters(p): Parameters<InstructionsParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let saved = self
                .rules
                .save_operator_instructions(self.actor()?, p.text)
                .await?;
            Ok(self.fmt.operator_instructions(f, Some(&saved)))
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
