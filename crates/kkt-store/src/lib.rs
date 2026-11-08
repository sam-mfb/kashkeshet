//! Storage: what the rules can ask of the database and change in it.
//!
//! The contract is two traits. [`Reads`] answers questions; [`Writes`] adds
//! the changes. Neither decides anything about permissions: the rules ask, for
//! example, whether an agent is a member, and decide what that means. A
//! query over many channels takes a [`ChannelScope`]: the rules say which
//! kinds of channel it covers, and the store only applies it.
//!
//! Avatars and channels belong to a swarm ([`SwarmId`]). Every method over
//! them, their memberships, messages, notes and memory takes the swarm, or
//! a [`ChannelScope`] that carries it, and sees and changes only what is in
//! that swarm: to it, an avatar or channel of another swarm does not exist.
//! A change that would add a row tied to something outside the swarm, such
//! as a message in another swarm's channel, fails with an error. Lanes, the
//! server announcement and server runs belong to the server, not to a
//! swarm. So do handles and channel IDs: [`Reads::agent_name_taken`] and
//! [`Reads::channel_id_taken`] look across every swarm.
//!
//! [`Db`] is the one implementation, over SQLite. Each call to [`Db::read`] or
//! [`Db::write`] runs one closure, and a write closure is one transaction: its
//! changes all commit together, or not at all.
//!
//! [`pending_upgrade`] and [`copy_database`] work on a database file without
//! opening it as a [`Db`], for backups.

mod sqlite;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

/// Where unit tests put their scratch files: shared with the integration tests.
#[cfg(test)]
use kkt_scratch as scratch;

pub use sqlite::Db;
#[cfg(any(test, feature = "real"))]
pub use sqlite::{copy_database, pending_upgrade};

/// A database whose tables are at schema version `from`, which this version
/// of kkt brings up to `to` when it opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaUpgrade {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(String),
    #[error("database task failed: {0}")]
    Task(String),
    #[error("database lock poisoned")]
    Poisoned,
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Which swarm an avatar or channel belongs to, and so which swarm a lookup
/// is bound to. An avatar or channel never moves to another swarm.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SwarmId(String);

impl SwarmId {
    /// The swarm with this ID.
    pub fn new(id: impl Into<String>) -> SwarmId {
        SwarmId(id.into())
    }

    /// The swarm the database's upgrade to swarms made, `sw_main`, which
    /// every avatar and channel from before swarms is in. A server in key
    /// mode has this one swarm and no other.
    pub fn main() -> SwarmId {
        SwarmId::new(MAIN_SWARM)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The ID of [`SwarmId::main`], as migration 0022 writes it.
const MAIN_SWARM: &str = "sw_main";

/// An agent as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecord {
    pub name: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    /// The server run that created the agent. `None` for agents from before
    /// runs were recorded.
    pub run_id: Option<i64>,
    /// When the operator revoked the agent's token, if it has.
    pub revoked_at: Option<String>,
    /// What the agent says about itself, if it has said anything.
    pub description: Option<String>,
    pub client: Option<AgentClient>,
    /// The lane the agent has taken, if any.
    pub lane: Option<AgentLaneRecord>,
}

/// The lane an agent has taken, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLaneRecord {
    pub taken: TakenLaneRecord,
    /// The lane's newest version, which may be newer than the one taken.
    pub current_version: i64,
}

/// Which lane, and which version of it, an agent takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakenLaneRecord {
    pub lane: String,
    pub version: i64,
    pub taken_at: String,
    /// The server run in which it was taken.
    pub run_id: i64,
}

/// MCP initialization metadata last observed at registration or reclaim.
/// Reported by the client, not verified by the server.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AgentClient {
    pub name: String,
    pub version: String,
    pub recorded_at: String,
    pub run_id: i64,
}

#[derive(Clone, Debug)]
pub struct NewAgentRecord {
    pub name: String,
    pub token_hash: Vec<u8>,
    pub recovery_hash: Option<Vec<u8>>,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub run_id: i64,
}

/// The facts about a channel that decide who may use it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelFacts {
    pub id: String,
    pub name: String,
    pub open: bool,
    /// The agent that may remove members and pass the ownership on, if any.
    pub owner: Option<String>,
}

/// One named entry in an avatar's memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryEntryRecord {
    pub name: String,
    pub text: String,
    pub saved_at: String,
    /// The server run that saved it; `None` for an entry made from a
    /// handoff note when the database was upgraded.
    pub run_id: Option<i64>,
}

