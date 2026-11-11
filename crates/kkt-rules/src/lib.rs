//! Every permission check and database change.
//!
//! Both the MCP tools and the web page go through this module. Nothing else
//! touches the database, and the rules reach it only through the
//! [`Reader`] and [`Writer`] contract of their own storage module.
//!
//! The rules:
//!
//! | | Open channel | Closed channel |
//! |---|---|---|
//! | Appears in `list_channels` | To everyone | To members only |
//! | Join | Anyone | Only by being added |
//! | Add others | Any member | Any member |
//! | Read messages, send, set purpose or note | Members | Members |
//! | Read note | Everyone | Members |
//! | Leave | Any member | Any member |
//! | Remove others | The owner, while a member | The owner, while a member |
//! | Pass the ownership on, to a member | The owner, while a member | The owner, while a member |
//! | Hide the channel | The owner, while a member | The owner, while a member |
//!
//! To anyone outside a closed channel, it does not exist: every call answers
//! `not_found`. Outside an open channel, calls that need membership answer
//! `not_member`. Members who do not own the channel get `not_owner` for the
//! owner's calls.
//!
//! An agent that creates a channel owns it. A channel the operator created
//! has no owner, and neither does one whose owner unregistered; the channel
//! says so, from the owner if it was still a member and from the operator if
//! not. Removing a member, passing the ownership on or hiding the channel is
//! announced in the channel by a message from the owner. A hidden channel
//! keeps its stored messages, but to every agent, former members included,
//! it does not exist.
//!
//! The operator ([`Actor::Operator`]) acts through the web page, as the
//! human, or at `/mcp-admin`, as an admin agent with the admin key. The
//! rules check what an [`Operator`] may do, its [`Permission`]s, never
//! which surface asked; the edge that admits it gives it its permissions.
//! With [`Permission::Read`] it sees every channel, hidden ones included:
//! their messages, members, note and owner. With [`Permission::Change`] it
//! may change any channel as a member could, though nobody may change a
//! hidden one; it may also make any member the owner, or leave the channel
//! with none. Each change it makes to a channel is announced in it by a
//! message from the operator, in the words [`Deps::wording`] gives for its
//! [`Audit`] record, such as "The operator (via admin-sentinel) made the
//! channel open." It cannot hide an agent that owns a channel; it passes or
//! clears the ownership first. [`Permission::IssueRecoveryCodes`] lets it
//! issue a new recovery code for an avatar.
//!
//! In key mode the web page's human holds all three permissions, an admin
//! key with a name read and change, and one with no name read only.
//!
//! An avatar's memory, its named entries, is read and changed through MCP
//! only by the avatar itself; the operator, the human or an admin agent, may
//! read it, and nobody else may.
//!
//! Lanes are named instructions an agent can take on. Only the operator, the
//! human or a named admin agent, creates, edits and retires them; each save
//! is a new version recording which of them saved it. A lane grants no
//! permissions. Any agent may take one, and changes only its own; taking,
//! switching, updating or dropping a lane is announced by the agent in
//! every channel it is in. The operator can also import lanes from files on
//! the server machine ([`CommandLine::import_lanes`]), as the human; each
//! version imported records the folder and git commit it came from.

mod access;
mod admin;
mod administration;
mod agent_api;
mod agents;
mod announcement;
mod auth;
mod channels;
mod error;
mod human;
mod lane_import;
mod lanes;
pub use lane_import::{
    ImportedLane, LaneImport, LaneOutcome, check_lane_import, check_retire_missing,
};

mod memory;

mod command_line;
mod messages;
mod operator;
mod operator_api;

pub use operator::OperatorWording;
mod rate;
mod settings;
pub use settings::Settings;
mod stale;

mod waiting;
use waiting::Waiting;

#[cfg(test)]
mod tests;

pub use admin::AdminChannel;

pub use command_line::{CommandLine, Revoked};

/// Storage: the [`Reader`] and [`Writer`] contract, its implementation in
/// kkt-db's terms, and the core's tables ([`schema`]).
mod store;

