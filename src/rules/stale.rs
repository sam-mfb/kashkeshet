//! Agents that have gone quiet, and hiding them together. The human (or an
//! admin agent) picks how long an agent must have been quiet, sees the list,
//! and chooses which to hide; nothing here happens on its own. Each is hidden by the same rule
//! as hiding one agent. A channel owner is never hidden this way.

use super::administration::{HIDDEN, hidden_body, operator_words};
use super::human::ChannelRef;
use super::*;
use chrono::{DateTime, Utc};
use std::time::Duration;

/// An agent that has made no authenticated call for longer than the period
/// asked about, and has no open wait.
#[derive(Clone, Debug)]
pub struct StaleAgent {
    pub info: AgentInfo,
    /// The channels it is in, in name order.
    pub channels: Vec<ChannelRef>,
    /// Whether its handoff note has any text.
    pub has_handoff_note: bool,
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
    pub name: String,
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

impl Rules {
    /// Every agent that has made no authenticated call for longer than
    /// `idle` and has no open wait, the longest quiet first. Owners are
    /// listed too, with the channels they own. `invalid` if `idle` is zero
    /// or reaches before any time the clock can name.
    pub async fn stale_agents(&self, idle: Duration) -> Result<Vec<StaleAgent>> {
        let cutoff = self.stale_cutoff(idle)?;
        let rules = self.clone();
        self.db()
            .read(move |store| {
                let mut stale = Vec::new();
                // Most recently seen first, so reversed: the longest quiet first.
                for record in store.agents_by_last_seen()?.into_iter().rev() {
                    if rules.is_stale(&record, cutoff) {
                        stale.push(stale_agent(store, record)?);
                    }
                }
                Ok(stale)
            })
            .await
    }

    /// Hides each agent in `chosen` that is still stale for `idle` and owns
    /// no channel, as [`Rules::hide_agent`] does, all in one write. The
    /// others are kept, with the reason. Only the human or an admin agent
    /// may do this.
    pub async fn hide_stale_agents(
        &self,
        actor: Actor,
        idle: Duration,
        chosen: Vec<String>,
    ) -> Result<StaleHidden> {
        let who = operator_words(&actor)?;
        let cutoff = self.stale_cutoff(idle)?;
        let rules = self.clone();
        let now = self.now_text();
        let (outcome, notices) = self
            .db()
            .write(move |store| {
                let mut outcome = StaleHidden {
                    hidden: Vec::new(),
                    kept: Vec::new(),
                };
                let mut notices = Vec::new();
                for name in chosen {
                    let seen = outcome.hidden.contains(&name)
                        || outcome.kept.iter().any(|k| k.name == name);
                    if seen {
                        continue;
                    }
                    let reason = match store.agent(&name)? {
                        None => Some(KeptReason::Gone),
                        Some(record) if !rules.is_stale(&record, cutoff) => {
                            Some(KeptReason::Active)
                        }
                        Some(_) if !store.owned_channel_ids(&name)?.is_empty() => {
                            Some(KeptReason::Owner)
                        }
                        Some(_) => None,
                    };
                    match reason {
                        Some(reason) => outcome.kept.push(Kept { name, reason }),
                        None => {
                            notices.extend(rules.remove_identity_in(
                                store,
                                &name,
                                names::OPERATOR,
                                &hidden_body(&who, &name),
                                HIDDEN,
                                &now,
                            )?);
                            outcome.hidden.push(name);
                        }
                    }
                }
                Ok::<_, RuleError>((outcome, notices))
            })
            .await?;
        for notice in &notices {
            self.publish_message(notice);
        }
        Ok(outcome)
    }

    /// The time an agent's last call must be before to count as stale.
    fn stale_cutoff(&self, idle: Duration) -> Result<DateTime<Utc>> {
        if idle.is_zero() {
            return Err(RuleError::invalid(
                "the idle period must be longer than zero",
            ));
        }
        chrono::Duration::from_std(idle)
            .ok()
            .and_then(|d| self.inner.deps.clock.now().checked_sub_signed(d))
            .ok_or_else(|| RuleError::invalid("the idle period is too long"))
    }

    /// Whether `record` made its last call before `cutoff` and has no wait.
    /// An agent that never called counts from when it registered. A time
    /// that cannot be read counts as not stale.
    fn is_stale(&self, record: &AgentRecord, cutoff: DateTime<Utc>) -> bool {
        let last_call = record.last_seen.as_deref().unwrap_or(&record.created_at);
        let quiet = clock::from_text(last_call).is_some_and(|at| at < cutoff);
        quiet
            && !self
                .inner
                .deps
                .waiting
                .has_wait(&record.name, self.inner.deps.clock.now())
    }
}

fn stale_agent(store: &dyn Reads, record: AgentRecord) -> Result<StaleAgent> {
    let name = record.name.clone();
    Ok(StaleAgent {
        channels: channel_refs(store, store.all_channel_ids(Some(&name))?)?,
        owns: channel_refs(store, store.owned_channel_ids(&name)?)?,
        has_handoff_note: store
            .handoff_note(&name)?
            .is_some_and(|note| !note.text.is_empty()),
        info: AgentInfo::from(record),
    })
}

fn channel_refs(store: &dyn Reads, ids: Vec<String>) -> Result<Vec<ChannelRef>> {
    let mut refs = Vec::new();
    for id in ids {
        if let Some(facts) = store.channel(&id)? {
            refs.push(ChannelRef {
                id: facts.id,
                name: facts.name,
            });
        }
    }
    Ok(refs)
}