/// A pinned channel note, including the most recent edit (also when cleared).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelNoteRecord {
    pub text: String,
    pub edited_by: Option<String>,
    pub edited_at: Option<String>,
}

/// A channel's stored fields and counts, as one viewer sees them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSummary {
    pub note: ChannelNoteRecord,
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub open: bool,
    pub created_by: String,
    pub created_at: String,
    pub owner: Option<String>,
    pub member_count: i64,
    /// False when there is no viewer.
    pub viewer_is_member: bool,
    /// Messages with IDs above the `after_id` asked for, if one was.
    pub new_messages: Option<i64>,
}

/// A channel, hidden or not, as no viewer in particular sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnyChannelRecord {
    pub summary: ChannelSummary,
    /// Hidden by its owner or the operator: no agent can find it.
    pub hidden: bool,
    /// The server run that created it; `None` for channels created before
    /// runs were recorded for them.
    pub run_id: Option<i64>,
    /// The server run that last edited its note; `None` if the note was
    /// never edited, or last edited before runs were recorded for notes.
    pub note_run_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct NewChannelRecord {
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub open: bool,
    pub created_by: String,
    pub created_at: String,
    pub owner: Option<String>,
    /// The server run creating it.
    pub run_id: i64,
}

/// A new text for a channel's note, and who saved it.
#[derive(Clone, Debug)]
pub struct NoteEditRecord {
    pub text: String,
    pub editor: String,
    pub at: String,
    /// The server run saving it.
    pub run_id: i64,
}

/// The stored facts that decide whether a viewer may use a channel: whether
/// it is hidden or open, and whether the viewer is in it or owns it. With no
/// viewer, `member` and `owner` are false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelKind {
    pub hidden: bool,
    pub open: bool,
    pub member: bool,
    pub owner: bool,
}

impl ChannelKind {
    /// Every kind, each once.
    pub fn all() -> impl Iterator<Item = ChannelKind> {
        let both = [false, true];
        both.into_iter().flat_map(move |hidden| {
            both.into_iter().flat_map(move |open| {
                both.into_iter().flat_map(move |member| {
                    both.into_iter().map(move |owner| ChannelKind {
                        hidden,
                        open,
                        member,
                        owner,
                    })
                })
            })
        })
    }
}

/// Which channels a query covers: those in one swarm whose kind, for the
/// viewer, is one of the allowed kinds. The swarm is a hard filter: no kind
/// reaches a channel of another swarm. The store only applies the scope;
/// the caller decides which kinds are allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelScope {
    swarm: SwarmId,
    viewer: Option<String>,
    allowed: Vec<ChannelKind>,
}

impl ChannelScope {
    /// The channels of `swarm` whose kind for `viewer` (or for no viewer)
    /// `allows` accepts. `allows` is asked about every kind.
    pub fn new(
        swarm: SwarmId,
        viewer: Option<String>,
        allows: impl Fn(ChannelKind) -> bool,
    ) -> ChannelScope {
        ChannelScope {
            swarm,
            viewer,
            allowed: ChannelKind::all().filter(|kind| allows(*kind)).collect(),
        }
    }

    pub fn swarm(&self) -> &SwarmId {
        &self.swarm
    }

    pub fn viewer(&self) -> Option<&str> {
        self.viewer.as_deref()
    }

    pub fn allowed(&self) -> &[ChannelKind] {
        &self.allowed
    }
}

/// Which channels [`Reads::channel_ids`] returns, in name order.
#[derive(Clone, Debug)]
pub struct ChannelFilter {
    /// Only channels in this scope.
    pub scope: ChannelScope,
    /// Only channels the scope's viewer shares with this agent: both are in
    /// them.
    pub shared_with: Option<String>,
    /// Only channels whose name or purpose contains this text.
    pub text: Option<String>,
    /// Only channels after this (name, ID) in name order.
    pub after: Option<(String, String)>,
    pub limit: usize,
}

/// A message as stored, with its channel's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRecord {
    pub id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
    pub created_at: String,
    /// The server run that stored the message. `None` for messages from
    /// before runs were recorded.
    pub run_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct NewMessageRecord {
    pub channel_id: String,
    pub agent: String,
    pub body: String,
    pub created_at: String,
    pub run_id: i64,
}

/// One start of the server and what produced it. See `crate::provenance`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRunRecord {
    pub id: i64,
    pub started_at: String,
    pub version: String,
    pub git_commit: String,
    pub settings: String,
    pub instructions_sha256: String,
}

