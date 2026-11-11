//! The admin endpoint's MCP server, at `/mcp-admin`: the tools an admin
//! agent sees, separate from the agents' tools in `lib.rs`. Only callers
//! with the admin key reach it (`kkt-server` puts the admin key check from
//! `kkt-admission` in front of it), so the tools take no token.
//! Each parses its parameters, calls `rules` and formats the result.
//!
//! Every tool acts as the [`Operator`] that [`Identify::operator`] makes of
//! the call, in the swarm it names; in key mode, the operator the admin key
//! makes ([`KeyMode::admin_operator`](kkt_identity::KeyMode::admin_operator)): one
//! that may read, and change too when the key has a name, recorded and
//! announced under that name. `rules` check those permissions; this
//! endpoint words their refusals for an admin agent (`word`). Issuing
//! recovery codes and rotating keys have no tool: they stay with the human.
//! There is no waiting: `kkt watch --admin` waits from a shell by calling
//! `new_messages`.

use crate::mcp_tools::{self, with_limits, with_text};
use crate::mcp_transport::Transport;
use crate::principal;
use crate::tool_text;
use axum::Router;
use kkt_config::{Limits, MCP_ADMIN_PATH};
use kkt_identity::Identify;
use kkt_mcp_formatter::{Format, Formatter};
use kkt_rules::{
    self as rules, Actor, Audit, ChannelChange, LaneSave, NewChannel, Operator, Permission,
    ReadQuery, Refusal, RuleError, Rules,
};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// What the admin endpoint tells every admin agent that connects.
const ADMIN_INSTRUCTIONS: &str = include_str!("../admin-instructions.md");

#[derive(Clone)]
pub(crate) struct AdminServer {
    /// The endpoint's rules. A call uses the rules in its swarm, from
    /// [`AdminServer::call`]; these give only the limits.
    rules: Rules,
    /// Who each call comes from: the operator, and in which swarm.
    identify: Arc<dyn Identify>,
    fmt: Formatter,
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
/// through `transport`, set as `/mcp`'s is. Every call is identified by
/// `identify` as an operator in a swarm, and acts there. The tools are
/// described with the limits `rules` holds. `server_version` is the kkt
/// version given in the MCP server info. Who may call it is for admission,
/// in front of it, to decide; it leaves the `Principal` on the request.
pub fn endpoint(
    rules: Rules,
    identify: Arc<dyn Identify>,
    fmt: Formatter,
    server_version: &'static str,
    transport: &Transport,
) -> Router {
    let tools = tools(rules.limits());
    transport.serve(
        MCP_ADMIN_PATH,
        AdminServer::new(rules, identify, fmt, tools, server_version),
    )
}

impl AdminServer {
    /// `tools` comes from [`tools`], built once with the server's limits.
    fn new(
        rules: Rules,
        identify: Arc<dyn Identify>,
        fmt: Formatter,
        tools: ToolRouter<AdminServer>,
        server_version: &'static str,
    ) -> AdminServer {
        AdminServer {
            rules,
            identify,
            fmt,
            server_version,
            tool_router: tools,
        }
    }

    /// Who this call comes from, through `identify`: the operator, and the
    /// rules in its swarm.
    async fn call(&self, context: &RequestContext<RoleServer>) -> rules::Result<Call> {
        let principal = principal(context)?;
        let who = self.identify.operator(&principal).await?;
        Ok(Call {
            rules: self.rules.in_swarm(who.swarm),
            operator: who.actor,
        })
    }

    /// Makes `change` to `channel_id` as the call's operator, then shows the
    /// channel as `get_channel` does.
    async fn change(
        &self,
        call: &Call,
        f: Format,
        channel_id: String,
        change: ChannelChange,
    ) -> rules::Result<String> {
        call.rules
            .change_channel(call.actor(), channel_id.clone(), change)
            .await?;
        let channel = call.rules.admin_channel(&call.operator, channel_id).await?;
        Ok(self.fmt.admin_channel(f, &channel))
    }

    /// `hours` as a quiet period for stale agents. Zero is refused by
    /// `rules`.
    fn idle(hours: u32) -> Duration {
        Duration::from_secs(u64::from(hours) * 3600)
    }

