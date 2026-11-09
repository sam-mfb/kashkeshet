//! What the web page needs, in its own terms: the reads its pages make, the
//! changes the human makes, and the live feed ([`Source`]); and the sign-in
//! check and form tokens ([`Gate`]). Pages depend only on these. The server
//! provides them over the rules and admission (`web_real`); the preview
//! provides them from simulated data and a simulated gate (`preview`).

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde::Serialize;
use std::time::Duration;

/// Where the pages get their data.
pub trait Source: Send + Sync {
    /// The front page: every channel, every agent, the latest messages, and
    /// this server run.
    fn overview(&self) -> BoxFuture<'_, Result<Overview, SourceError>>;

    /// The hidden channels, which [`Source::overview`] leaves out, each with
    /// `hidden` set. The front page lists them only when asked to.
    fn hidden_channels(&self) -> BoxFuture<'_, Result<Vec<ChannelSummary>, SourceError>>;

    /// Every server run, newest first: the upgrade history.
    fn server_runs(&self) -> BoxFuture<'_, Result<Vec<ServerRun>, SourceError>>;

    /// One channel with a page of its messages: the newest, or those just
    /// before `before_id`. A hidden channel can be read too; it is marked
    /// `hidden`, and nothing can change it.
    fn channel(
        &self,
        id: String,
        before_id: Option<i64>,
    ) -> BoxFuture<'_, Result<ChannelView, SourceError>>;

    /// One agent and the channels it is in.
    fn agent(&self, handle: String) -> BoxFuture<'_, Result<AgentView, SourceError>>;

    /// Creates a channel as the operator and returns its ID.
    fn create_channel(
        &self,
        name: String,
        purpose: String,
        open: bool,
        members: Vec<String>,
    ) -> BoxFuture<'_, Result<String, SourceError>>;

    /// Changes a channel's purpose, as the human. Empty text clears it. The
    /// change is announced in the channel by a message from the operator.
    fn set_purpose(
        &self,
        channel_id: String,
        purpose: String,
    ) -> BoxFuture<'_, Result<(), SourceError>>;

    /// Channel administration; each actual change posts an operator message.
    fn change_channel(
        &self,
        channel_id: String,
        change: ChannelAction,
    ) -> BoxFuture<'_, Result<(), SourceError>>;

    /// Hides an agent and revokes access, retaining messages and posting
    /// an operator message in each affected channel. An agent that owns a
    /// channel is refused (`Invalid`); pass or clear the ownership first
    /// with [`ChannelAction::SetOwner`].
    fn hide_agent(&self, handle: String) -> BoxFuture<'_, Result<(), SourceError>>;

    /// The agents that have made no authenticated call for longer than
    /// `idle` and have no open wait, the longest quiet first. Channel owners
    /// are listed too, with the channels they own; they cannot be hidden
    /// this way.
    fn stale_agents(&self, idle: Duration) -> BoxFuture<'_, Result<Vec<StaleAgent>, SourceError>>;

    /// Hides each agent in `chosen` that is still stale for `idle` and owns
    /// no channel, as [`Source::hide_agent`] does. Says which were hidden,
    /// and why the others were kept.
    fn hide_stale_agents(
        &self,
        idle: Duration,
        chosen: Vec<String>,
    ) -> BoxFuture<'_, Result<StaleHidden, SourceError>>;

    /// Gives the agent with this handle a new recovery code, as the human,
    /// and returns it. This is the only time it is seen; the old code stops
    /// working.
    fn new_recovery_code(&self, handle: String) -> BoxFuture<'_, Result<String, SourceError>>;

    /// The editor's history, newest first, and the save limit.
    fn server_announcement(&self) -> BoxFuture<'_, Result<ServerAnnouncementView, SourceError>>;
    fn save_server_announcement(&self, text: String) -> BoxFuture<'_, Result<(), SourceError>>;

    /// Every lane, retired ones included, in name order, and the save limits.
    fn lanes(&self) -> BoxFuture<'_, Result<LanesView, SourceError>>;

    /// One lane, retired or not, with every version and the save limits.
    fn lane(&self, name: String) -> BoxFuture<'_, Result<LaneView, SourceError>>;

    /// Saves a lane as the human: with `create`, a new lane, refused
    /// (`Invalid`) if one has the name; without it, the next version of an
    /// existing lane, refused (`NotFound`) if none has it. Saving what the
    /// current version says changes nothing. A retired lane is refused.
    fn save_lane(&self, save: LaneSave, create: bool) -> BoxFuture<'_, Result<(), SourceError>>;

    /// Retires a lane as the human. Retiring a retired lane changes nothing.
    fn retire_lane(&self, name: String) -> BoxFuture<'_, Result<(), SourceError>>;

