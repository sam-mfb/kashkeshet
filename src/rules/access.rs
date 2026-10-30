//! Who may use a channel: the rules table from the module docs, as one pure
//! function.

/// What is stored under a channel ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Channel {
    /// No channel has this ID.
    Missing,
    /// A hidden channel: its messages are kept, but only the human may read
    /// them, and nobody may change it.
    Hidden,
    /// A channel in use, open or closed.
    Live { open: bool },
}

/// What a call needs from a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Need {
    /// To see it: list its members, or join it. For the human, also to read
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
    /// The human operator, who sees every channel and may change any of
    /// them without being in it.
    Human,
    /// An admin agent at `/mcp-admin`, acting for the operator: it may use
    /// every channel as the human may, and read hidden ones as the human
    /// does.
    Admin,
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
        // it; the human and the admin agent may still read it.
        Channel::Hidden => {
            return if matches!(who, Who::Human | Who::Admin) && need == Need::See {
                Decision::Allowed
            } else {
                Decision::NotFound
            };
        }
        Channel::Live { open } => open,
    };
    let (member, owner) = match who {
        Who::Human | Who::Admin => return Decision::Allowed,
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
        let human = Who::Human;
        let admin = Who::Admin;
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
            (closed, human, Ownership, Allowed),
            (Missing, human, Ownership, NotFound),
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
            (Missing, human, See, NotFound),
            (Missing, human, Membership, NotFound),
            (open, human, See, Allowed),
            (open, human, Membership, Allowed),
            (closed, human, See, Allowed),
            (closed, human, Membership, Allowed),
            // A hidden channel: to agents, former members and owners
            // included, it does not exist. The human may read it, but
            // nobody may change it.
            (Hidden, agent(false), See, NotFound),
            (Hidden, agent(true), See, NotFound),
            (Hidden, agent(true), Membership, NotFound),
            (Hidden, owner(true), Ownership, NotFound),
            (Hidden, human, See, Allowed),
            (Hidden, human, Membership, NotFound),
            (Hidden, human, Ownership, NotFound),
            // The admin agent: as the human, open, closed or hidden.
            (Missing, admin, See, NotFound),
            (Missing, admin, Membership, NotFound),
            (open, admin, See, Allowed),
            (closed, admin, See, Allowed),
            (closed, admin, Membership, Allowed),
            (closed, admin, Ownership, Allowed),
            (Hidden, admin, See, Allowed),
            (Hidden, admin, Membership, NotFound),
            (Hidden, admin, Ownership, NotFound),
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
