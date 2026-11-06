//! Open waits and the short grace after normal completion. Never persisted.
use super::*;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

#[derive(Default)]
pub struct Waiting(Mutex<State>);
#[derive(Default)]
struct State {
    next: u64,
    calls: HashMap<u64, Call>,
}
struct Call {
    agent: String,
    channels: Option<Vec<String>>,
    until: Option<Time>,
}
pub(super) struct Guard<'a> {
    waiting: &'a Waiting,
    id: u64,
    completed: bool,
}
impl Waiting {
    pub(super) fn start(&self, agent: &str, channels: Option<Vec<String>>, now: Time) -> Guard<'_> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state
            .calls
            .retain(|_, c| c.until.is_none_or(|end| c.agent != agent && end > now));
        state.next += 1;
        let id = state.next;
        state.calls.insert(
            id,
            Call {
                agent: agent.into(),
                channels,
                until: None,
            },
        );
        Guard {
            waiting: self,
            id,
            completed: false,
        }
    }
    /// Whether `agent` has an open wait, or one in its grace after finishing.
    pub(super) fn has_wait(&self, agent: &str, now: Time) -> bool {
        !self.filters(agent, now).is_empty()
    }
    fn filters(&self, agent: &str, now: Time) -> Vec<Option<Vec<String>>> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state
            .calls
            .retain(|_, c| c.until.is_none_or(|end| end > now));
        state
            .calls
            .values()
            .filter(|c| c.agent == agent)
            .map(|c| c.channels.clone())
            .collect()
    }
}
impl Guard<'_> {
    pub(super) fn finish(mut self, until: Time) {
        let mut state = self.waiting.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(call) = state.calls.get_mut(&self.id) {
            call.until = Some(until);
        }
        self.completed = true;
    }
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.waiting
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .calls
                .remove(&self.id);
        }
    }
}
/// When `agent` last sent a message `viewer` can see: in `channel` if given,
/// else in any channel. `viewer` is `None` for the human and admin agents,
/// who see every channel, so for them it is the plain value.
pub(super) fn last_sent(
    store: &dyn Reads,
    agent: &str,
    channel: Option<&str>,
    viewer: Option<&str>,
) -> Result<Option<String>> {
    Ok(store
        .latest_message_from(&SentFilter {
            agent: agent.to_string(),
            channel_id: channel.map(str::to_string),
            visible_to: viewer.map(str::to_string),
        })?
        .map(|m| m.created_at))
}

impl Rules {
    /// Adds only channels the viewer can see, still joined by the waiting
    /// agent, and the agent's latest message the viewer can see: in
    /// `channel` if given, else in any channel. `viewer` is `None` for the
    /// human and admin agents, who see every channel.
    pub(super) fn observed_agent(
        &self,
        store: &dyn Reads,
        record: AgentRecord,
        viewer: Option<&str>,
        channel: Option<&str>,
    ) -> Result<AgentInfo> {
        let filters = self
            .inner
            .waiting
            .filters(&record.name, self.inner.deps.clock.now());
        let mut ids = BTreeSet::new();
        for filter in filters {
            let channels = match filter {
                Some(ids) => ids,
                None => store.all_channel_ids(Some(&record.name))?,
            };
            for id in channels {
                let Some(facts) = store.channel(&id)? else {
                    continue;
                };
                if !store.is_member(&id, &record.name)? {
                    continue;
                }
                let who = match viewer {
                    None => Who::Human,
                    // Seeing needs no ownership.
                    Some(handle) => Who::Agent {
                        member: store.is_member(&id, handle)?,
                        owner: false,
                    },
                };
                let channel = access::Channel::Live { open: facts.open };
                let visible = access::decide(channel, who, Need::See) == Decision::Allowed;
                if visible {
                    ids.insert(id);
                }
            }
        }
        let last_sent = last_sent(store, &record.name, channel, viewer)?;
        let mut info = AgentInfo::from(record);
        info.last_sent = last_sent;
        info.waiting_channels = ids.into_iter().collect();
        info.listening = channel.map_or(!info.waiting_channels.is_empty(), |id| {
            info.waiting_channels.iter().any(|c| c == id)
        });
        Ok(info)
    }
}
