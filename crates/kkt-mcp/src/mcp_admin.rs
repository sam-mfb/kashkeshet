//! The admin endpoint's MCP server, at `/mcp-admin`: the tools an admin
//! agent sees, separate from the agents' tools in `lib.rs`. Only callers
//! with the admin key reach it (`kkt-server` puts the admin key check from
//! `kkt-admission` in front of it), so the tools take no token.
//! Each parses its parameters, calls `rules` and formats the result.
//!
//! The read tools work with any admin key. The change tools act as
//! [`Actor::Admin`] under the name tied to the key, or as
//! [`Actor::UnnamedAdmin`] when the key has none, which `rules` refuse. Issuing recovery codes and rotating keys have no tool: they
//! stay with the human. There is no waiting: `kkt watch --admin` waits from
//! a shell by calling `new_messages`.

use crate::mcp_tools::{self, with_limits, with_text};
use crate::mcp_transport::Transport;
use crate::tool_text;
use axum::Router;
use kkt_config::{Limits, MCP_ADMIN_PATH};
use kkt_format::{Format, Formatter};
use kkt_names::AdminName;
use kkt_rules::{self as rules, Actor, ChannelChange, LaneSave, NewChannel, ReadQuery, Rules};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::time::Duration;

/// What the admin endpoint tells every admin agent that connects.
const ADMIN_INSTRUCTIONS: &str = include_str!("../admin-instructions.md");

#[derive(Clone)]
pub(crate) struct AdminServer {
    rules: Rules,
    fmt: Formatter,
    /// The name tied to the admin key, if it was given one. Without it the
    /// change tools refuse.
    name: Option<AdminName>,
    /// The kkt version given in the MCP server info.
    server_version: &'static str,
    tool_router: ToolRouter<Self>,
}

/// The admin tools, described by `admin-tools.toml` with every limit filled
/// in from `limits`.
pub(crate) fn tools(limits: &Limits) -> ToolRouter<AdminServer> {
    with_limits(
        with_text(AdminServer::tool_router(), tool_text::ADMIN),
        limits,
    )
}

/// The admin endpoint: a router serving the admin tools at `/mcp-admin`
/// through `transport`, set as `/mcp`'s is. The tools are described with the
/// limits `rules` holds. `name` is the name tied to the admin key, if it has
/// one. `server_version` is the kkt version given in the MCP server info.
/// Who may call it is for the caller to decide.
pub fn endpoint(
    rules: Rules,
    fmt: Formatter,
    name: Option<AdminName>,
    server_version: &'static str,
    transport: &Transport,
) -> Router {
    let tools = tools(rules.limits());
    transport.serve(
        MCP_ADMIN_PATH,
        AdminServer::new(rules, fmt, tools, name, server_version),
    )
}

impl AdminServer {
    /// `tools` comes from [`tools`], built once with the server's limits.
    /// `name` is the name tied to the admin key, if it has one.
    fn new(
        rules: Rules,
        fmt: Formatter,
        tools: ToolRouter<AdminServer>,
        name: Option<AdminName>,
        server_version: &'static str,
    ) -> AdminServer {
        AdminServer {
            rules,
            fmt,
            name,
            server_version,
            tool_router: tools,
        }
    }

    /// Who makes a change: the admin agent, under its key's name, or an
    /// unnamed admin, whose changes `rules` refuse.
    fn actor(&self) -> Actor {
        self.name.clone().map_or(Actor::UnnamedAdmin, Actor::Admin)
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
            .change_channel(self.actor(), channel_id.clone(), change)
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
        mcp_tools::respond(&self.fmt, format, out, |e| {
            tracing::error!(error = %e, "admin tool call failed");
        })
    }

    fn limit(&self, requested: Option<u32>) -> usize {
        self.rules.limits().limit(requested)
    }
}

// ---- Parameters ----
//
// What admin agents are told about each tool and parameter is in
// `admin-tools.toml`, not here; `tools` adds it.