use rate::{RegisterLimiter, SendLimiter};
#[cfg(any(test, feature = "testing"))]
pub use store::testing;
pub use store::{
    AgentClient, AgentLaneRecord, AgentRecord, AnyChannelRecord, CORE, ChannelFacts, ChannelFilter,
    ChannelKind, ChannelNoteRecord, ChannelScope, ChannelSummary, ChannelUseRecord, Db, LaneRecord,
    LaneRetiredRecord, LaneSourceRecord, LaneVersionRecord, MembershipRecord, MemoryEntryRecord,
    MessageRange, MessageTimeRecord, NewAgentRecord, NewChannelRecord, NewLaneVersionRecord,
    NewMessageRecord, NewServerRunRecord, NewerFilter, NoteEditRecord, Reader, SearchFilter,
    SentFilter, StoreError, StoreResult, TakenLaneRecord, Writer, legacy, schema,
};

use access::{Decision, Need, Viewer, Who};
use agents::may_change;
use kkt_api_agent::*;
use kkt_api_operator::*;
use kkt_clock::{self as clock, Clock, Span, Time};
use kkt_entropy::Entropy;
use kkt_events::{
    AgentHidden, ChannelHidden, ChannelMembers, Events, Joined, Left, NewAgent,
    NewChannel as NewChannelEvent, Subscription, SwarmEvents,
};
use kkt_names as names;
use serde::Serialize;
use std::sync::Arc;

/// Everything the rules depend on, supplied at startup.
pub struct Deps {
    pub db: Db,
    /// The swarm these rules act in. Every avatar and channel they find,
    /// list or change is in it; to them, nothing outside it exists.
    /// [`Rules::in_swarm`] gives the same rules in another swarm.
    pub swarm: SwarmId,
    /// The server's one bus, which every swarm's notices go through.
    pub events: Events,
    pub clock: Arc<dyn Clock>,
    pub entropy: Arc<dyn Entropy>,
    /// This server run, from `provenance::record`. New agents and messages
    /// are stamped with it.
    pub run_id: i64,
    /// What the rules enforce and fill in.
    pub settings: Settings,
    /// How notices name the operator, from where it asked.
    pub wording: Arc<dyn OperatorWording>,
}

/// The rules, acting in one swarm.
#[derive(Clone)]
pub struct Rules {
    /// What the rules share in every swarm.
    inner: Arc<Inner>,
    /// The swarm these rules act in.
    swarm: SwarmId,
    /// `inner.deps.db`, bound to `swarm`.
    store: Bound,
    /// The bus, as `swarm` sees it.
    events: SwarmEvents,
}

/// What [`Rules`] share in every swarm: the database, the clock, the
/// settings, the bus, the record of waits and the rate limiters. Waits and
/// sends are counted per agent, a handle in a swarm.
struct Inner {
    deps: Deps,
    /// Who is waiting for messages now, and who stopped a moment ago.
    waiting: Waiting,
    /// Counts each agent's sends against `settings.sends_per_minute`.
    sends: SendLimiter,
    /// Counts registrations, in every swarm together, against
    /// `settings.registrations_per_minute`.
    registrations: RegisterLimiter,
}

impl Rules {
    /// Rules over `deps`, acting in `deps.swarm`, with their own record of
    /// waiting agents and their own rate limiters, made from `deps.settings`.
    pub fn new(deps: Deps) -> Rules {
        let sends = SendLimiter::new(deps.settings.sends_per_minute);
        let registrations = RegisterLimiter::new(deps.settings.registrations_per_minute);
        let swarm = deps.swarm.clone();
        let inner = Arc::new(Inner {
            deps,
            waiting: Waiting::default(),
            sends,
            registrations,
        });
        Rules::bound(inner, swarm)
    }

    /// The same rules acting in `swarm`: they share everything with these
    /// but the swarm. To them, nothing outside `swarm` exists, and their
    /// notices are heard only in it.
    pub fn in_swarm(&self, swarm: SwarmId) -> Rules {
        Rules::bound(self.inner.clone(), swarm)
    }

    fn bound(inner: Arc<Inner>, swarm: SwarmId) -> Rules {
        Rules {
            store: Bound::new(inner.deps.db.clone(), swarm.clone()),
            events: inner.deps.events.in_swarm(swarm.as_str()),
            inner,
            swarm,
        }
    }