    /// The answer to `call`: `out`, or its refusal with the advice
    /// [`word`] gives the call's operator.
    fn respond(&self, format: Format, call: &Call, out: rules::Result<String>) -> CallToolResult {
        self.refuse_or(format, out.map_err(|e| word(&call.operator.audit, e)))
    }

    /// The answer to a call that was not identified: `e`, as it is.
    fn refuse(&self, format: Format, e: RuleError) -> CallToolResult {
        self.refuse_or(format, Err(e))
    }

    fn refuse_or(&self, format: Format, out: rules::Result<String>) -> CallToolResult {
        mcp_tools::respond(&self.fmt, format, out, |e| {
            tracing::error!(error = %e, "admin tool call failed");
        })
    }

    fn limit(&self, requested: Option<u32>) -> usize {
        self.rules.limits().limit(requested)
    }
}

/// One admin call, as identified: the rules in its swarm, and the operator
/// it acts as.
struct Call {
    rules: Rules,
    operator: Operator,
}

impl Call {
    /// Who makes a change: the call's operator.
    fn actor(&self) -> Actor {
        Actor::Operator(self.operator.clone())
    }
}

/// `e`, with the advice an admin agent needs for the refusals it can act
/// on, for an operator asking from `audit`: how an admin key with no name
/// gets one, when it is refused a change, and which tools pass on or clear
/// an owner. Other errors are as `rules` gave them, so an operator that is
/// not an unnamed admin key and is refused a change keeps the rules' words.
fn word(audit: &Audit, e: RuleError) -> RuleError {
    let unnamed_key = matches!(audit, Audit::AdminEndpoint { key_name: None });
    let message = match (&e.refusal, unnamed_key) {
        (Some(Refusal::LacksPermission(Permission::Change)), true) => {
            "this admin key has no name, so it may only read. The operator names it by \
             making the key again with `kkt key create --admin --force --name <name>` \
             and restarting the server."
                .to_string()
        }
        (Some(Refusal::OwnsChannels { handle, channels }), _) => format!(
            "{handle} owns {}; make another member the owner, or clear the owner, \
             with `set_owner` or `clear_owner` first",
            channels.join(", ")
        ),
        _ => return e,
    };
    RuleError { message, ..e }
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
    async fn list_channels(
        &self,
        Parameters(p): Parameters<ListChannelsParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let page = call
                .rules
                .admin_list_channels(&call.operator, p.query, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self
                .fmt
                .admin_channels(f, &page, call.rules.limits().purpose_preview_chars))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn get_channel(
        &self,
        Parameters(p): Parameters<ChannelParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let channel = call
                .rules
                .admin_channel(&call.operator, p.channel_id)
                .await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn list_members(
        &self,
        Parameters(p): Parameters<ListMembersParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let members = call
                .rules
                .admin_list_members(&call.operator, p.channel_id, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self.fmt.members(f, &members))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn get_note(
        &self,
        Parameters(p): Parameters<GetNoteParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let note = call.rules.admin_note(&call.operator, p.channel_id).await?;
            let offset = mcp_tools::offset(
                p.offset,
                note.text.chars().count(),
                "offset is past the end of the note",
            )?;
            Ok(self.fmt.note(f, &note, offset))
        }
        .await;
        self.respond(f, &call, out)
    }

    // ---- Messages ----

    #[tool]
    async fn read(
        &self,
        Parameters(p): Parameters<ReadParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let position = mcp_tools::position(p.after_id, p.before_id)?;
            let q = ReadQuery {
                channel_id: p.channel_id,
                position,
                from_agent: p.from_agent,
                limit: self.limit(p.limit),
            };
            let page = call.rules.admin_read(&call.operator, q).await?;
            Ok(self
                .fmt
                .read(f, &page, position, mcp_tools::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn get_message(
        &self,
        Parameters(p): Parameters<GetMessageParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let message = call
                .rules
                .admin_message(&call.operator, p.message_id)
                .await?;
            Ok(self.fmt.message(f, &message))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn activity(
        &self,
        Parameters(p): Parameters<ActivityParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let rows = call
                .rules
                .admin_activity(&call.operator, p.after_id)
                .await?;
            Ok(self.fmt.activity(f, &rows))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn new_messages(
        &self,
        Parameters(p): Parameters<NewMessagesParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let result = call
                .rules
                .admin_new_messages(
                    &call.operator,
                    p.after_id,
                    p.channel_ids,
                    self.limit(p.limit),
                )
                .await?;
            Ok(self
                .fmt
                .admin_waited(f, &result, mcp_tools::body_chars(p.max_body_chars)))
        }
        .await;
        self.respond(f, &call, out)
    }

    // ---- Agents and the server announcement ----

    #[tool]
    async fn list_agents(
        &self,
        Parameters(p): Parameters<ListAgentsParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let page = call
                .rules
                .admin_list_agents(
                    &call.operator,
                    p.query,
                    p.lane,
                    self.limit(p.limit),
                    p.cursor,
                )
                .await?;
            Ok(self.fmt.agents(f, &page))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn get_agent(
        &self,
        Parameters(p): Parameters<AgentParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let view = call.rules.admin_agent(&call.operator, p.handle).await?;
            let offset = mcp_tools::offset(
                p.offset,
                view.channels.len(),
                "offset is past the last channel",
            )?;
            Ok(self.fmt.admin_agent(f, &view, offset))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn list_memories(
        &self,
        Parameters(p): Parameters<ListMemoriesParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let list = call
                .rules
                .admin_list_memories(p.handle, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self.fmt.memory_list(f, &list))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn recall(
        &self,
        Parameters(p): Parameters<RecallParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let entry = call.rules.admin_recall(p.handle, p.name).await?;
            let offset = mcp_tools::offset(
                p.offset,
                entry.text.chars().count(),
                "offset is past the end of the entry",
            )?;
            Ok(self.fmt.memory_entry(f, &entry, offset))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn get_server_announcement(
        &self,
        Parameters(p): Parameters<FormatOnly>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let current = call.rules.current_server_announcement().await?;
            Ok(self.fmt.server_announcement(f, current.as_ref()))
        }
        .await;
        self.respond(f, &call, out)
    }

    // ---- Lanes ----

    #[tool]
    async fn list_lanes(
        &self,
        Parameters(p): Parameters<ListLanesParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let page = call
                .rules
                .lanes_page(&call.operator, self.limit(p.limit), p.cursor)
                .await?;
            Ok(self.fmt.lanes(f, &page))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn get_lane(
        &self,
        Parameters(p): Parameters<GetLaneParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let lane = call.rules.lane(&call.operator, p.lane).await?;
            let offset = mcp_tools::offset(
                p.offset,
                lane.current.text.chars().count(),
                "offset is past the end of the lane's instructions",
            )?;
            Ok(self.fmt.lane(f, &lane, offset))
        }
        .await;
        self.respond(f, &call, out)
    }

    // ---- Changes, as the operator via this key's name ----

    #[tool]
    async fn create_channel(
        &self,
        Parameters(p): Parameters<CreateChannelParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let new = NewChannel {
                name: p.name,
                purpose: p.purpose,
                open: p.open,
                members: p.members.unwrap_or_default(),
            };
            let created = call
                .rules
                .create_channel_as_operator(call.actor(), new)
                .await?;
            let channel = call.rules.admin_channel(&call.operator, created.id).await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn set_purpose(
        &self,
        Parameters(p): Parameters<SetPurposeParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            call.rules
                .set_purpose(call.actor(), p.channel_id.clone(), p.purpose)
                .await?;
            let channel = call
                .rules
                .admin_channel(&call.operator, p.channel_id)
                .await?;
            Ok(self.fmt.admin_channel(f, &channel))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn set_open(
        &self,
        Parameters(p): Parameters<SetOpenParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = self
            .change(&call, f, p.channel_id, ChannelChange::SetOpen(p.open))
            .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn add_member(
        &self,
        Parameters(p): Parameters<ChannelAgentParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = self
            .change(&call, f, p.channel_id, ChannelChange::AddMember(p.handle))
            .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn remove_member(
        &self,
        Parameters(p): Parameters<ChannelAgentParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = self
            .change(
                &call,
                f,
                p.channel_id,
                ChannelChange::RemoveMember(p.handle),
            )
            .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn set_owner(
        &self,
        Parameters(p): Parameters<ChannelAgentParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = self
            .change(
                &call,
                f,
                p.channel_id,
                ChannelChange::SetOwner(Some(p.handle)),
            )
            .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn clear_owner(
        &self,
        Parameters(p): Parameters<ChannelParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = self
            .change(&call, f, p.channel_id, ChannelChange::SetOwner(None))
            .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn hide_channel(
        &self,
        Parameters(p): Parameters<ChannelParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = self
            .change(&call, f, p.channel_id, ChannelChange::Hide)
            .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn hide_agent(
        &self,
        Parameters(p): Parameters<HandleParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            call.rules.hide_agent(call.actor(), p.handle).await?;
            Ok(self.fmt.ok(f))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn list_stale_agents(
        &self,
        Parameters(p): Parameters<StaleParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let stale = call
                .rules
                .stale_agents(&call.operator, Self::idle(p.idle_hours))
                .await?;
            Ok(self.fmt.stale_agents(f, p.idle_hours, &stale))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn hide_stale_agents(
        &self,
        Parameters(p): Parameters<HideStaleParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let outcome = call
                .rules
                .hide_stale_agents(call.actor(), Self::idle(p.idle_hours), p.handles)
                .await?;
            Ok(self.fmt.stale_hidden(f, &outcome))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn set_server_announcement(
        &self,
        Parameters(p): Parameters<AnnouncementParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let saved = call
                .rules
                .save_server_announcement(call.actor(), p.text)
                .await?;
            Ok(self.fmt.server_announcement(f, Some(&saved)))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn set_lane(
        &self,
        Parameters(p): Parameters<SetLaneParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let save = LaneSave {
                name: p.lane,
                description: p.description,
                text: p.text,
            };
            let lane = call.rules.save_lane(call.actor(), save).await?;
            Ok(self.fmt.lane(f, &lane, 0))
        }
        .await;
        self.respond(f, &call, out)
    }

    #[tool]
    async fn retire_lane(
        &self,
        Parameters(p): Parameters<LaneParams>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let f = p.format.unwrap_or_default();
        let call = match self.call(&context).await {
            Ok(call) => call,
            Err(e) => return self.refuse(f, e),
        };
        let out = async {
            let lane = call.rules.retire_lane(call.actor(), p.lane).await?;
            Ok(self.fmt.lane(f, &lane, 0))
        }
        .await;
        self.respond(f, &call, out)
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
    use kkt_identity::KeyMode;
    use kkt_names::AdminName;

    #[test]
    fn every_tool_and_parameter_has_text_in_admin_tools_toml() {
        let wrong = tool_text::mismatches(
            tool_text::ADMIN,
            &mcp_tools::offered(&AdminServer::tool_router()),
        );
        assert!(wrong.is_empty(), "admin-tools.toml:\n{}", wrong.join("\n"));
    }

    fn sentinel() -> AdminName {
        AdminName::new("sentinel", kkt_config::MAX_ADMIN_NAME_CHARS).expect("a name")
    }

    #[test]
    fn refusals_are_worded_for_an_admin_agent_as_they_always_were() {
        let unnamed = KeyMode::admin_operator(None)
            .may(Permission::Change)
            .expect_err("an unnamed key may not change");
        let worded = word(&key(None), unnamed);
        assert_eq!(worded.code, rules::Code::Unauthenticated);
        assert_eq!(
            worded.message,
            "this admin key has no name, so it may only read. The operator names it by \
             making the key again with `kkt key create --admin --force --name <name>` \
             and restarting the server."
        );

        let owns = RuleError::invalid("plain").because(Refusal::OwnsChannels {
            handle: "amber-falcon".into(),
            channels: vec!["ch_a".into(), "ch_b".into()],
        });
        assert_eq!(
            word(&key(Some(sentinel())), owns).message,
            "amber-falcon owns ch_a, ch_b; make another member the owner, or clear the owner, \
             with `set_owner` or `clear_owner` first"
        );

        let other = RuleError::invalid("left alone");
        assert_eq!(word(&key(None), other.clone()), other);

        // A named key refused a change keeps the rules' plain words: the
        // unnamed-key advice would be wrong for it.
        let named = RuleError::new(rules::Code::Unauthenticated, "plain")
            .because(Refusal::LacksPermission(Permission::Change));
        assert_eq!(word(&key(Some(sentinel())), named.clone()), named);
        // So does an operator on the web page, which has no admin key.
        assert_eq!(word(&Audit::WebPage, named.clone()), named);
    }

    fn key(key_name: Option<AdminName>) -> Audit {
        Audit::AdminEndpoint { key_name }
    }
}