#[derive(Debug, Deserialize, JsonSchema)]
struct FormatOnly {
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListChannelsParams {
    query: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListAgentsParams {
    query: Option<String>,
    lane: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AgentParams {
    handle: String,
    offset: Option<u32>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ChannelParams {
    channel_id: String,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListMemoriesParams {
    handle: String,
    limit: Option<u32>,
    cursor: Option<String>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecallParams {
    handle: String,
    name: String,
    offset: Option<u32>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListMembersParams {
    channel_id: String,
    limit: Option<u32>,
    cursor: Option<String>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetNoteParams {
    channel_id: String,
    offset: Option<u32>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadParams {
    channel_id: String,
    after_id: Option<i64>,
    before_id: Option<i64>,
    from_agent: Option<String>,
    limit: Option<u32>,
    max_body_chars: Option<u32>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetMessageParams {
    message_id: i64,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ActivityParams {
    after_id: i64,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NewMessagesParams {
    after_id: i64,
    channel_ids: Option<Vec<String>>,
    limit: Option<u32>,
    max_body_chars: Option<u32>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateChannelParams {
    name: String,
    open: bool,
    purpose: Option<String>,
    members: Option<Vec<String>>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetPurposeParams {
    channel_id: String,
    purpose: String,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetOpenParams {
    channel_id: String,
    open: bool,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ChannelAgentParams {
    channel_id: String,
    handle: String,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HandleParams {
    handle: String,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StaleParams {
    idle_hours: u32,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HideStaleParams {
    idle_hours: u32,
    handles: Vec<String>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AnnouncementParams {
    text: String,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListLanesParams {
    limit: Option<u32>,
    cursor: Option<String>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetLaneParams {
    lane: String,
    offset: Option<u32>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetLaneParams {
    lane: String,
    description: String,
    text: String,
    format: Option<Format>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LaneParams {
    lane: String,
    format: Option<Format>,
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
            let offset = mcp_tools::offset(
                p.offset,
                note.text.chars().count(),
                "offset is past the end of the note",
            )?;
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
            let position = mcp_tools::position(p.after_id, p.before_id)?;
            let q = ReadQuery {
                channel_id: p.channel_id,
                position,
                from_agent: p.from_agent,
                limit: self.limit(p.limit),
            };
            let page = self.rules.admin_read(q).await?;
            Ok(self
                .fmt
                .read(f, &page, position, mcp_tools::body_chars(p.max_body_chars)))
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
                .waited(f, &result, mcp_tools::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, out)
    }

    // ---- Agents and the server announcement ----

    #[tool]
    async fn list_agents(&self, Parameters(p): Parameters<ListAgentsParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let page = self
                .rules
                .admin_list_agents(p.query, p.lane, self.limit(p.limit), p.cursor)
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
            let view = self.rules.admin_agent(p.handle).await?;
            let offset = mcp_tools::offset(
                p.offset,
                view.channels.len(),
                "offset is past the last channel",
            )?;
            Ok(self.fmt.admin_agent(f, &view, offset))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn list_memories(&self, Parameters(p): Parameters<ListMemoriesParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let list = self
                .rules
                .admin_list_memories(p.handle, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self.fmt.memory_list(f, &list))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn recall(&self, Parameters(p): Parameters<RecallParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let entry = self.rules.admin_recall(p.handle, p.name).await?;
            let offset = mcp_tools::offset(
                p.offset,
                entry.text.chars().count(),
                "offset is past the end of the entry",
            )?;
            Ok(self.fmt.memory_entry(f, &entry, offset))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn get_server_announcement(
        &self,
        Parameters(p): Parameters<FormatOnly>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let current = self.rules.current_server_announcement().await?;
            Ok(self.fmt.server_announcement(f, current.as_ref()))
        }
        .await;
        self.respond(f, out)
    }

    // ---- Lanes ----

    #[tool]
    async fn list_lanes(&self, Parameters(p): Parameters<ListLanesParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let page = self.rules.lanes_page(self.limit(p.limit), p.cursor).await?;
            Ok(self.fmt.lanes(f, &page))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn get_lane(&self, Parameters(p): Parameters<GetLaneParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let lane = self.rules.lane(p.lane).await?;
            let offset = mcp_tools::offset(
                p.offset,
                lane.current.text.chars().count(),
                "offset is past the end of the lane's instructions",
            )?;
            Ok(self.fmt.lane(f, &lane, offset))
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
                .create_channel_as_operator(self.actor(), new)
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
                .set_purpose(self.actor(), p.channel_id.clone(), p.purpose)
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
            .change(f, p.channel_id, ChannelChange::AddMember(p.handle))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn remove_member(&self, Parameters(p): Parameters<ChannelAgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::RemoveMember(p.handle))
            .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_owner(&self, Parameters(p): Parameters<ChannelAgentParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = self
            .change(f, p.channel_id, ChannelChange::SetOwner(Some(p.handle)))
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
    async fn hide_agent(&self, Parameters(p): Parameters<HandleParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            self.rules.hide_agent(self.actor(), p.handle).await?;
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
                .hide_stale_agents(self.actor(), Self::idle(p.idle_hours), p.handles)
                .await?;
            Ok(self.fmt.stale_hidden(f, &outcome))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_server_announcement(
        &self,
        Parameters(p): Parameters<AnnouncementParams>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let saved = self
                .rules
                .save_server_announcement(self.actor(), p.text)
                .await?;
            Ok(self.fmt.server_announcement(f, Some(&saved)))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn set_lane(&self, Parameters(p): Parameters<SetLaneParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let save = LaneSave {
                name: p.lane,
                description: p.description,
                text: p.text,
            };
            let lane = self.rules.save_lane(self.actor(), save).await?;
            Ok(self.fmt.lane(f, &lane, 0))
        }
        .await;
        self.respond(f, out)
    }

    #[tool]
    async fn retire_lane(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let out = async {
            let lane = self.rules.retire_lane(self.actor(), p.lane).await?;
            Ok(self.fmt.lane(f, &lane, 0))
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
                Implementation::new("kashkeshet-admin", self.server_version)
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
            &mcp_tools::offered(&AdminServer::tool_router()),
        );
        assert!(wrong.is_empty(), "admin-tools.toml:\n{}", wrong.join("\n"));
    }
}
