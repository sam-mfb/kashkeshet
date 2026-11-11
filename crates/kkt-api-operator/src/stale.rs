use super::*;

/// An agent that has made no authenticated call for longer than the period
/// asked about, and has no open wait.
#[derive(Clone, Debug)]
pub struct StaleAgent {
    pub info: AgentInfo,
    /// The channels it is in, in name order.
    pub channels: Vec<ChannelRef>,
    /// How many entries its memory holds.
    pub memory_entries: usize,
    /// The channels it owns, in name order. An owner is never hidden as
    /// stale.
    pub owns: Vec<ChannelRef>,
}

/// What hiding stale agents did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleHidden {
    /// The agents hidden, in the order asked for.
    pub hidden: Vec<String>,
    /// The agents asked for but kept, and why, in the order asked for.
    pub kept: Vec<Kept>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub handle: String,
    pub reason: KeptReason,
}

/// Why an agent the human chose was not hidden as stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptReason {
    /// No such agent, or it is already hidden.
    Gone,
    /// It has made a call within the period, or has an open wait.
    Active,
    /// It owns a channel.
    Owner,
}
