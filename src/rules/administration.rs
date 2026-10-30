//! The operator's channel and identity administration, made by the human or
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
        let rules = self.clone();
        let now = self.now_text();
        let notice = self
            .db()
            .write(move |store| {
                // Use the shared access decision even when an agent attempts an
                // operator-only action, so closed channels remain undiscoverable.
                let channel = changeable_channel(store, &actor, &channel_id)?;
                let who = operator_words(&actor)?;
                let body = match change {
                    ChannelChange::SetOpen(open) => {
                        if channel.open == open {
                            return Ok(None);
                        }
                        store.set_channel_open(&channel_id, open)?;
                        format!(
                            "{who} made the channel {}.",
                            if open { "open" } else { "closed" }
                        )
                    }
                    ChannelChange::AddMember(name) => {
                        let name = name.trim();
                        if !agent_exists(store, name)? {
                            return Err(RuleError::agent_not_found(name));
                        }
                        if store.is_member(&channel_id, name)? {
                            return Ok(None);
                        }
                        store.add_member(&channel_id, name, &now)?;
                        format!("{who} added {name} to the channel.")
                    }
                    ChannelChange::RemoveMember(name) => {
                        let name = name.trim();
                        if !agent_exists(store, name)? {
                            return Err(RuleError::agent_not_found(name));
                        }
                        if !store.is_member(&channel_id, name)? {
                            return Ok(None);
                        }
                        store.remove_member(&channel_id, name)?;
                        format!("{who} removed {name} from the channel.")
                    }
                    ChannelChange::SetOwner(Some(name)) => {
                        let name = name.trim();
                        if !agent_exists(store, name)? {
                            return Err(RuleError::agent_not_found(name));
                        }
                        if !store.is_member(&channel_id, name)? {
                            return Err(RuleError::invalid(format!(
                                "{name} is not a member of {channel_id}; add it first"
                            )));
                        }
                        if channel.owner.as_deref() == Some(name) {
                            return Ok(None);
                        }
                        store.set_owner(&channel_id, Some(name))?;
                        format!("{who} made {name} the owner of the channel.")
                    }
                    ChannelChange::SetOwner(None) => {
                        let Some(old) = channel.owner.as_deref() else {
                            return Ok(None);
                        };
                        store.set_owner(&channel_id, None)?;
                        format!(
                            "{who} removed {old} as the owner of the channel. \
                             The channel no longer has an owner."
                        )
                    }
                    ChannelChange::Hide => {
                        return rules.hide(store, &channel, names::OPERATOR, &who).map(Some);
                    }
                };
                Ok(Some(rules.post_as_operator(store, &channel, body)?))
            })
            .await?;
        if let Some(notice) = notice {
            self.publish_message(&notice);
        }
        Ok(())
    }

    /// Hides `channel` and removes its memberships, keeping its messages,
    /// after `author` says so in it, as `who`. The caller has decided that
    /// `author` may hide it, and runs this inside that decision's write.
    pub(super) fn hide(
        &self,
        store: &dyn Writes,
        channel: &ChannelFacts,
        author: &str,
        who: &str,
    ) -> Result<Message> {
        let body = format!("{who} hid the channel. Its messages are kept.");
        let notice = self.post_notice(store, channel, author, body)?;
        store.hide_channel(&channel.id)?;
        Ok(notice)
    }

    /// Hides and revokes `name`, as the human or an admin agent. An agent
    /// that owns a channel is refused, checked in the same write: the
    /// operator passes the ownership on or clears it first
    /// ([`ChannelChange::SetOwner`]).
    pub async fn hide_agent(&self, actor: Actor, name: String) -> Result<()> {
        let who = operator_words(&actor)?;
        let first = match actor {
            Actor::Admin(_) => "with `set_owner` or `clear_owner`",
            _ => "on each channel's page",
        };
        let rules = self.clone();
        let now = self.now_text();
        let notices = self
            .db()
            .write(move |store| {
                let owned = store.owned_channel_ids(&name)?;
                if !owned.is_empty() {
                    return Err(RuleError::invalid(format!(
                        "{name} owns {}; make another member the owner, or clear the \
                         owner, {first} first",
                        owned.join(", ")
                    )));
                }
                rules.remove_identity_in(
                    store,
                    &name,
                    names::OPERATOR,
                    &hidden_body(&who, &name),
                    HIDDEN,
                    &now,
                )
            })
            .await?;
        for notice in notices {
            self.publish_message(&notice);
        }
        Ok(())
    }

    /// Removes `me`'s own identity, as the operator's hide does: its token
    /// stops working, it cannot be reclaimed, it leaves every channel and
    /// drops out of agent lists. It says so itself in each channel it was in.
    /// Its messages are kept.
    pub async fn unregister(&self, me: &Agent) -> Result<()> {
        let body = format!(
            "{} unregistered and left the channel. Its messages are kept.",
            me.name
        );
        self.remove_identity(me.name.clone(), me.name.clone(), body, "unregistered")
            .await
    }

    /// Hides and revokes `name`, removing its memberships and ownerships.
    /// Before that, in the same write, `author` posts `body` in each channel
    /// `name` was in, and the operator explains, in each channel `name` owns
    /// but had left, that `name` `what` and the channel has no owner now.
    async fn remove_identity(
        &self,
        name: String,
        author: String,
        body: String,
        what: &'static str,
    ) -> Result<()> {
        let rules = self.clone();
        let now = self.now_text();
        let notices = self
            .db()
            .write(move |store| rules.remove_identity_in(store, &name, &author, &body, what, &now))
            .await?;
        for notice in notices {
            self.publish_message(&notice);
        }
        Ok(())
    }

    /// [`Rules::remove_identity`]'s change, inside a write the caller holds.
    /// Returns the messages it posted, for the caller to publish once the
    /// write is done.
    pub(super) fn remove_identity_in(
        &self,
        store: &dyn Writes,
        name: &str,
        author: &str,
        body: &str,
        what: &str,
        now: &str,
    ) -> Result<Vec<Message>> {
        if !agent_exists(store, name)? {
            return Err(RuleError::agent_not_found(name));
        }
        let mut notices = Vec::new();
        let member_of = store.all_channel_ids(Some(name))?;
        let owned = store.owned_channel_ids(name)?;
        for id in &member_of {
            let channel = changeable_channel(store, &Actor::Human, id)?;
            let text = if owned.contains(id) {
                format!("{body} The channel no longer has an owner.")
            } else {
                body.to_string()
            };
            notices.push(self.post_notice(store, &channel, author, text)?);
        }
        for id in owned.iter().filter(|id| !member_of.contains(id)) {
            let channel = changeable_channel(store, &Actor::Human, id)?;
            let text =
                format!("{name} owned the channel and {what}. The channel no longer has an owner.");
            notices.push(self.post_as_operator(store, &channel, text)?);
        }
        store.hide_agent(name, now)?;
        Ok(notices)
    }
}

/// What the operator says in each channel of an agent it hides, as `who`
/// (see [`operator_words`]).
pub(super) fn hidden_body(who: &str, name: &str) -> String {
    format!("{who} hid {name} and revoked its access. Its messages are kept.")
}

/// How the operator's explanation, in a channel the agent owned but had
/// left, says what happened to an agent it hid.
pub(super) const HIDDEN: &str = "was hidden";

/// Who an operator's change is announced as: "The operator" for the human,
/// "The operator (via admin-sentinel)" for the admin agent of that name.
/// Agents may not make the operator's changes.
pub(super) fn operator_words(actor: &Actor) -> Result<String> {
    match actor {
        Actor::Human => Ok("The operator".into()),
        Actor::Admin(name) => Ok(format!("The operator (via {name})")),
        Actor::Agent(_) => Err(RuleError::new(
            Code::Unauthenticated,
            "only the human operator may administer channels and identities",
        )),
    }
}
