//! What /mcp-admin's administrator does: the admin reads across every
//! channel of a swarm ([`Admin`]), and what is server-wide, creating a swarm
//! ([`Swarms`]), with [`AdminChannel`]. [`AdminApi`] is every operation
//! /mcp-admin uses, in one swarm.
//!
//! A contract crate: traits and plain values. kkt-rules implements it. The
//! records every level shares are kkt-api-agent's.

use kkt_api_agent::{
    ActivityRow, AgentInfo, AgentView, Answer, ChannelInfo, ChannelNote, Members, MemoryEntry,
    MemoryList, Message, MessagePage, Operator, Page, ReadQuery, SwarmId, WaitResult,
};
use kkt_api_operator::OperatorApi;
use serde::Serialize;
use std::sync::Arc;

/// A channel as the admin agent sees it.
#[derive(Clone, Debug, Serialize)]
pub struct AdminChannel {
    pub info: ChannelInfo,
    /// Hidden by its owner or the operator: no agent can find it, but its
    /// messages are kept.
    pub hidden: bool,
}

/// What /mcp-admin reads, across every channel of the swarm, hidden ones
/// included: activity, agents, channels, members, memories, messages and
/// notes. Each checks the operator's permission to read, where it takes
/// one.
pub trait Admin: Send + Sync {
    fn admin_activity<'a>(
        &'a self,
        operator: &'a Operator,
        after_id: i64,
    ) -> Answer<'a, Vec<ActivityRow>>;

    fn admin_agent<'a>(&'a self, operator: &'a Operator, handle: String) -> Answer<'a, AgentView>;

    fn admin_channel<'a>(
        &'a self,
        operator: &'a Operator,
        channel_id: String,
    ) -> Answer<'a, AdminChannel>;

    fn admin_list_agents<'a>(
        &'a self,
        operator: &'a Operator,
        query: Option<String>,
        lane: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<AgentInfo>>;

    fn admin_list_channels<'a>(
        &'a self,
        operator: &'a Operator,
        query: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<AdminChannel>>;

    fn admin_list_members<'a>(
        &'a self,
        operator: &'a Operator,
        channel_id: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Members>;

    fn admin_list_memories(
        &self,
        handle: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'_, MemoryList>;

    fn admin_message<'a>(&'a self, operator: &'a Operator, id: i64) -> Answer<'a, Message>;

    fn admin_new_messages<'a>(
        &'a self,
        operator: &'a Operator,
        after_id: i64,
        channel_ids: Option<Vec<String>>,
        limit: usize,
    ) -> Answer<'a, WaitResult>;

    fn admin_note<'a>(
        &'a self,
        operator: &'a Operator,
        channel_id: String,
    ) -> Answer<'a, ChannelNote>;

    fn admin_read<'a>(&'a self, operator: &'a Operator, q: ReadQuery) -> Answer<'a, MessagePage>;

    fn admin_recall(&self, handle: String, name: String) -> Answer<'_, MemoryEntry>;
}

/// What is server-wide: creating a swarm.
pub trait Swarms: Send + Sync {
    /// Makes the swarm `swarm`, recorded as made by this server run. It
    /// starts with no avatars and no channels.
    fn create_swarm(&self, swarm: SwarmId) -> Answer<'_, ()>;
}

/// Every operation /mcp-admin uses, in one swarm: the operator's and the
/// admin reads. [`AdminApi::in_swarm_admin`] gives the same in another swarm.
pub trait AdminApi: OperatorApi + Admin {
    fn in_swarm_admin(&self, swarm: SwarmId) -> Arc<dyn AdminApi>;
}