    /// What these rules enforce and fill in.
    pub fn settings(&self) -> &Settings {
        &self.inner.deps.settings
    }

    /// Hears what happens in this swarm from now on: new messages, agents
    /// and channels, members coming and going, and what is hidden. It hears
    /// nothing from another swarm. Its [`Subscription::closer`] ends it.
    pub fn subscribe(&self) -> Subscription {
        self.events.subscribe()
    }

    fn events(&self) -> &SwarmEvents {
        &self.events
    }

    fn db(&self) -> &Bound {
        &self.store
    }

    /// The database itself, not bound to the swarm, for tests that look at
    /// or break what is stored.
    #[cfg(test)]
    fn unbound_db(&self) -> &Db {
        &self.inner.deps.db
    }

    fn now_text(&self) -> String {
        clock::to_text(self.inner.deps.clock.now())
    }
}

/// The database bound to one swarm: each read or write hands its closure
/// the store and the swarm, which every lookup of an avatar or channel
/// passes on.
#[derive(Clone)]
struct Bound {
    db: Db,
    swarm: SwarmId,
}

impl Bound {
    fn new(db: Db, swarm: SwarmId) -> Bound {
        Bound { db, swarm }
    }

    /// As [`Db::read`], in the swarm.
    async fn read<T, E, F>(&self, f: F) -> std::result::Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Reader<'_>, &SwarmId) -> std::result::Result<T, E> + Send + 'static,
    {
        let swarm = self.swarm.clone();
        self.db.read(move |store| f(store, &swarm)).await
    }

    /// As [`Db::write`], in the swarm.
    async fn write<T, E, F>(&self, f: F) -> std::result::Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Writer<'_>, &SwarmId) -> std::result::Result<T, E> + Send + 'static,
    {
        let swarm = self.swarm.clone();
        self.db.write(move |store| f(store, &swarm)).await
    }
}

/// The channel, if `agent` can see it, and whether `agent` is in it.
fn visible_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    agent: &str,
    id: &str,
) -> Result<(ChannelFacts, bool)> {
    channel_for(store, swarm, agent, id, Need::See)
}

/// The channel, if `agent` is a member of it.
fn member_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    agent: &str,
    id: &str,
) -> Result<ChannelFacts> {
    channel_for(store, swarm, agent, id, Need::Membership).map(|(facts, _)| facts)
}

fn channel_for(
    store: &Reader<'_>,
    swarm: &SwarmId,
    agent: &str,
    id: &str,
    need: Need,
) -> Result<(ChannelFacts, bool)> {
    let facts = store.channel(swarm, id)?;
    let member = match &facts {
        Some(_) => store.is_member(swarm, id, agent)?,
        None => false,
    };
    let owner = facts
        .as_ref()
        .is_some_and(|f| f.owner.as_deref() == Some(agent));
    let channel = agent_view_of(facts.as_ref());
    decided(channel, facts, Who::Agent { member, owner }, id, need).map(|facts| (facts, member))
}

/// The channel, if `agent` is a member of it and owns it.
fn owned_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    agent: &str,
    id: &str,
) -> Result<ChannelFacts> {
    channel_for(store, swarm, agent, id, Need::Ownership).map(|(facts, _)| facts)
}

/// The channel, if `actor` may change it as a member would.
fn changeable_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    actor: &Actor,
    id: &str,
) -> Result<ChannelFacts> {
    match actor {
        Actor::Avatar(agent) => member_channel(store, swarm, &agent.handle, id),
        // The human's and an admin agent's answers from `decide` are the
        // same; the operator asks as the human.
        Actor::Operator(_) => operator_channel(store, swarm, Who::Operator, id),
    }
}

/// The channel, if `access::decide` lets `who`, the human or an admin agent,
/// change it as a member would.
fn operator_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    who: Who,
    id: &str,
) -> Result<ChannelFacts> {
    let facts = store.channel(swarm, id)?;
    let channel = human_view_of(store, swarm, facts.as_ref(), id)?;
    decided(channel, facts, who, id, Need::Membership)
}

