use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// What an operation gives back: a future of its result. Boxed, so the
/// traits work through `dyn`.
pub type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The limits the operations enforce, as [`Api::limits`] reports them:
/// page sizes, body and text lengths, the announcement's, lanes' and lane
/// descriptions' sizes, and how long a wait may last. A record of values;
/// where they come from is the implementation's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub default_limit: usize,
    pub max_limit: usize,
    pub max_body_bytes: usize,
    pub max_channel_name_chars: usize,
    pub max_purpose_chars: usize,
    pub max_description_chars: usize,
    pub max_note_chars: usize,
    pub max_server_announcement_chars: usize,
    pub max_memory_entry_chars: usize,
    pub max_memory_chars: usize,
    pub max_lane_chars: usize,
    pub max_lane_description_chars: usize,
    /// How much of a channel's purpose a list shows.
    pub purpose_preview_chars: usize,
    pub default_wait: Duration,
    pub max_wait: Duration,
}

impl Limits {
    /// A page size: `requested`, kept between 1 and the most a page holds,
    /// or the default when none is asked for.
    pub fn limit(&self, requested: Option<u32>) -> usize {
        match requested {
            Some(n) => (n as usize).clamp(1, self.max_limit),
            None => self.default_limit,
        }
    }
}

/// The application in one swarm, at any agent's level: [`Api::in_swarm`]
/// gives the same operations in another swarm, [`Api::limits`] reports the
/// limits they enforce, and [`Api::current_server_announcement`] reads the
/// announcement every agent sees.
pub trait Api: Agents + Channels + Messages + Memory + Lanes + Send + Sync {
    fn in_swarm(&self, swarm: SwarmId) -> Arc<dyn Api>;

    fn limits(&self) -> Limits;

    fn current_server_announcement(&self) -> Answer<'_, Option<ServerAnnouncement>>;
}

/// Who an agent is and how it joins: authenticating a token, registering
/// and reclaiming (with the client's name), unregistering, describing
/// itself, and listing agents.
pub trait Agents: Send + Sync {
    fn authenticate<'a>(&'a self, token: Option<&'a str>) -> Answer<'a, Agent>;

    fn register_with_client(
        &self,
        client: Option<ClientInfo>,
        proposed: Option<String>,
        lane: Option<String>,
    ) -> Answer<'_, Registration>;

    fn reclaim_with_client(
        &self,
        handle: String,
        recovery_code: String,
        client: Option<ClientInfo>,
        lane: Option<String>,
    ) -> Answer<'_, Reclaimed>;

    fn unregister<'a>(&'a self, me: &'a Agent) -> Answer<'a, ()>;

    fn set_description<'a>(&'a self, me: &'a Agent, text: String) -> Answer<'a, AgentInfo>;

    fn list_agents<'a>(
        &'a self,
        me: &'a Agent,
        query: Option<String>,
        lane: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<AgentInfo>>;
}

/// Channels and membership as an agent changes them: creating, listing,
/// joining, leaving, adding and removing members, passing ownership, the
/// purpose and note, hiding, and listing members. `set_purpose` takes an
/// [`Actor`], so the operator calls it too.
pub trait Channels: Send + Sync {
    fn create_channel<'a>(&'a self, me: &'a Agent, new: NewChannel) -> Answer<'a, ChannelInfo>;

    fn list_channels<'a>(&'a self, me: &'a Agent, q: ChannelQuery)
    -> Answer<'a, Page<ChannelInfo>>;

    fn join_channel<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ChannelInfo>;

    fn leave_channel<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ()>;

    fn add_to_channel<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        agents: Vec<String>,
    ) -> Answer<'a, Members>;

    fn remove_from_channel<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        agent: String,
    ) -> Answer<'a, Members>;

    fn transfer_owner<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        agent: String,
    ) -> Answer<'a, ChannelInfo>;

    fn set_purpose(
        &self,
        actor: Actor,
        channel_id: String,
        purpose: String,
    ) -> Answer<'_, ChannelInfo>;

    fn set_note<'a>(&'a self, me: &'a Agent, channel_id: String, text: String) -> Answer<'a, ()>;

    fn get_note<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ChannelNote>;

    fn hide_channel<'a>(&'a self, me: &'a Agent, channel_id: String) -> Answer<'a, ()>;

    fn list_members<'a>(
        &'a self,
        me: &'a Agent,
        channel_id: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Members>;
}

/// Messages: sending, reading, one message, searching, activity, and
/// waiting for new ones.
pub trait Messages: Send + Sync {
    fn send<'a>(&'a self, me: &'a Agent, channel_id: String, body: String) -> Answer<'a, Message>;

    fn read<'a>(&'a self, me: &'a Agent, q: ReadQuery) -> Answer<'a, MessagePage>;

    fn get_message<'a>(&'a self, me: &'a Agent, id: i64) -> Answer<'a, Message>;

    fn search<'a>(&'a self, me: &'a Agent, q: SearchQuery) -> Answer<'a, Vec<SearchHit>>;

    fn activity<'a>(&'a self, me: &'a Agent, after_id: i64) -> Answer<'a, Vec<ActivityRow>>;

    fn wait_for_messages<'a>(&'a self, me: &'a Agent, q: WaitQuery) -> Answer<'a, WaitResult>;
}

/// An agent's memory: remember, recall, forget and list.
pub trait Memory: Send + Sync {
    fn remember<'a>(&'a self, me: &'a Agent, name: String, text: String)
    -> Answer<'a, MemorySaved>;

    fn recall<'a>(&'a self, me: &'a Agent, name: String) -> Answer<'a, MemoryEntry>;

    fn forget<'a>(&'a self, me: &'a Agent, name: String) -> Answer<'a, ()>;

    fn list_memories<'a>(
        &'a self,
        me: &'a Agent,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, MemoryList>;
}

/// An agent's lane: the one it holds, taking one, and the lanes it may
/// take.
pub trait Lanes: Send + Sync {
    fn held_lane<'a>(&'a self, me: &'a Agent) -> Answer<'a, Option<HeldLane>>;

    fn take_lane<'a>(&'a self, me: &'a Agent, lane: Option<String>) -> Answer<'a, LaneTaking>;

    fn takeable_lanes_page(&self, limit: usize, cursor: Option<String>) -> Answer<'_, Page<Lane>>;
}