#[derive(Clone, Debug)]
pub struct NewServerRunRecord {
    pub started_at: String,
    pub version: String,
    pub git_commit: String,
    pub settings: String,
    pub instructions_sha256: String,
}

/// Which messages of a channel to return, and in what order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRange {
    /// The newest, newest first.
    Latest,
    /// IDs above this one, oldest first.
    After(i64),
    /// IDs below this one, newest first.
    Before(i64),
}

/// Which messages [`Reads::newer_messages`] and [`Reads::count_newer_messages`]
/// cover.
#[derive(Clone, Debug)]
pub struct NewerFilter {
    /// Messages in channels in this scope.
    pub scope: ChannelScope,
    pub after_id: i64,
    /// Only these channels, if given.
    pub channel_ids: Option<Vec<String>>,
}

/// Which messages [`Reads::latest_message_from`] looks at.
#[derive(Clone, Debug)]
pub struct SentFilter {
    /// Messages this agent sent.
    pub agent: String,
    /// Only in this channel, if given.
    pub channel_id: Option<String>,
    /// Only in channels in this scope.
    pub scope: ChannelScope,
}

#[derive(Clone, Debug)]
pub struct SearchFilter {
    /// Messages in channels in this scope.
    pub scope: ChannelScope,
    /// Words that must all appear. Search syntax in them has no effect.
    pub words: Vec<String>,
    /// Words that must appear together and in this order, if given. Search
    /// syntax in it has no effect. At least one of `words` and `phrase` must
    /// hold a word.
    pub phrase: Option<String>,
    pub channel_id: Option<String>,
    pub from_agent: Option<String>,
    /// Only messages with IDs above this, if given.
    pub after_id: Option<i64>,
    /// Only messages with IDs below this, if given.
    pub before_id: Option<i64>,
    /// Only messages stored at or after this stored time, if given.
    pub since: Option<String>,
    /// Only messages stored before this stored time, if given.
    pub until: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHitRecord {
    pub message_id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub created_at: String,
    /// The matching part of the body, with matches in `**`.
    pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityRecord {
    pub channel_id: String,
    pub channel_name: String,
    pub new_messages: i64,
    pub latest_id: i64,
}

/// How much a channel has been used: for the human's dashboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelUseRecord {
    pub channel_id: String,
    pub message_count: i64,
    /// When the newest message was sent, and by whom; `None` with no messages.
    pub last_message_at: Option<String>,
    pub last_speaker: Option<String>,
}

/// When a message was sent, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageTimeRecord {
    pub channel_id: String,
    pub created_at: String,
}

/// One agent's membership of one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipRecord {
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
}

/// One immutable save of the server announcement. Kept in the
/// `operator_instructions` table, its name before the rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAnnouncementRecord {
    pub version: i64,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent that saved it, such as `admin-sentinel`; `None` when
    /// the human operator did.
    pub saved_by_admin: Option<String>,
}

/// One immutable save of a lane: its description and instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneVersionRecord {
    /// Numbered from 1 within the lane.
    pub version: i64,
    pub description: String,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent that saved it, such as `admin-sentinel`; `None` when
    /// the human operator did.
    pub saved_by_admin: Option<String>,
    /// Where it was imported from; `None` when it was saved on the web page
    /// or at `/mcp-admin`.
    pub source: Option<LaneSourceRecord>,
}

/// The folder a lane version was imported from, and the git commit checked
/// out there, if it was a git checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneSourceRecord {
    pub folder: String,
    pub commit: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewLaneVersionRecord {
    pub lane: String,
    pub description: String,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent saving it; `None` for the human operator.
    pub saved_by_admin: Option<String>,
    /// Where it is imported from; `None` unless it is.
    pub source: Option<LaneSourceRecord>,
}

/// A lane with its newest version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneRecord {
    pub name: String,
    pub created_at: String,
    /// The server run that created it.
    pub run_id: i64,
    /// Set once the lane is retired.
    pub retired: Option<LaneRetiredRecord>,
    pub current: LaneVersionRecord,
}

/// When a lane was retired, and by whom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneRetiredRecord {
    pub at: String,
    pub run_id: i64,
    /// The admin agent that retired it; `None` when the human operator did.
    pub by_admin: Option<String>,
}