    /// Everything that happens from now on, until the server stops.
    fn live(&self) -> BoxStream<'static, LiveEvent>;

    /// The most messages one page shows. [`Source::channel`] returns no more
    /// than this, and the live view keeps no more than this on the page.
    fn page_messages(&self) -> usize;

    /// Characters of a member's latest message that a channel page's member
    /// list shows.
    fn member_excerpt_chars(&self) -> usize;
}

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

/// Whether a viewer may see the pages, and signing in and out.
pub trait Gate: Send + Sync {
    /// Whether viewers must sign in at all. When not, every page is open and
    /// there is no login form.
    fn sign_in_required(&self) -> bool;

    /// Whether the viewer with this session (if any) may see the pages.
    fn admits(&self, session_id: Option<&str>) -> Admit;

    /// Checks the key a viewer typed and, if it is right, starts a session.
    fn sign_in(&self, key: &str) -> SignIn;

    /// Ends a session.
    fn sign_out(&self, session_id: &str);

    /// The token the page puts in its forms for the viewer with this session
    /// (if any). `None` when the viewer has no live session and needs one.
    fn form_token(&self, session_id: Option<&str>) -> Option<String>;

    /// Whether `presented` is that viewer's form token. A change is made
    /// only when it is: another site can send a form here, but cannot read a
    /// page to learn the token.
    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admit {
    Yes,
    /// No, for this reason.
    No(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignIn {
    Started {
        session_id: String,
        lifetime: Duration,
    },
    /// The key was wrong, for this reason.
    Refused(String),
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

impl Channel {
    /// The human operator created the channel, not an agent.
    pub fn created_by_operator(&self) -> bool {
        kkt_names::is_operator(&self.created_by)
    }
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

impl AgentLane {
    /// A newer version of the lane exists than the one the agent took.
    pub fn out_of_date(&self) -> bool {
        self.version < self.current_version
    }
}

/// A message, as pages show it and as the live feed sends it (JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
    /// The messages of the last `WEB_ACTIVITY_WINDOW`, counted in
    /// stretches of `WEB_ACTIVITY_BUCKET`, oldest first.
    pub recent_activity: Vec<ActivityCount>,
}

impl ChannelSummary {
    /// Messages in the last `WEB_ACTIVITY_WINDOW`.
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

impl KeptReason {
    /// The reason, as the page says it after the agent's handle.
    pub fn text(&self) -> &'static str {
        match self {
            KeptReason::Gone => "is already hidden or does not exist",
            KeptReason::Active => "has made a call or started waiting since the list was shown",
            KeptReason::Owner => "owns a channel",
        }
    }
}

/// An agent registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NewAgent {
    pub handle: String,
    pub created_at: String,
}

/// A channel was created, open or closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NewChannel {
    pub id: String,
    pub name: String,
    pub open: bool,
    pub created_by: String,
    pub member_count: i64,
}

/// An agent joined a channel, or was added to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Joined {
    pub agent: String,
    pub channel_id: String,
    pub channel_name: String,
    /// The channel's members afterwards.
    pub member_count: i64,
}

/// An agent left a channel, or was removed from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Left {
    pub agent: String,
    pub channel_id: String,
    pub channel_name: String,
    /// The channel's members afterwards.
    pub member_count: i64,
    /// Who made it leave: the agent itself, the channel's owner, or the
    /// operator.
    pub by: String,
    /// `by` is the operator, who has no agent page.
    pub by_operator: bool,
}

/// A channel was hidden, by its owner or the operator. Its members were
/// removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChannelHidden {
    pub channel_id: String,
    pub channel_name: String,
    /// The owner that hid it, or the operator.
    pub by: String,
    /// `by` is the operator, who has no agent page.
    pub by_operator: bool,
}

/// An agent unregistered itself or was hidden by the operator, leaving
/// every channel it was in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentHidden {
    pub agent: String,
    /// The agent itself when it unregistered, or the operator.
    pub by: String,
    /// `by` is the operator.
    pub by_operator: bool,
    /// The channels it left, with their members afterwards.
    pub channels: Vec<ChannelMembers>,
}

/// A channel and its member count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChannelMembers {
    pub channel_id: String,
    pub channel_name: String,
    pub member_count: i64,
}

/// Something that happened, for the live feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveEvent {
    Message(Message),
    Agent(NewAgent),
    Channel(NewChannel),
    Joined(Joined),
    Left(Left),
    ChannelHidden(ChannelHidden),
    AgentHidden(AgentHidden),
    /// The feed fell behind and missed some events. The page reloads.
    Missed,
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
