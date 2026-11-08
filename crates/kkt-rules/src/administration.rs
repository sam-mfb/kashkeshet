//! The operator's channel and avatar administration, made by the human or
//! an admin agent. Every channel change and the message explaining it commit
//! together.

use super::*;

#[derive(Clone, Debug)]
pub enum ChannelChange {
    SetOpen(bool),
    AddMember(String),
    RemoveMember(String),
    /// Makes the named member the owner, or with `None` leaves the channel
    /// with no owner.
    SetOwner(Option<String>),
    Hide,
}

impl Rules {
    pub async fn change_channel(
        &self,
        actor: Actor,
        channel_id: String,
        change: ChannelChange,
    ) -> Result<()> {
        actor.may_change()?;
        let rules = self.clone();
        let now = self.now_text();
        let changed = self
            .db()
            .write(move |store, swarm| {
                // Use the shared access decision even when an agent attempts an
                // operator-only action, so closed channels remain undiscoverable.
                let channel = changeable_channel(store, swarm, &actor, &channel_id)?;
                let who = rules.operator_words(&actor)?;
                let mut event = None;
                let body = match change {
                    ChannelChange::SetOpen(open) => {
                        if channel.open == open {
                            return Ok(None);
                        }
                        store.set_channel_open(swarm, &channel_id, open)?;
                        format!(
                            "{who} made the channel {}.",
                            if open { "open" } else { "closed" }
                        )
                    }
                    ChannelChange::AddMember(handle) => {
                        let handle = handle.trim();
                        if !agent_exists(store, swarm, handle)? {
                            return Err(RuleError::agent_not_found(handle));
                        }
                        if store.is_member(swarm, &channel_id, handle)? {
                            return Ok(None);
                        }
                        store.add_member(swarm, &channel_id, handle, &now)?;
                        // Being added is joining, as for `add_to_channel`.
                        event = Some(Announced::Joined(Joined {
                            agent: handle.to_string(),
                            channel_id: channel.id.clone(),
                            channel_name: channel.name.clone(),
                            member_count: store.member_count(swarm, &channel_id)?,
                        }));
                        format!("{who} added {handle} to the channel.")
                    }
                    ChannelChange::RemoveMember(handle) => {
                        let handle = handle.trim();
                        if !agent_exists(store, swarm, handle)? {
                            return Err(RuleError::agent_not_found(handle));
                        }
                        if !store.is_member(swarm, &channel_id, handle)? {
                            return Ok(None);
                        }
                        store.remove_member(swarm, &channel_id, handle)?;
                        event = Some(Announced::Left(Left {
                            agent: handle.to_string(),
                            channel_id: channel.id.clone(),
                            channel_name: channel.name.clone(),
                            member_count: store.member_count(swarm, &channel_id)?,
                            by: names::OPERATOR.into(),
                            by_operator: true,
                        }));
                        format!("{who} removed {handle} from the channel.")
                    }
                    ChannelChange::SetOwner(Some(handle)) => {
                        let handle = handle.trim();
                        if !agent_exists(store, swarm, handle)? {
                            return Err(RuleError::agent_not_found(handle));
                        }
                        if !store.is_member(swarm, &channel_id, handle)? {
                            return Err(RuleError::invalid(format!(
                                "{handle} is not a member of {channel_id}; add it first"
                            )));
                        }
                        if channel.owner.as_deref() == Some(handle) {
                            return Ok(None);
                        }
                        store.set_owner(swarm, &channel_id, Some(handle))?;
                        format!("{who} made {handle} the owner of the channel.")
                    }
                    ChannelChange::SetOwner(None) => {
                        let Some(old) = channel.owner.as_deref() else {
                            return Ok(None);
                        };
                        store.set_owner(swarm, &channel_id, None)?;
                        format!(
                            "{who} removed {old} as the owner of the channel. \
                             The channel no longer has an owner."
                        )
                    }
                    ChannelChange::Hide => {
                        let notice = rules.hide(store, swarm, &channel, names::OPERATOR, &who)?;
                        let hidden = Announced::ChannelHidden(ChannelHidden {
                            channel_id: channel.id.clone(),
                            channel_name: channel.name.clone(),
                            by: names::OPERATOR.into(),
                            by_operator: true,
                        });
                        return Ok(Some((notice, Some(hidden))));
                    }
                };
                Ok(Some((
                    rules.post_as_operator(store, swarm, &channel, body)?,
                    event,
                )))
            })
            .await?;
        if let Some((notice, event)) = changed {
            self.publish_message(&notice);
            match event {
                Some(Announced::Joined(joined)) => self.events().publish_joined(joined),
                Some(Announced::Left(left)) => self.events().publish_left(left),
                Some(Announced::ChannelHidden(hidden)) => {
                    self.events().publish_channel_hidden(hidden)
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Hides `channel` and removes its memberships, keeping its messages,
    /// after `author` says so in it, as `who`. The caller has decided that
    /// `author` may hide it, and runs this inside that decision's write.
    pub(super) fn hide(
        &self,
        store: &dyn Writes,
        swarm: &SwarmId,
        channel: &ChannelFacts,
        author: &str,
        who: &str,
    ) -> Result<Message> {
        let body = format!("{who} hid the channel. Its messages are kept.");
        let notice = self.post_notice(store, swarm, channel, author, body)?;
        store.hide_channel(swarm, &channel.id)?;
        Ok(notice)
    }

    /// Hides and revokes `handle`, as the operator. An agent that owns a
    /// channel is refused, checked in the same write, with
    /// [`Refusal::OwnsChannels`], which each edge words with how to pass
    /// the ownership on or clear it first ([`ChannelChange::SetOwner`]).
    pub async fn hide_agent(&self, actor: Actor, handle: String) -> Result<()> {
        actor.may_change()?;
        let who = self.operator_words(&actor)?;
        let rules = self.clone();
        let now = self.now_text();
        let removed = self
            .db()
            .write(move |store, swarm| {
                let owned = store.owned_channel_ids(swarm, &handle)?;
                if !owned.is_empty() {
                    let refused = RuleError::invalid(format!(
                        "{handle} owns {}; make another member the owner, or clear the \
                         owner, first",
                        owned.join(", ")
                    ));
                    return Err(refused.because(Refusal::OwnsChannels {
                        handle: handle.clone(),
                        channels: owned,
                    }));
                }
                rules.remove_avatar_in(
                    store,
                    swarm,
                    Removal {
                        handle: &handle,
                        author: names::OPERATOR,
                        body: &hidden_body(&who, &handle),
                        what: HIDDEN,
                        now: &now,
                    },
                )
            })
            .await?;
        self.publish_removed(removed);
        Ok(())
    }

    /// Publishes what removing an avatar did: its messages, then the
    /// agent's removal.
    pub(super) fn publish_removed(&self, removed: Removed) {
        for notice in &removed.notices {
            self.publish_message(notice);
        }
        self.events().publish_agent_hidden(removed.event);
    }

    /// Removes `me`'s own avatar, as the operator's hide does: its token
    /// stops working, it cannot be reclaimed, it leaves every channel and
    /// drops out of agent lists. It says so itself in each channel it was in.
    /// Its messages are kept.
    pub async fn unregister(&self, me: &Agent) -> Result<()> {
        let body = format!(
            "{} unregistered and left the channel. Its messages are kept.",
            me.handle
        );
        self.remove_avatar(me.handle.clone(), me.handle.clone(), body, "unregistered")
            .await
    }

    /// Hides and revokes `handle`, removing its memberships and ownerships.
    /// Before that, in the same write, `author` posts `body` in each channel
    /// `handle` was in, and the operator explains, in each channel `handle`
    /// owns but had left, that `handle` `what` and the channel has no owner
    /// now.
    async fn remove_avatar(
        &self,
        handle: String,
        author: String,
        body: String,
        what: &'static str,
    ) -> Result<()> {
        let rules = self.clone();
        let now = self.now_text();
        let removed = self
            .db()
            .write(move |store, swarm| {
                rules.remove_avatar_in(
                    store,
                    swarm,
                    Removal {
                        handle: &handle,
                        author: &author,
                        body: &body,
                        what,
                        now: &now,
                    },
                )
            })
            .await?;
        self.publish_removed(removed);
        Ok(())
    }

    /// [`Rules::remove_avatar`]'s change, inside a write the caller holds.
    /// Returns the messages it posted and the event saying `author` removed
    /// `handle`, for the caller to publish once the write is done.
    pub(super) fn remove_avatar_in(
        &self,
        store: &dyn Writes,
        swarm: &SwarmId,
        removal: Removal,
    ) -> Result<Removed> {
        let Removal {
            handle,
            author,
            body,
            what,
            now,
        } = removal;
        if !agent_exists(store, swarm, handle)? {
            return Err(RuleError::agent_not_found(handle));
        }
        let mut notices = Vec::new();
        let member_of = store.all_channel_ids(swarm, Some(handle))?;
        let owned = store.owned_channel_ids(swarm, handle)?;
        for id in &member_of {
            let channel = operator_channel(store, swarm, Who::Operator, id)?;
            let text = if owned.contains(id) {
                format!("{body} The channel no longer has an owner.")
            } else {
                body.to_string()
            };
            notices.push(self.post_notice(store, swarm, &channel, author, text)?);
        }
        for id in owned.iter().filter(|id| !member_of.contains(id)) {
            let channel = operator_channel(store, swarm, Who::Operator, id)?;
            let text = format!(
                "{handle} owned the channel and {what}. The channel no longer has an owner."
            );
            notices.push(self.post_as_operator(store, swarm, &channel, text)?);
        }
        store.hide_agent(swarm, handle, now)?;
        let mut channels = Vec::new();
        for id in &member_of {
            let channel = operator_channel(store, swarm, Who::Operator, id)?;
            channels.push(ChannelMembers {
                member_count: store.member_count(swarm, &channel.id)?,
                channel_id: channel.id,
                channel_name: channel.name,
            });
        }
        Ok(Removed {
            notices,
            event: AgentHidden {
                agent: handle.to_string(),
                by: author.to_string(),
                by_operator: author == names::OPERATOR,
                channels,
            },
        })
    }
}

/// What removing an avatar did, to publish once its write is done.
pub(super) struct Removed {
    /// The messages posted about it.
    pub notices: Vec<Message>,
    pub event: AgentHidden,
}

/// The event an operator's channel change raises besides its message.
enum Announced {
    Joined(Joined),
    Left(Left),
    ChannelHidden(ChannelHidden),
}

/// Which avatar [`Rules::remove_avatar_in`] removes, who removes it, and
/// what it posts: `body` in each channel the avatar was in, and, in each it
/// owned, that it `what` and the channel has no owner.
pub(super) struct Removal<'a> {
    pub(super) handle: &'a str,
    pub(super) author: &'a str,
    pub(super) body: &'a str,
    pub(super) what: &'a str,
    pub(super) now: &'a str,
}

/// What the operator says in each channel of an agent it hides, as `who`
/// (see [`Rules::operator_words`]).
pub(super) fn hidden_body(who: &str, handle: &str) -> String {
    format!("{who} hid {handle} and revoked its access. Its messages are kept.")
}

/// How the operator's explanation, in a channel the agent owned but had
/// left, says what happened to an agent it hid.
pub(super) const HIDDEN: &str = "was hidden";

impl Rules {
    /// Who an operator's change is announced as, in the words
    /// [`Deps::wording`] gives for where it asked from, such as "The
    /// operator" or "The operator (via admin-sentinel)". Avatars may not
    /// make the operator's changes.
    pub(super) fn operator_words(&self, actor: &Actor) -> Result<String> {
        match actor {
            Actor::Operator(operator) => Ok(self.inner.deps.wording.operator(&operator.audit)),
            Actor::Avatar(_) => Err(RuleError::operator_only("administer channels and avatars")),
        }
    }
}
