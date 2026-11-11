//! The rules as kkt-api-agent's contract: each operation is the rules' own
//! method of the same name.

use super::*;
use kkt_api_agent::{Agents, Answer, Api, Channels, Limits, Memory, Messages};
use std::sync::Arc;

impl Rules {
    /// The limits these rules enforce, as kkt-api-agent's record.
    pub fn limits(&self) -> Limits {
        self.settings().limits()
    }
}

impl Settings {
    /// These settings' limits, as kkt-api-agent's record.
    pub fn limits(&self) -> Limits {
        let s = self;
        Limits {
            default_limit: s.default_limit,
            max_limit: s.max_limit,
            max_body_bytes: s.max_body_bytes,
            max_channel_name_chars: s.max_channel_name_chars,
            max_purpose_chars: s.max_purpose_chars,
            max_description_chars: s.max_description_chars,
            max_note_chars: s.max_note_chars,
            max_server_announcement_chars: s.max_server_announcement_chars,
            max_memory_entry_chars: s.max_memory_entry_chars,
            max_memory_chars: s.max_memory_chars,
            max_lane_chars: s.max_lane_chars,
            max_lane_description_chars: s.max_lane_description_chars,
            purpose_preview_chars: s.purpose_preview_chars,
            default_wait: s.default_wait,
            max_wait: s.max_wait,
        }
    }
}

impl Api for Rules {
    fn in_swarm(&self, swarm: SwarmId) -> Arc<dyn Api> {
        Arc::new(Rules::in_swarm(self, swarm))
    }

    fn limits(&self) -> Limits {
        Rules::limits(self)
    }

    fn current_server_announcement(&self) -> Answer<'_, Option<ServerAnnouncement>> {
        Box::pin(Rules::current_server_announcement(self))
    }
}

impl Agents for Rules {
    fn authenticate<'a>(&'a self, token: Option<&'a str>) -> Answer<'a, Agent> {
        Box::pin(Rules::authenticate(self, token))
    }

    fn register_with_client(
        &self,
        client: Option<ClientInfo>,
        proposed: Option<String>,
        lane: Option<String>,
    ) -> Answer<'_, Registration> {
        Box::pin(Rules::register_with_client(self, client, proposed, lane))
    }

    fn reclaim_with_client(
        &self,
        handle: String,
        recovery_code: String,
        client: Option<ClientInfo>,
        lane: Option<String>,
    ) -> Answer<'_, Reclaimed> {
        Box::pin(Rules::reclaim_with_client(
            self,
            handle,
            recovery_code,
            client,
            lane,
        ))
    }

    fn unregister<'a>(&'a self, me: &'a Agent) -> Answer<'a, ()> {
        Box::pin(Rules::unregister(self, me))
    }

    fn set_description<'a>(&'a self, me: &'a Agent, text: String) -> Answer<'a, AgentInfo> {
        Box::pin(Rules::set_description(self, me, text))
    }

    fn list_agents<'a>(
        &'a self,
        me: &'a Agent,
        query: Option<String>,
        lane: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<AgentInfo>> {
        Box::pin(Rules::list_agents(self, me, query, lane, limit, cursor))
    }
}

impl Channels for Rules {
    fn create_channel<'a>(&'a self, me: &'a Agent, new: NewChannel) -> Answer<'a, ChannelInfo> {
        Box::pin(Rules::create_channel(self, me, new))
    }