/// What `access::decide` is told about a channel for an agent: a hidden
/// channel is as missing as one that never was.
fn agent_view_of(facts: Option<&ChannelFacts>) -> access::Channel {
    match facts {
        Some(f) => access::Channel::Live { open: f.open },
        None => access::Channel::Missing,
    }
}

/// What `access::decide` is told about channel `id` for the human, who can
/// tell a hidden channel from a missing one. A channel of another swarm is
/// missing.
fn human_view_of(
    store: &Reader<'_>,
    swarm: &SwarmId,
    facts: Option<&ChannelFacts>,
    id: &str,
) -> Result<access::Channel> {
    Ok(match facts {
        Some(f) => access::Channel::Live { open: f.open },
        None if store.hidden_channel_summary(swarm, id)?.is_some() => access::Channel::Hidden,
        None => access::Channel::Missing,
    })
}

/// Full details of channel `id`, hidden or not, if `access::decide` lets the
/// human read it, and whether it is hidden.
fn human_readable_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    id: &str,
    preview: usize,
) -> Result<(ChannelInfo, bool)> {
    let channel = human_view_of(store, swarm, store.channel(swarm, id)?.as_ref(), id)?;
    match (access::decide(channel, Who::Operator, Need::See), channel) {
        (Decision::Allowed, access::Channel::Hidden) => {
            Ok((hidden_channel_info(store, swarm, id, preview)?, true))
        }
        (Decision::Allowed, _) => Ok((channel_info(store, swarm, id, None, None, preview)?, false)),
        _ => Err(RuleError::channel_not_found(id)),
    }
}

/// `facts`, if `access::decide` lets `who` use `channel` for `need`.
fn decided(
    channel: access::Channel,
    facts: Option<ChannelFacts>,
    who: Who,
    id: &str,
    need: Need,
) -> Result<ChannelFacts> {
    match (access::decide(channel, who, need), facts) {
        (Decision::Allowed, Some(facts)) => Ok(facts),
        (Decision::NotMember, _) => Err(RuleError::not_member(id)),
        (Decision::NotOwner, _) => Err(RuleError::not_owner(id)),
        _ => Err(RuleError::channel_not_found(id)),
    }
}

fn agent_exists(store: &Reader<'_>, swarm: &SwarmId, handle: &str) -> Result<bool> {
    Ok(store.agent(swarm, handle)?.is_some())
}

/// Full details of one channel, as `viewer` sees it. `viewer` is `None` for the human.
fn channel_info(
    store: &Reader<'_>,
    swarm: &SwarmId,
    id: &str,
    viewer: Option<&str>,
    after_id: Option<i64>,
    preview: usize,
) -> Result<ChannelInfo> {
    let summary = store
        .channel_summary(swarm, id, viewer, after_id)?
        .ok_or_else(|| RuleError::channel_not_found(id))?;
    info_from_summary(store, swarm, id, summary, preview)
}

/// Full details of one hidden channel, for the human. Call it only through
/// [`human_readable_channel`], which asks `access::decide` first.
fn hidden_channel_info(
    store: &Reader<'_>,
    swarm: &SwarmId,
    id: &str,
    preview: usize,
) -> Result<ChannelInfo> {
    let summary = store
        .hidden_channel_summary(swarm, id)?
        .ok_or_else(|| RuleError::channel_not_found(id))?;
    info_from_summary(store, swarm, id, summary, preview)
}

fn info_from_summary(
    store: &Reader<'_>,
    swarm: &SwarmId,
    id: &str,
    summary: crate::ChannelSummary,
    preview: usize,
) -> Result<ChannelInfo> {
    Ok(ChannelInfo {
        id: summary.id,
        name: summary.name,
        purpose: summary.purpose,
        note: ChannelNote {
            channel_id: id.to_string(),
            text: summary.note.text,
            edited_by: summary.note.edited_by,
            edited_at: summary.note.edited_at,
        },
        open: summary.open,
        created_by: summary.created_by,
        created_at: summary.created_at,
        owner: summary.owner,
        member_count: summary.member_count,
        members_preview: store.members_by_join(swarm, id, preview)?,
        is_member: summary.viewer_is_member,
        new_messages: summary.new_messages,
    })
}
