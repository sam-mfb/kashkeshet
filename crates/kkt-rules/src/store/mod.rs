//! Storage: what the rules can ask of the database and change in it.
//!
//! Two types carry it: [`Reader`] answers questions, and [`Writer`] adds the
//! changes, with the questions through [`Reader`]. Neither decides anything
//! about permissions: the rules ask, for
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
//! swarm. So do handles and channel IDs: [`Reader::agent_name_taken`] and
//! [`Reader::channel_id_taken`] look across every swarm.
//!
//! [`Db`] runs closures over a [`kkt_db::Backend`], which the program
//! opens and passes in: the store knows no particular database. Each call
//! to [`Db::read`] or [`Db::write`] runs one closure; a read sees one
//! state, and a write closure is one transaction whose changes all commit
//! together, or not at all. [`schema`] gives the core's tables as kkt-db
//! migrations, to open a database with.

mod queries;
mod schema;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use schema::{CORE, legacy, schema};

use crate::ServerRunInfo;
use kkt_api_agent::{ActivityRow, Message, SearchHit, ServerAnnouncement, SwarmId};
use kkt_db::{Backend, BackendExt, DbError};
pub use queries::{Reader, Writer};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(String),
    #[error("database task failed: {0}")]
    Task(String),
    #[error("database lock poisoned")]
    Poisoned,
    /// A change would tie a row to an avatar or channel outside the swarm
    /// it was made in, such as a message in another swarm's channel.
    /// Nothing was changed. The rules look everything up in their swarm
    /// first, so for them this is a bug, not a refusal to report.
    #[error("{0}")]
    OutsideSwarm(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

impl From<DbError> for StoreError {
    fn from(e: DbError) -> StoreError {
        match e {
            DbError::Task(e) => StoreError::Task(e),
            e => StoreError::Database(e.to_string()),
        }
    }
}

/// A database the store reads and changes. Cloning shares it.
#[derive(Clone)]
pub struct Db {
    database: Arc<dyn Backend>,
    /// For [`testing::fail_next_write`]: the backend of a store
    /// [`testing::memory`] made.
    #[cfg(any(test, feature = "testing"))]
    faults: Option<kkt_db::testing::FaultyBackend>,
}

/// A closure's own error, or the database's.
enum Failed<E> {
    Database(DbError),
    Closure(E),
}

impl<E> From<DbError> for Failed<E> {
    fn from(e: DbError) -> Failed<E> {
        Failed::Database(e)
    }
}

impl<E: From<StoreError>> Failed<E> {
    fn into_error(self) -> E {
        match self {
            Failed::Database(e) => E::from(StoreError::from(e)),
            Failed::Closure(e) => e,
        }
    }
}

impl Db {
    /// The store over `database`, which must hold the tables [`schema`]
    /// declares.
    pub fn new(database: Arc<dyn Backend>) -> Db {
        Db {
            database,
            #[cfg(any(test, feature = "testing"))]
            faults: None,
        }
    }

    /// Runs `f` over a view of one state of the database.
    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Reader<'_>) -> Result<T, E> + Send + 'static,
    {
        BackendExt::read(self.database.as_ref(), move |c| {
            f(&Reader(c)).map_err(Failed::Closure)
        })
        .await
        .map_err(Failed::into_error)
    }

    /// Runs `f` inside one transaction. It commits only if `f` succeeds.
    pub async fn write<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Writer<'_>) -> Result<T, E> + Send + 'static,
    {
        BackendExt::write(self.database.as_ref(), move |c| {
            f(&Writer(c, Reader(c))).map_err(Failed::Closure)
        })
        .await
        .map_err(Failed::into_error)
    }
}

/// The ID of [`SwarmId::main`], as the core's second migration writes it.
/// A test checks the two agree.
pub(crate) const MAIN_SWARM: &str = "sw_main";

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

/// Which channels [`Reader::channel_ids`] returns, in name order.
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

#[derive(Clone, Debug)]
pub struct NewMessageRecord {
    pub channel_id: String,
    pub agent: String,
    pub body: String,
    pub created_at: String,
    pub run_id: i64,
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

/// Which messages [`Reader::newer_messages`] and [`Reader::count_newer_messages`]
/// cover.
#[derive(Clone, Debug)]
pub struct NewerFilter {
    /// Messages in channels in this scope.
    pub scope: ChannelScope,
    pub after_id: i64,
    /// Only these channels, if given.
    pub channel_ids: Option<Vec<String>>,
}

/// Which messages [`Reader::latest_message_from`] looks at.
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