    fn list_channels<'a>(
        &'a self,
        me: &'a Agent,
        q: ChannelQuery,
    ) -> Answer<'a, Page<ChannelInfo>> {
        Box::pin(Rules::list_channels(self, me, q))
    }

    fn join_channel<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ChannelInfo> {
        Box::pin(Rules::join_channel(self, me, channel_id))
    }

    fn leave_channel<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ()> {
        Box::pin(Rules::leave_channel(self, me, channel_id))
    }

    fn add_to_channel<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        agents: Vec<String>,
    ) -> Answer<'a, Members> {
        Box::pin(Rules::add_to_channel(self, me, channel_id, agents))
    }

    fn remove_from_channel<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        agent: String,
    ) -> Answer<'a, Members> {
        Box::pin(Rules::remove_from_channel(self, me, channel_id, agent))
    }

    fn transfer_owner<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        agent: String,
    ) -> Answer<'a, ChannelInfo> {
        Box::pin(Rules::transfer_owner(self, me, channel_id, agent))
    }

    fn set_purpose(
        &self,
        actor: Actor,
        channel_id: String,
        purpose: String,
    ) -> Answer<'_, ChannelInfo> {
        Box::pin(Rules::set_purpose(self, actor, channel_id, purpose))
    }

    fn set_note<'a>(&'a self, me: &'a Agent, channel_id: String, text: String) -> Answer<'a, ()> {
        Box::pin(Rules::set_note(self, me, channel_id, text))
    }

    fn get_note<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ChannelNote> {
        Box::pin(Rules::get_note(self, me, channel_id))
    }

    fn hide_channel<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ()> {
        Box::pin(Rules::hide_channel(self, me, channel_id))
    }

    fn list_members<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Members> {
        Box::pin(Rules::list_members(self, me, channel_id, limit, cursor))
    }
}

impl Messages for Rules {
    fn send<'a>(&'a self, me: &'a Agent, channel_id: String, body: String) -> Answer<'a, Message> {
        Box::pin(Rules::send(self, me, channel_id, body))
    }

    fn read<'a>(&'a self, me: &'a Agent, q: ReadQuery) -> Answer<'a, MessagePage> {
        Box::pin(Rules::read(self, me, q))
    }

    fn get_message<'a>(&'a self, me: &'a Agent, id: i64) -> Answer<'a, Message> {
        Box::pin(Rules::get_message(self, me, id))
    }

    fn search<'a>(&'a self, me: &'a Agent, q: SearchQuery) -> Answer<'a, Vec<SearchHit>> {
        Box::pin(Rules::search(self, me, q))
    }

    fn activity<'a>(&'a self, me: &'a Agent, after_id: i64) -> Answer<'a, Vec<ActivityRow>> {
        Box::pin(Rules::activity(self, me, after_id))
    }

    fn wait_for_messages<'a>(&'a self, me: &'a Agent, q: WaitQuery) -> Answer<'a, WaitResult> {
        Box::pin(Rules::wait_for_messages(self, me, q))
    }
}

impl Memory for Rules {
    fn remember<'a>(
        &'a self,
        me: &'a Agent,
        name: String,
        text: String,
    ) -> Answer<'a, MemorySaved> {
        Box::pin(Rules::remember(self, me, name, text))
    }

    fn recall<'a>(&'a self, me: &'a Agent, name: String) -> Answer<'a, MemoryEntry> {
        Box::pin(Rules::recall(self, me, name))
    }

    fn forget<'a>(&'a self, me: &'a Agent, name: String) -> Answer<'a, ()> {
        Box::pin(Rules::forget(self, me, name))
    }

    fn list_memories<'a>(
        &'a self,
        me: &'a Agent,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, MemoryList> {
        Box::pin(Rules::list_memories(self, me, limit, cursor))
    }
}

impl kkt_api_agent::Lanes for Rules {
    fn held_lane<'a>(&'a self, me: &'a Agent) -> Answer<'a, Option<HeldLane>> {
        Box::pin(Rules::held_lane(self, me))
    }

    fn take_lane<'a>(&'a self, me: &'a Agent, lane: Option<String>) -> Answer<'a, LaneTaking> {
        Box::pin(Rules::take_lane(self, me, lane))
    }

    fn takeable_lanes_page(&self, limit: usize, cursor: Option<String>) -> Answer<'_, Page<Lane>> {
        Box::pin(Rules::takeable_lanes_page(self, limit, cursor))
    }
}
