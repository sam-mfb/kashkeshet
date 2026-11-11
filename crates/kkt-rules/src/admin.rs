//! What an admin agent reads at `/mcp-admin`, acting for the operator as
//! [`Who::Operator`]. It sees every channel, closed and hidden ones included,
//! and every agent the web page shows. Admission (the admin key) has already
//! let the caller in; nothing here changes anything.

use super::channels::decode_channel_cursor;
use super::*;
use crate::AnyChannelRecord;

impl Rules {
    /// Every channel, hidden ones included, in name order, whose name or
    /// purpose contains `query` if given.
    pub async fn admin_list_channels(
        &self,
        operator: &Operator,
        query: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<AdminChannel>> {
        operator.may(Permission::Read)?;
        let preview = self.settings().member_preview;
        let after = cursor.as_deref().map(decode_channel_cursor).transpose()?;
        self.db()
            .read(move |store, swarm| {
                let mut ids =
                    store.any_channel_ids(swarm, query.as_deref(), after.as_ref(), limit + 1)?;
                let more = ids.len() > limit;
                ids.truncate(limit);
                let items = ids
                    .iter()
                    .map(|id| {
                        let record = admin_channel(store, swarm, id, Need::See)?;
                        admin_view(store, swarm, record, preview)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = match (more, items.last()) {
                    (true, Some(last)) => Some(channel_cursor(&last.info)),
                    _ => None,
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// One channel, hidden or not.
    pub async fn admin_channel(
        &self,
        operator: &Operator,
        channel_id: String,
    ) -> Result<AdminChannel> {
        operator.may(Permission::Read)?;
        let preview = self.settings().member_preview;
        self.db()
            .read(move |store, swarm| {
                let record = admin_channel(store, swarm, &channel_id, Need::See)?;
                admin_view(store, swarm, record, preview)
            })
            .await
    }

    /// A channel's members in handle order, with what the web page shows of
    /// each. A hidden channel has none: hiding removes them.
    pub async fn admin_list_members(
        &self,
        operator: &Operator,
        channel_id: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Members> {
        operator.may(Permission::Read)?;
        let rules = self.clone();
        self.db()
            .read(move |store, swarm| {
                let record = admin_channel(store, swarm, &channel_id, Need::See)?;
                let total = store.member_count(swarm, &channel_id)?;
                let mut items = store
                    .members_by_name(swarm, &channel_id, cursor.as_deref(), limit + 1)?
                    .into_iter()
                    .map(|a| rules.observed_agent(store, swarm, a, None, Some(&channel_id)))
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = if items.len() > limit {
                    items.truncate(limit);
                    items.last().map(|a| a.handle.clone())
                } else {
                    None
                };
                Ok(Members {
                    channel_id,
                    total,
                    owner: record.summary.owner,
                    page: Page { items, next_cursor },
                })
            })
            .await
    }

    /// A channel's pinned note, hidden or not.
    pub async fn admin_note(&self, operator: &Operator, channel_id: String) -> Result<ChannelNote> {
        operator.may(Permission::Read)?;
        self.db()
            .read(move |store, swarm| {
                let note = admin_channel(store, swarm, &channel_id, Need::See)?
                    .summary
                    .note;
                Ok(ChannelNote {
                    channel_id,
                    text: note.text,
                    edited_by: note.edited_by,
                    edited_at: note.edited_at,
                })
            })
            .await
    }

    /// Messages in any channel, hidden or not.
    pub async fn admin_read(&self, operator: &Operator, q: ReadQuery) -> Result<MessagePage> {
        operator.may(Permission::Read)?;
        self.db()
            .read(move |store, swarm| {
                admin_channel(store, swarm, &q.channel_id, Need::See)?;
                super::messages::read_channel(store, swarm, &q)
            })
            .await
    }

    /// One message in full, in any channel.
    pub async fn admin_message(&self, operator: &Operator, id: i64) -> Result<Message> {
        operator.may(Permission::Read)?;
        self.db()
            .read(move |store, swarm| {
                let message = store
                    .message(swarm, id)?
                    .ok_or_else(|| RuleError::message_not_found(id))?;
                admin_channel(store, swarm, &message.channel_id, Need::See)
                    .map_err(|_| RuleError::message_not_found(id))?;
                Ok(message)
            })
            .await
    }

    /// Every channel, hidden ones included, with messages newer than
    /// `after_id`: the one with the newest message first.
    pub async fn admin_activity(
        &self,
        operator: &Operator,
        after_id: i64,
    ) -> Result<Vec<ActivityRow>> {
        operator.may(Permission::Read)?;
        self.db()
            .read(move |store, swarm| {
                let rows = store.any_activity(swarm, after_id)?;
                Ok(rows.into_iter().collect())
            })
            .await
    }

    /// Messages newer than `after_id` in every channel, hidden ones
    /// included, or only in `channel_ids`; oldest first. It returns at once:
    /// the admin endpoint has no waiting.
    pub async fn admin_new_messages(
        &self,
        operator: &Operator,
        after_id: i64,
        channel_ids: Option<Vec<String>>,
        limit: usize,
    ) -> Result<WaitResult> {
        operator.may(Permission::Read)?;
        if channel_ids.as_ref().is_some_and(Vec::is_empty) {
            return Err(RuleError::invalid(
                "`channel_ids` must list at least one channel, or be left out",
            ));
        }
        self.db()
            .read(move |store, swarm| {
                if let Some(ids) = &channel_ids {
                    for id in ids {
                        admin_channel(store, swarm, id, Need::See)?;
                    }
                }
                let messages =
                    store.any_newer_messages(swarm, after_id, channel_ids.as_deref(), limit)?;
                let total =
                    store.count_any_newer_messages(swarm, after_id, channel_ids.as_deref())?;
                Ok(WaitResult {
                    more_waiting: total - messages.len() as i64,
                    messages: messages.into_iter().collect(),
                    newer_lane: None,
                })
            })
            .await
    }

    /// Agents in handle order, as the web page shows them: whether they are
    /// listening covers every channel.
    pub async fn admin_list_agents(
        &self,
        operator: &Operator,
        query: Option<String>,
        lane: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<AgentInfo>> {
        operator.may(Permission::Read)?;
        let rules = self.clone();
        self.db()
            .read(move |store, swarm| {
                let mut items = store
                    .agents(
                        swarm,
                        query.as_deref(),
                        lane.as_deref(),
                        cursor.as_deref(),
                        limit + 1,
                    )?
                    .into_iter()
                    .map(|a| rules.observed_agent(store, swarm, a, None, None))
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = if items.len() > limit {
                    items.truncate(limit);
                    items.last().map(|a| a.handle.clone())
                } else {
                    None
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// One agent as the web page's agent view shows it: what
    /// `admin_list_agents` gives, its channels in name order, how many
    /// messages it has sent, and its memory. Never a recovery code:
    /// issuing one stays with the human.
    pub async fn admin_agent(&self, operator: &Operator, handle: String) -> Result<AgentView> {
        operator.may(Permission::Read)?;
        self.agent_view(operator, handle).await
    }
}

/// The channel `id`, hidden or not, if `access::decide` lets the admin agent
/// use it for `need`.
fn admin_channel(
    store: &Reader<'_>,
    swarm: &SwarmId,
    id: &str,
    need: Need,
) -> Result<AnyChannelRecord> {
    let record = store.any_channel(swarm, id)?;
    let channel = match &record {
        None => access::Channel::Missing,
        Some(r) if r.hidden => access::Channel::Hidden,
        Some(r) => access::Channel::Live {
            open: r.summary.open,
        },
    };
    match (access::decide(channel, Who::Operator, need), record) {
        (Decision::Allowed, Some(record)) => Ok(record),
        _ => Err(RuleError::channel_not_found(id)),
    }
}

fn admin_view(
    store: &Reader<'_>,
    swarm: &SwarmId,
    record: AnyChannelRecord,
    preview: usize,
) -> Result<AdminChannel> {
    let id = record.summary.id.clone();
    Ok(AdminChannel {
        hidden: record.hidden,
        info: info_from_summary(store, swarm, &id, record.summary, preview)?,
    })
}
