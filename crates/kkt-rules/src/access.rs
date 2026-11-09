//! Who may use a channel: the rules table from the module docs, as one pure
//! function, and the same rule as a scope the store's queries apply.

use kkt_store::{ChannelScope, SwarmId};

/// What is stored under a channel ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Channel {
    /// No channel has this ID.
    Missing,
    /// A hidden channel: its messages are kept, but only the operator may read
    /// them, and nobody may change it.
    Hidden,
    /// A channel in use, open or closed.
    Live { open: bool },
}

/// What a call needs from a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Need {
    /// To see it: list its members, or join it. For the operator, also to read
    /// it, hidden or not.
    See,
    /// To be in it: read, send, set the purpose, add others, leave.
    Membership,
    /// To be in it and own it: remove others, pass the ownership on.
    Ownership,
}

/// Who is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Who {
    /// An agent, whether it is in the channel, and whether it owns it.
    Agent { member: bool, owner: bool },
    /// The operator, on the web page or at `/mcp-admin`: it sees every
    /// channel, reads hidden ones, and may change any live one without being
    /// in it. Whether this operator may read or change at all is checked
    /// before, on its permissions.
    Operator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Allowed,
    /// The channel does not exist, or is closed and the caller is not in it.
    /// The two must look the same.
    NotFound,
    /// The channel is open and the caller is not in it.
    NotMember,
    /// The caller is in the channel but does not own it.
    NotOwner,
}

pub(super) fn decide(channel: Channel, who: Who, need: Need) -> Decision {
    let open = match channel {
        Channel::Missing => return Decision::NotFound,
        // To agents a hidden channel does not exist, and nobody may change
        // it; the operator may still read it.
        Channel::Hidden => {
            return if who == Who::Operator && need == Need::See {
                Decision::Allowed
            } else {
                Decision::NotFound
            };
        }
        Channel::Live { open } => open,
    };
    let (member, owner) = match who {
        Who::Operator => return Decision::Allowed,
        Who::Agent { member, owner } => (member, owner),
    };
    if !open && !member {
        return Decision::NotFound;
    }
    match (need, member, owner) {
        (Need::See, _, _) => Decision::Allowed,
        (Need::Membership | Need::Ownership, false, _) => Decision::NotMember,
        (Need::Ownership, true, false) => Decision::NotOwner,
        _ => Decision::Allowed,
    }
}

/// Who asks a query that covers many channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Viewer<'a> {
    /// The agent with this handle.
    Agent(&'a str),
    /// The operator.
    Operator,
}

/// The channels of `swarm` that `viewer` may use for `need`, as a scope for
/// the store's queries: [`decide`], asked about every kind of channel the
/// store tells apart.
pub(super) fn scope(swarm: &SwarmId, viewer: Viewer, need: Need) -> ChannelScope {
    let handle = match viewer {
        Viewer::Agent(handle) => Some(handle.to_string()),
        Viewer::Operator => None,
    };
    ChannelScope::new(swarm.clone(), handle, |kind| {
        let channel = if kind.hidden {
            Channel::Hidden
        } else {
            Channel::Live { open: kind.open }
        };
        let who = match viewer {
            Viewer::Agent(_) => Who::Agent {
                member: kind.member,
                owner: kind.owner,
            },
            Viewer::Operator => Who::Operator,
        };
        decide(channel, who, need) == Decision::Allowed
    })
}

#[cfg(test)]
mod tests {
    use super::Channel::*;
    use super::Decision::*;
    use super::Need::*;
    use super::*;

    #[test]
    fn decisions() {
        let agent = |member| Who::Agent {
            member,
            owner: false,
        };
        let owner = |member| Who::Agent {
            member,
            owner: true,
        };
        let open = Live { open: true };
        let closed = Live { open: false };
        let operator = Who::Operator;
        // (channel, who, need) -> decision
        let table = [
            (Missing, owner(false), Ownership, NotFound),
            (open, agent(false), Ownership, NotMember),
            (open, agent(true), Ownership, NotOwner),
            (open, owner(false), Ownership, NotMember),
            (open, owner(true), Ownership, Allowed),
            (closed, agent(false), Ownership, NotFound),
            (closed, agent(true), Ownership, NotOwner),
            // An owner that has left a closed channel is an outsider to it.
            (closed, owner(false), Ownership, NotFound),
            (closed, owner(true), Ownership, Allowed),
            (closed, operator, Ownership, Allowed),
            (Missing, operator, Ownership, NotFound),
            (Missing, agent(false), See, NotFound),
            (Missing, agent(false), Membership, NotFound),
            (open, agent(false), See, Allowed),
            (open, agent(false), Membership, NotMember),
            (open, agent(true), See, Allowed),
            (open, agent(true), Membership, Allowed),
            (closed, agent(false), See, NotFound),
            (closed, agent(false), Membership, NotFound),
            (closed, agent(true), See, Allowed),
            (closed, agent(true), Membership, Allowed),
            (Missing, operator, See, NotFound),
            (Missing, operator, Membership, NotFound),
            (open, operator, See, Allowed),
            (open, operator, Membership, Allowed),
            (closed, operator, See, Allowed),
            (closed, operator, Membership, Allowed),
            // A hidden channel: to agents, former members and owners
            // included, it does not exist. The operator may read it, but
            // nobody may change it.
            (Hidden, agent(false), See, NotFound),
            (Hidden, agent(true), See, NotFound),
            (Hidden, agent(true), Membership, NotFound),
            (Hidden, owner(true), Ownership, NotFound),
            (Hidden, operator, See, Allowed),
            (Hidden, operator, Membership, NotFound),
            (Hidden, operator, Ownership, NotFound),
            // The admin agent's rows, from before the human and the admin
            // agent were one operator: the same answers.
            (Missing, operator, See, NotFound),
            (Missing, operator, Membership, NotFound),
            (open, operator, See, Allowed),
            (closed, operator, See, Allowed),
            (closed, operator, Membership, Allowed),
            (closed, operator, Ownership, Allowed),
            (Hidden, operator, See, Allowed),
            (Hidden, operator, Membership, NotFound),
            (Hidden, operator, Ownership, NotFound),
        ];
        for (channel, who, need, want) in table {
            assert_eq!(
                decide(channel, who, need),
                want,
                "channel={channel:?} who={who:?} need={need:?}"
            );
        }
    }
}
