//! What the operator sees through the web page: everything, including
//! closed channels, with [`Permission::Read`]. Its changes are the same
//! rules as agents', made as [`Actor::Operator`].

use super::*;
use kkt_config::{WEB_ACTIVITY_BUCKET, WEB_ACTIVITY_WINDOW};
use std::collections::{BTreeMap, HashMap};

impl Rules {
    /// The front page: every channel with how much it is used, every agent
    /// with the channels it is in, and this server run.
    pub async fn overview(&self, operator: &Operator) -> Result<Overview> {
        operator.may(Permission::Read)?;
        let rules = self.clone();
        let run_id = self.inner.deps.run_id;
        let preview = self.limits().member_preview;
        let (since, now) = self.activity_window()?;
        self.db()
            .read(move |store, swarm| {
                let channels = channel_activity(
                    store,
                    swarm,
                    &store.all_channel_ids(swarm, None)?,
                    &since,
                    |id| channel_info(store, swarm, id, None, None, preview),
                )?;
                let mut member_of: HashMap<String, Vec<ChannelRef>> = HashMap::new();
                for m in store.memberships(swarm)? {
                    member_of.entry(m.agent).or_default().push(ChannelRef {
                        id: m.channel_id,
                        name: m.channel_name,
                    });
                }
                let agents = store
                    .agents_by_last_seen(swarm)?
                    .into_iter()
                    .map(|a| {
                        Ok(AgentActivity {
                            channels: member_of.remove(&a.name).unwrap_or_default(),
                            info: rules.observed_agent(store, swarm, a, None, None)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let current_run = store
                    .server_runs()?
                    .into_iter()
                    .find(|r| r.id == run_id)
                    .map(ServerRunInfo::from);
                Ok(Overview {
                    channels,
                    agents,
                    current_run,
                    now,
                })
            })
            .await
    }

    /// The hidden channels, with how much they were used, in name order.
    /// Only the human sees them; to agents they do not exist.
    pub async fn hidden_channels(&self, operator: &Operator) -> Result<Vec<ChannelActivity>> {
        operator.may(Permission::Read)?;
        let preview = self.limits().member_preview;
        let (since, _) = self.activity_window()?;
        self.db()
            .read(move |store, swarm| {
                channel_activity(
                    store,
                    swarm,
                    &store.hidden_channel_ids(swarm)?,
                    &since,
                    |id| human_readable_channel(store, swarm, id, preview).map(|(info, _)| info),
                )
            })
            .await
    }

    /// Every server run, newest first: the server's upgrade history.
    pub async fn server_runs(&self, operator: &Operator) -> Result<Vec<ServerRunInfo>> {
        operator.may(Permission::Read)?;
        self.db()
            .read(|store, _| {
                Ok(store
                    .server_runs()?
                    .into_iter()
                    .map(ServerRunInfo::from)
                    .collect())
            })
            .await
    }

    /// The start of `WEB_ACTIVITY_WINDOW` and now, as stored times.
    fn activity_window(&self) -> Result<(String, String)> {
        let now = self.inner.deps.clock.now();
        let window = Span::from_std(WEB_ACTIVITY_WINDOW)
            .map_err(|e| RuleError::new(Code::Internal, e.to_string()))?;
        Ok((clock::to_text(now - window), clock::to_text(now)))
    }

    /// One channel with a page of its messages, hidden or not: the human
    /// may read a hidden channel, though nobody can change it.
    pub async fn channel_view(
        &self,
        operator: &Operator,
        id: String,
        before_id: Option<i64>,
    ) -> Result<ChannelView> {
        operator.may(Permission::Read)?;
        let rules = self.clone();
        let preview = self.limits().member_preview;
        let page_messages = self.limits().web_page_messages;
        self.db()
            .read(move |store, swarm| {
                let (channel, hidden) = human_readable_channel(store, swarm, &id, preview)?;
                let members = store
                    .members_by_join(swarm, &id, usize::MAX >> 1)?
                    .iter()
                    .map(|handle| {
                        store
                            .agent(swarm, handle)?
                            .ok_or_else(|| RuleError::agent_not_found(handle))
                            .and_then(|a| rules.observed_agent(store, swarm, a, None, Some(&id)))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut latest_messages = Vec::new();
                for m in &members {
                    latest_messages.extend(
                        store
                            .latest_message_from(&SentFilter {
                                agent: m.handle.clone(),
                                channel_id: Some(id.clone()),
                                scope: access::scope(swarm, Viewer::Human, Need::See),
                            })?
                            .map(Message::from),
                    );
                }
                let page = super::messages::read_channel(
                    store,
                    swarm,
                    &ReadQuery {
                        channel_id: id.clone(),
                        position: before_id.map_or(Position::Latest, Position::Before),
                        from_agent: None,
                        limit: page_messages,
                    },
                )?;
                let older_before_id = match page.more {
                    Some(Position::Before(b)) => Some(b),
                    _ => None,
                };
                Ok(ChannelView {
                    channel,
                    hidden,
                    members,
                    latest_messages,
                    messages: page.messages,
                    older_before_id,
                })
            })
            .await
    }

    pub async fn agent_view(&self, operator: &Operator, handle: String) -> Result<AgentView> {
        operator.may(Permission::Read)?;
        let rules = self.clone();
        let preview = self.limits().member_preview;
        self.db()
            .read(move |store, swarm| {
                let agent = store
                    .agent(swarm, &handle)?
                    .ok_or_else(|| RuleError::agent_not_found(&handle))
                    .and_then(|a| rules.observed_agent(store, swarm, a, None, None))?;
                let channels = store
                    .all_channel_ids(swarm, Some(&handle))?
                    .iter()
                    .map(|id| channel_info(store, swarm, id, Some(&handle), None, preview))
                    .collect::<Result<Vec<_>>>()?;
                let message_count = store.message_count_from(swarm, &handle)?;
                Ok(AgentView {
                    memory: super::memory::memory(store, swarm, &handle)?,
                    agent,
                    channels,
                    message_count,
                })
            })
            .await
    }

    /// Gives the agent with the handle `handle` a new recovery code, for the
    /// human to hand to a new session, which calls `reclaim` with it. The old
    /// code stops working; the agent's current token keeps working until a
    /// `reclaim`. Only a hash is stored, so the code is returned once and
    /// never again. A revoked agent cannot be reclaimed, so gets none.
    pub async fn new_recovery_code(&self, operator: &Operator, handle: String) -> Result<String> {
        operator.may(Permission::IssueRecoveryCodes)?;
        let code = auth::recovery_code(self.inner.deps.entropy.as_ref());
        let hash = auth::hash_token(&code);
        self.db()
            .write(move |store, swarm| {
                let agent = store
                    .agent(swarm, &handle)?
                    .ok_or_else(|| RuleError::agent_not_found(&handle))?;
                if agent.revoked_at.is_some() {
                    return Err(RuleError::invalid(format!(
                        "{handle} is revoked, and a revoked avatar cannot be reclaimed"
                    )));
                }
                store.set_recovery_hash(swarm, &handle, &hash)?;
                Ok(())
            })
            .await?;
        Ok(code)
    }
}

/// What the human sees on the front page.
#[derive(Clone, Debug)]
pub struct Overview {
    /// In name order.
    pub channels: Vec<ChannelActivity>,
    /// Most recently seen first.
    pub agents: Vec<AgentActivity>,
    /// This server run, if it is recorded (it always is for a running
    /// server).
    pub current_run: Option<ServerRunInfo>,
    /// The server's time when this was read, so the page can tell how long
    /// ago things happened without trusting the viewer's clock.
    pub now: String,
}

/// A channel and how much it is used.
#[derive(Clone, Debug)]
pub struct ChannelActivity {
    pub info: ChannelInfo,
    pub message_count: i64,
    pub last_message_at: Option<String>,
    pub last_speaker: Option<String>,
    /// The messages of the last `WEB_ACTIVITY_WINDOW`, counted in
    /// stretches of `WEB_ACTIVITY_BUCKET`, oldest first.
    pub recent_activity: Vec<ActivityCount>,
}

/// How many messages a channel got in the stretch starting at `start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityCount {
    pub start: String,
    pub count: i64,
}

/// An agent and the channels it is in.
#[derive(Clone, Debug)]
pub struct AgentActivity {
    pub info: AgentInfo,
    /// In the order it joined them.
    pub channels: Vec<ChannelRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRef {
    pub id: String,
    pub name: String,
}

/// One start of the server and what produced it.
#[derive(Clone, Debug)]
pub struct ServerRunInfo {
    pub id: i64,
    pub started_at: String,
    pub version: String,
    pub git_commit: String,
    pub settings: String,
    pub instructions_sha256: String,
}

impl From<ServerRunRecord> for ServerRunInfo {
    fn from(r: ServerRunRecord) -> ServerRunInfo {
        ServerRunInfo {
            id: r.id,
            started_at: r.started_at,
            version: r.version,
            git_commit: r.git_commit,
            settings: r.settings,
            instructions_sha256: r.instructions_sha256,
        }
    }
}

/// Each of `ids`, described by `info`, with how much it was used: its
/// messages, its newest one, and those since `since` counted in stretches of
/// `WEB_ACTIVITY_BUCKET`.
fn channel_activity(
    store: &dyn Reads,
    swarm: &SwarmId,
    ids: &[String],
    since: &str,
    info: impl Fn(&str) -> Result<ChannelInfo>,
) -> Result<Vec<ChannelActivity>> {
    let mut used: HashMap<String, ChannelUseRecord> = store
        .channel_use(swarm)?
        .into_iter()
        .map(|u| (u.channel_id.clone(), u))
        .collect();
    let mut recent: HashMap<String, BTreeMap<String, i64>> = HashMap::new();
    for m in store.message_times_since(swarm, since)? {
        let start = clock::floor_text(&m.created_at, WEB_ACTIVITY_BUCKET).unwrap_or(m.created_at);
        *recent
            .entry(m.channel_id)
            .or_default()
            .entry(start)
            .or_default() += 1;
    }
    ids.iter()
        .map(|id| {
            let info = info(id)?;
            let use_ = used.remove(id);
            let recent_activity = recent
                .remove(id)
                .unwrap_or_default()
                .into_iter()
                .map(|(start, count)| ActivityCount { start, count })
                .collect();
            Ok(ChannelActivity {
                message_count: use_.as_ref().map_or(0, |u| u.message_count),
                last_message_at: use_.as_ref().and_then(|u| u.last_message_at.clone()),
                last_speaker: use_.and_then(|u| u.last_speaker),
                recent_activity,
                info,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct ChannelView {
    pub channel: ChannelInfo,
    /// The channel is hidden: agents cannot see it, and nobody can change it.
    pub hidden: bool,
    pub members: Vec<AgentInfo>,
    /// Each member's latest message in the channel, for the members that
    /// have sent one, in the order of `members`.
    pub latest_messages: Vec<Message>,
    pub messages: Vec<Message>,
    /// Set when older messages exist.
    pub older_before_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct AgentView {
    pub agent: AgentInfo,
    pub channels: Vec<ChannelInfo>,
    pub message_count: i64,
    /// Every entry in its memory, whole, in name order.
    pub memory: Vec<super::MemoryEntry>,
}
