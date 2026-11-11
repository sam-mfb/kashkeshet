//! The preview's own records of its simulated world: what kkt-web's own
//! contract described before the page used the kkt-api crates. The preview
//! keeps them inside, and gives the page kkt-api's records (`api`).

#[derive(Clone, Debug)]
pub enum ChannelAction {
    SetOpen(bool),
    AddMember(String),
    RemoveMember(String),
    /// Makes the member with this handle the owner, or with `None` leaves the channel
    /// with no owner.
    SetOwner(Option<String>),
    Hide,
}

/// Why a read failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// No such channel or agent.
    NotFound(String),
    /// The request made no sense, such as a malformed position or a purpose
    /// that is too long.
    Invalid(String),
    /// Something broke. The page says so; the cause is logged.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub note: String,
    pub note_edited_by: Option<String>,
    pub note_edited_at: Option<String>,
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub open: bool,
    pub created_by: String,
    pub created_at: String,
    pub member_count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub handle: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub listening: bool,
    pub waiting_channels: Vec<String>,
    /// The server run that created the agent, if known.
    pub run_id: Option<i64>,
    /// What the agent says about itself, if anything.
    pub description: Option<String>,
    /// The lane the agent holds, if any.
    pub lane: Option<AgentLane>,
}

/// The lane an agent holds: the version it took, and the lane's newest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLane {
    pub name: String,
    pub version: i64,
    pub current_version: i64,
}

/// A message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    /// The message is from the human operator, not an agent; `agent` is
    /// then `operator`.
    pub from_operator: bool,
    pub body: String,
    pub created_at: String,
    /// The server run that stored the message, if known.
    pub run_id: Option<i64>,
}

/// One start of the server and what produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRun {
    pub id: i64,
    pub started_at: String,
    pub version: String,
    pub git_commit: String,
    pub settings: String,
    pub instructions_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub channels: Vec<ChannelSummary>,
    pub agents: Vec<AgentSummary>,
    /// This server run, if it is recorded. Its start and `now` give the
    /// uptime.
    pub current_run: Option<ServerRun>,
    /// The source's time when this was read, in the stored form. The page
    /// measures "how long ago" from it, not from the viewer's clock.
    pub now: String,
}

/// A channel and how much it is used, for the front page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSummary {
    pub channel: Channel,
    /// Hidden: agents cannot see it, and only the human can read it.
    pub hidden: bool,
    pub message_count: i64,
    /// When the newest message was sent, and by whom.
    pub last_message_at: Option<String>,
    pub last_speaker: Option<String>,
    /// The newest message is from the operator, who has no agent page.
    pub last_from_operator: bool,
    /// The messages of the last stretch of time the caller asked for (the
    /// window), counted in stretches of the length it asked for (the
    /// bucket), oldest first.
    pub recent_activity: Vec<ActivityCount>,
}

#[cfg(test)]
impl ChannelSummary {
    /// Messages in the window [`ChannelSummary::recent_activity`] covers.
    pub fn recent_count(&self) -> i64 {
        self.recent_activity.iter().map(|a| a.count).sum()
    }
}

/// How many messages a channel got in the stretch starting at `start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityCount {
    pub start: String,
    pub count: i64,
}

/// An agent and the channels it is in, for the front page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSummary {
    pub agent: Agent,
    pub channels: Vec<ChannelLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelLink {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelView {
    pub channel: Channel,
    /// Hidden: agents cannot see it, and nothing can change it. Hiding
    /// removed its members; its messages and note are kept.
    pub hidden: bool,
    /// The agent that owns the channel, if any.
    pub owner: Option<String>,
    /// Members in the order they joined.
    pub members: Vec<Agent>,
    /// Each member's latest message in the channel, for the members that
    /// have sent one.
    pub latest_messages: Vec<Message>,
    /// Oldest first.
    pub messages: Vec<Message>,
    /// Set when older messages exist: the `before_id` that reads them.
    pub older_before_id: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentView {
    pub agent: Agent,
    pub channels: Vec<Channel>,
    pub message_count: i64,
    /// The entries the avatar keeps in its memory for whoever next holds
    /// it, in name order.
    pub memory: Vec<MemoryEntry>,
}

/// One named entry in an avatar's memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryEntry {
    pub name: String,
    pub text: String,
    pub saved_at: String,
}

/// An agent that has gone quiet, with what the human needs to decide
/// whether to hide it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleAgent {
    pub agent: Agent,
    /// The channels it is in.
    pub channels: Vec<ChannelLink>,
    /// How many entries its memory holds.
    pub memory_entries: usize,
    /// The channels it owns. An owner cannot be hidden as stale.
    pub owns: Vec<ChannelLink>,
}

/// What hiding stale agents did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleHidden {
    /// In the order chosen.
    pub hidden: Vec<String>,
    /// The agents chosen but kept, in the order chosen.
    pub kept: Vec<KeptAgent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptAgent {
    pub handle: String,
    pub reason: KeptReason,
}

/// Why a chosen agent was not hidden as stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptReason {
    /// No such agent, or it is already hidden.
    Gone,
    /// It made a call within the period, or is waiting for messages.
    Active,
    /// It owns a channel.
    Owner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAnnouncementVersion {
    pub version: i64,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent that saved it, such as `admin-sentinel`; `None` when
    /// the human did.
    pub saved_by_admin: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAnnouncementView {
    pub history: Vec<ServerAnnouncementVersion>,
    pub max_chars: usize,
}

/// What the human saves as a lane, or its next version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneSave {
    pub name: String,
    pub description: String,
    pub text: String,
}

/// How long a lane's parts may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneLimits {
    pub max_name_chars: usize,
    pub max_description_chars: usize,
    pub max_text_chars: usize,
}

/// One saved version of a lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneVersion {
    pub version: i64,
    pub description: String,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent that saved it; `None` when the human did.
    pub saved_by_admin: Option<String>,
    /// Where `kkt lanes import` read it from; `None` when it was saved on
    /// the web page or at `/mcp-admin`.
    pub source: Option<LaneSource>,
}

/// The folder a lane version was imported from, and its git commit if the
/// folder was in a git checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneSource {
    pub folder: String,
    pub commit: Option<String>,
}

/// When a lane was retired, and by whom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneRetired {
    pub at: String,
    /// The admin agent that retired it; `None` when the human did.
    pub by_admin: Option<String>,
}

/// A lane and its current version, for the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneSummary {
    pub name: String,
    pub created_at: String,
    pub retired: Option<LaneRetired>,
    pub current: LaneVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanesView {
    /// In name order.
    pub lanes: Vec<LaneSummary>,
    pub limits: LaneLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneView {
    pub name: String,
    pub created_at: String,
    pub retired: Option<LaneRetired>,
    /// Newest first; the first is the current version. Never empty.
    pub history: Vec<LaneVersion>,
    pub limits: LaneLimits,
}