/// Questions the rules can ask.
pub trait Reads {
    fn current_server_announcement(&self) -> StoreResult<Option<ServerAnnouncementRecord>>;
    /// Newest version first.
    fn server_announcement_history(&self) -> StoreResult<Vec<ServerAnnouncementRecord>>;

    // ---- lanes ----

    /// Every lane, retired ones included, in name order.
    fn lanes(&self) -> StoreResult<Vec<LaneRecord>>;

    /// At most `limit` lanes, retired ones included, in name order, after
    /// the lane named `after` if given.
    fn lanes_after(&self, after: Option<&str>, limit: usize) -> StoreResult<Vec<LaneRecord>>;

    /// One lane, retired or not, by its exact name.
    fn lane(&self, name: &str) -> StoreResult<Option<LaneRecord>>;

    /// Every version of one lane, newest first. Empty if there is no such
    /// lane.
    fn lane_history(&self, name: &str) -> StoreResult<Vec<LaneVersionRecord>>;

    /// One version of one lane, retired or not.
    fn lane_version(&self, name: &str, version: i64) -> StoreResult<Option<LaneVersionRecord>>;

    /// As [`Reads::lanes_after`], but only lanes that are not retired.
    fn takeable_lanes_after(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<LaneRecord>>;

    // ---- agents ----

    /// A non-hidden avatar, by its handle (the `name` column).
    fn agent(&self, swarm: &SwarmId, name: &str) -> StoreResult<Option<AgentRecord>>;

    /// Whether a handle is reserved by any avatar, including hidden ones,
    /// ignoring ASCII case.
    fn agent_name_taken(&self, name: &str) -> StoreResult<bool>;

    fn agent_by_token_hash(&self, swarm: &SwarmId, hash: &[u8])
    -> StoreResult<Option<AgentRecord>>;

    /// Looks up an agent by both its name and hashed recovery code.
    fn agent_by_recovery_hash(
        &self,
        swarm: &SwarmId,
        name: &str,
        hash: &[u8],
    ) -> StoreResult<Option<AgentRecord>>;

    /// The entry named `name` in the memory of the avatar `agent`, hidden or
    /// not, if there is one.
    fn memory_entry(
        &self,
        swarm: &SwarmId,
        agent: &str,
        name: &str,
    ) -> StoreResult<Option<MemoryEntryRecord>>;

    /// Every entry in the memory of the avatar `agent`, hidden or not, in
    /// name order.
    fn memory_entries(&self, swarm: &SwarmId, agent: &str) -> StoreResult<Vec<MemoryEntryRecord>>;

    /// Up to `limit` agents in name order, after `after_name` if given, whose
    /// names or descriptions contain `text` if given, and that hold the lane
    /// named `lane` if given.
    fn agents(
        &self,
        swarm: &SwarmId,
        text: Option<&str>,
        lane: Option<&str>,
        after_name: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AgentRecord>>;

    /// How many agents have not been revoked.
    fn active_agent_count(&self, swarm: &SwarmId) -> StoreResult<i64>;

    /// Every agent, most recently seen first.
    fn agents_by_last_seen(&self, swarm: &SwarmId) -> StoreResult<Vec<AgentRecord>>;

    // ---- channels ----

    fn channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<Option<ChannelFacts>>;

    /// Whether an ID is reserved, including by a hidden channel.
    fn channel_id_taken(&self, id: &str) -> StoreResult<bool>;

    fn is_member(&self, swarm: &SwarmId, channel_id: &str, agent: &str) -> StoreResult<bool>;

    /// IDs of the channels `filter` selects, in name order.
    fn channel_ids(&self, filter: &ChannelFilter) -> StoreResult<Vec<String>>;

    /// IDs of every channel in name order, or only those `member` is in.
    fn all_channel_ids(&self, swarm: &SwarmId, member: Option<&str>) -> StoreResult<Vec<String>>;

    /// IDs of the channels `owner` owns, in name order, whether or not it is
    /// still a member.
    fn owned_channel_ids(&self, swarm: &SwarmId, owner: &str) -> StoreResult<Vec<String>>;

    /// IDs of every hidden channel, in name order.
    fn hidden_channel_ids(&self, swarm: &SwarmId) -> StoreResult<Vec<String>>;

    /// One channel as `viewer` sees it. `after_id` asks for a count of newer messages.
    fn channel_summary(
        &self,
        swarm: &SwarmId,
        id: &str,
        viewer: Option<&str>,
        after_id: Option<i64>,
    ) -> StoreResult<Option<ChannelSummary>>;

    /// One hidden channel, with no viewer; `None` if no hidden channel has
    /// this ID. [`Reads::channel_summary`] never returns a hidden channel.
    fn hidden_channel_summary(
        &self,
        swarm: &SwarmId,
        id: &str,
    ) -> StoreResult<Option<ChannelSummary>>;

    /// Up to `limit` member names, in the order they joined.
    fn members_by_join(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<String>>;

    fn member_count(&self, swarm: &SwarmId, channel_id: &str) -> StoreResult<i64>;

    /// Up to `limit` members in name order, after `after_name` if given.
    fn members_by_name(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        after_name: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AgentRecord>>;

    // ---- messages ----

    fn message(&self, swarm: &SwarmId, id: i64) -> StoreResult<Option<MessageRecord>>;

    /// Up to `limit` messages of one channel, optionally from one agent, in
    /// the order `range` gives.
    fn channel_messages(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        from_agent: Option<&str>,
        range: MessageRange,
        limit: usize,
    ) -> StoreResult<Vec<MessageRecord>>;

    /// Up to `limit` messages `filter` covers, oldest first.
    fn newer_messages(&self, filter: &NewerFilter, limit: usize)
    -> StoreResult<Vec<MessageRecord>>;

    fn count_newer_messages(&self, filter: &NewerFilter) -> StoreResult<i64>;

    /// The newest message `filter` covers, if any.
    fn latest_message_from(&self, filter: &SentFilter) -> StoreResult<Option<MessageRecord>>;

    /// Up to `limit` messages across all channels, newest first.
    fn recent_messages(&self, swarm: &SwarmId, limit: usize) -> StoreResult<Vec<MessageRecord>>;

    fn message_count_from(&self, swarm: &SwarmId, agent: &str) -> StoreResult<i64>;

    /// Messages that contain every word, newest first.
    fn search(&self, filter: &SearchFilter) -> StoreResult<Vec<SearchHitRecord>>;

    /// For each channel in `scope` with messages above `after_id`: how many,
    /// and the newest ID. The channel with the newest message comes first.
    fn activity(&self, scope: &ChannelScope, after_id: i64) -> StoreResult<Vec<ActivityRecord>>;

    /// For every channel: how many messages it has, and its newest one's
    /// time and sender. In no particular order.
    fn channel_use(&self, swarm: &SwarmId) -> StoreResult<Vec<ChannelUseRecord>>;

    /// Every membership, in the order agents joined.
    fn memberships(&self, swarm: &SwarmId) -> StoreResult<Vec<MembershipRecord>>;

    /// The channel and time of every message sent at or after `since` (a
    /// stored time), oldest first. It reads only those messages, through an
    /// index on the time, however large the table.
    fn message_times_since(
        &self,
        swarm: &SwarmId,
        since: &str,
    ) -> StoreResult<Vec<MessageTimeRecord>>;

    // ---- server runs ----

    /// Every server run, newest first.
    fn server_runs(&self) -> StoreResult<Vec<ServerRunRecord>>;

    // ---- every channel, hidden ones included, for the admin endpoint ----

    /// One channel, hidden or not, with no viewer, and whether it is hidden.
    fn any_channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<Option<AnyChannelRecord>>;

    /// IDs of every channel, hidden ones included, whose name or purpose
    /// contains `text` if given, after `after` (name, ID) if given, in name
    /// order.
    fn any_channel_ids(
        &self,
        swarm: &SwarmId,
        text: Option<&str>,
        after: Option<&(String, String)>,
        limit: usize,
    ) -> StoreResult<Vec<String>>;

    /// As [`Reads::activity`], for every channel, hidden ones included.
    fn any_activity(&self, swarm: &SwarmId, after_id: i64) -> StoreResult<Vec<ActivityRecord>>;

    /// Up to `limit` messages with IDs above `after_id` in any channel,
    /// hidden ones included, or only in `channel_ids` if given; oldest first.
    fn any_newer_messages(
        &self,
        swarm: &SwarmId,
        after_id: i64,
        channel_ids: Option<&[String]>,
        limit: usize,
    ) -> StoreResult<Vec<MessageRecord>>;

    /// How many messages [`Reads::any_newer_messages`] covers, with no limit.
    fn count_any_newer_messages(
        &self,
        swarm: &SwarmId,
        after_id: i64,
        channel_ids: Option<&[String]>,
    ) -> StoreResult<i64>;
}

/// Changes the rules can make, on top of what they can ask.
pub trait Writes: Reads {
    /// Saves a new version, by the admin agent named `saved_by_admin`, or by
    /// the human operator when it is `None`.
    fn append_server_announcement(
        &self,
        text: &str,
        at: &str,
        run_id: i64,
        saved_by_admin: Option<&str>,
    ) -> StoreResult<ServerAnnouncementRecord>;

    /// Adds a lane with no versions yet. Add its first version in the same
    /// write with [`Writes::append_lane_version`].
    fn insert_lane(&self, name: &str, at: &str, run_id: i64) -> StoreResult<()>;

    /// Saves the lane's next version: one more than its newest, or 1.
    fn append_lane_version(&self, version: &NewLaneVersionRecord)
    -> StoreResult<LaneVersionRecord>;

    /// Marks the lane retired.
    fn retire_lane(&self, name: &str, retired: &LaneRetiredRecord) -> StoreResult<()>;

    /// Adds a swarm, made at `at` by the server run `run_id`. It starts
    /// with no avatars or channels. A swarm is never deleted.
    fn insert_swarm(&self, swarm: &SwarmId, at: &str, run_id: i64) -> StoreResult<()>;

    fn insert_agent(&self, swarm: &SwarmId, agent: &NewAgentRecord) -> StoreResult<()>;

    /// Replaces the authentication token, keeping the avatar.
    fn set_token_hash(&self, swarm: &SwarmId, name: &str, hash: &[u8]) -> StoreResult<()>;

    /// Replaces the hashed recovery code; the old code stops working.
    fn set_recovery_hash(&self, swarm: &SwarmId, name: &str, hash: &[u8]) -> StoreResult<()>;

    /// Saves `entry` in the memory of the avatar `agent`, replacing any
    /// entry with its name.
    fn save_memory_entry(
        &self,
        swarm: &SwarmId,
        agent: &str,
        entry: &MemoryEntryRecord,
    ) -> StoreResult<()>;

    /// Deletes the entry named `name` from the memory of the avatar `agent`.
    /// False if there was none.
    fn delete_memory_entry(&self, swarm: &SwarmId, agent: &str, name: &str) -> StoreResult<bool>;

    fn set_last_seen(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()>;

    /// Replaces the agent's description. `None` clears it.
    fn set_description(
        &self,
        swarm: &SwarmId,
        name: &str,
        description: Option<&str>,
    ) -> StoreResult<()>;
    /// Replaces the last observed MCP client, or clears it when unavailable.
    fn set_agent_client(
        &self,
        swarm: &SwarmId,
        name: &str,
        client: Option<&AgentClient>,
    ) -> StoreResult<()>;

    /// Replaces the lane the agent holds. `None` drops it.
    fn set_agent_lane(
        &self,
        swarm: &SwarmId,
        name: &str,
        lane: Option<&TakenLaneRecord>,
    ) -> StoreResult<()>;

    /// Marks the agent's token as revoked at `at`.
    fn set_revoked(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()>;

    fn insert_channel(&self, swarm: &SwarmId, channel: &NewChannelRecord) -> StoreResult<()>;

    fn set_purpose(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        purpose: Option<&str>,
    ) -> StoreResult<()>;

    fn set_channel_open(&self, swarm: &SwarmId, channel_id: &str, open: bool) -> StoreResult<()>;

    /// Hides a channel and removes its memberships, retaining its messages.
    fn hide_channel(&self, swarm: &SwarmId, channel_id: &str) -> StoreResult<()>;

    /// Replaces the channel's owner. `None` leaves it with no owner.
    fn set_owner(&self, swarm: &SwarmId, channel_id: &str, owner: Option<&str>) -> StoreResult<()>;

    /// Hides and revokes an avatar, removing its memberships and channel
    /// ownerships but retaining messages.
    fn hide_agent(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()>;

    fn set_note(&self, swarm: &SwarmId, channel_id: &str, edit: &NoteEditRecord)
    -> StoreResult<()>;

    /// Adds a member. Adding one already there does nothing.
    fn add_member(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        agent: &str,
        joined_at: &str,
    ) -> StoreResult<()>;

    fn remove_member(&self, swarm: &SwarmId, channel_id: &str, agent: &str) -> StoreResult<()>;

    /// Stores a message and returns its new ID.
    fn insert_message(&self, swarm: &SwarmId, message: &NewMessageRecord) -> StoreResult<i64>;

    /// Records a server start and returns its new ID.
    fn insert_server_run(&self, run: &NewServerRunRecord) -> StoreResult<i64>;
}
