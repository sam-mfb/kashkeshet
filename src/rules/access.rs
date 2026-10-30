//! Who may use a channel: the rules table from the module docs, as one pure
//! function.

/// What a call needs from a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Need {
    /// To see it: list its members, or join it.
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
    /// An admin agent at `/mcp-admin`, acting for the operator: it sees
    /// every channel, hidden ones included (the caller passes `exists` true
    /// for those), and may use any of them as the human may.
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

pub(super) fn decide(exists: bool, open: bool, who: Who, need: Need) -> Decision {
    let (member, owner) = match who {
        Who::Human | Who::Admin => {
            return if exists {
                Decision::Allowed
            } else {
                Decision::NotFound
            };
        }
        Who::Agent { member, owner } => (member, owner),
    };
    if !exists || (!open && !member) {
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
        // (exists, open, who, need) -> decision
        let table = [
            (false, false, owner(false), Ownership, NotFound),
            (true, true, agent(false), Ownership, NotMember),
            (true, true, agent(true), Ownership, NotOwner),
            (true, true, owner(false), Ownership, NotMember),
            (true, true, owner(true), Ownership, Allowed),
            (true, false, agent(false), Ownership, NotFound),
            (true, false, agent(true), Ownership, NotOwner),
            // An owner that has left a closed channel is an outsider to it.
            (true, false, owner(false), Ownership, NotFound),
            (true, false, owner(true), Ownership, Allowed),
            (true, false, Who::Human, Ownership, Allowed),
            (false, true, Who::Human, Ownership, NotFound),
            (false, false, agent(false), See, NotFound),
            (false, false, agent(false), Membership, NotFound),
            (true, true, agent(false), See, Allowed),
            (true, true, agent(false), Membership, NotMember),
            (true, true, agent(true), See, Allowed),
            (true, true, agent(true), Membership, Allowed),
            (true, false, agent(false), See, NotFound),
            (true, false, agent(false), Membership, NotFound),
            (true, false, agent(true), See, Allowed),
            (true, false, agent(true), Membership, Allowed),
            (false, false, Who::Human, See, NotFound),
            (false, true, Who::Human, Membership, NotFound),
            (true, true, Who::Human, See, Allowed),
            (true, true, Who::Human, Membership, Allowed),
            (true, false, Who::Human, See, Allowed),
            (true, false, Who::Human, Membership, Allowed),
            (false, false, Who::Admin, See, NotFound),
            (false, true, Who::Admin, Membership, NotFound),
            (true, true, Who::Admin, See, Allowed),
            (true, false, Who::Admin, See, Allowed),
            (true, false, Who::Admin, Membership, Allowed),
            (true, false, Who::Admin, Ownership, Allowed),
        ];
        for (exists, open, who, need, want) in table {
            assert_eq!(
                decide(exists, open, who, need),
                want,
                "exists={exists} open={open} who={who:?} need={need:?}"
            );
        }
    }
}
