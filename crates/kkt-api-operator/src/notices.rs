use super::*;
use std::future::Future;
use std::pin::Pin;

/// What the operator hears as it happens in its swarm: a new message, a new
/// agent or channel, an agent joining or leaving a channel, and an agent or
/// channel hidden. Closed channels' notices are heard too. [`Notice::Missed`]
/// says the listener fell behind and missed some, not how many.
///
/// The records serialize with their fields in the order written here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Message(Message),
    Agent(NewAgent),
    Channel(ChannelCreated),
    Joined(Joined),
    Left(Left),
    ChannelHidden(ChannelHidden),
    AgentHidden(AgentHidden),
    /// The listener fell behind and missed some notices. What they were
    /// about can be read again; the notice says no number.
    Missed,
}

/// An agent registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NewAgent {
    pub handle: String,
    pub created_at: String,
}

/// A channel was created, open or closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChannelCreated {
    pub id: String,
    pub name: String,
    pub open: bool,
    pub created_by: String,
    /// The creator and anyone added at creation.
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
    /// `by` is the operator.
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
    /// `by` is the operator.
    pub by_operator: bool,
}

/// An agent unregistered or was hidden by the operator, leaving every
/// channel it was in.
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

/// The next notice, or `None` once there will be no more.
pub type NextNotice<'a> = Pin<Box<dyn Future<Output = Option<Notice>> + Send + 'a>>;

/// One listener's notices, from when it subscribed ([`Views::subscribe`]).
/// Dropping it stops listening.
pub trait Notices: Send {
    /// The next notice in the swarm, or `None` once there will be no more:
    /// the server is stopping, or the subscription was ended.
    fn next(&mut self) -> NextNotice<'_>;
}
