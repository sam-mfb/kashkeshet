//! What an admin agent reads at `/mcp-admin`, acting for the operator as
//! [`Who::Admin`]. It sees every channel, closed and hidden ones included,
//! and every agent the web page shows. Admission (the admin key) has already
//! let the caller in; nothing here changes anything.

use super::channels::{decode_channel_cursor, encode_channel_cursor};
use super::*;
use crate::store::AnyChannelRecord;

/// A channel as the admin agent sees it.
#[derive(Clone, Debug, Serialize)]
pub struct AdminChannel {
    pub info: ChannelInfo,
    /// Hidden by its owner or the operator: no agent can find it, but its
    /// messages are kept.
    pub hidden: bool,
}

impl Rules {
    /// Every channel, hidden ones included, in name order, whose name or
    /// purpose contains `query` if given.
    pub async fn admin_list_channels(
        &self,
        query: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<AdminChannel>> {
        let preview = self.limits().member_preview;
        let after = cursor.as_deref().map(decode_channel_cursor).transpose()?;
        self.db()
            .read(move |store| {
                let mut ids = store.any_channel_ids(query.as_deref(), after.as_ref(), limit + 1)?;
                let more = ids.len() > limit;
                ids.truncate(limit);
                let items = ids
                    .iter()
                    .map(|id| {
                        let record = admin_channel(store, id, Need::See)?;
                        admin_view(store, record, preview)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = match (more, items.last()) {
                    (true, Some(last)) => {
                        Some(encode_channel_cursor(&last.info.name, &last.info.id))
                    }
                    _ => None,
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// One channel, hidden or not.
    pub async fn admin_channel(&self, channel_id: String) -> Result<AdminChannel> {
        let preview = self.limits().member_preview;
        self.db()
            .read(move |store| {
                let record = admin_channel(store, &channel_id, Need::See)?;
                admin_view(store, record, preview)
            })
            .await
    }

    /// A channel's members in name order, with what the web page shows of
    /// each. A hidden channel has none: hiding removes them.
    pub async fn admin_list_members(
        &self,
        channel_id: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Members> {
        let rules = self.clone();
        self.db()
            .read(move |store| {
                let record = admin_channel(store, &channel_id, Need::See)?;
                let total = store.member_count(&channel_id)?;
                let mut items = store
                    .members_by_name(&channel_id, cursor.as_deref(), limit + 1)?
                    .into_iter()
                    .map(|a| rules.observed_agent(store, a, None, Some(&channel_id)))
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = if items.len() > limit {
                    items.truncate(limit);
                    items.last().map(|a| a.name.clone())
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
    pub async fn admin_note(&self, channel_id: String) -> Result<ChannelNote> {
        self.db()
            .read(move |store| {
                let note = admin_channel(store, &channel_id, Need::See)?.summary.note;
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
    pub async fn admin_read(&self, q: ReadQuery) -> Result<MessagePage> {
        self.db()
            .read(move |store| {
                admin_channel(store, &q.channel_id, Need::See)?;
                super::messages::read_channel(store, &q)
            })
            .await
    }

    /// One message in full, in any channel.
    pub async fn admin_message(&self, id: i64) -> Result<Message> {
        self.db()
            .read(move |store| {
                let message = store
                    .message(id)?
                    .ok_or_else(|| RuleError::message_not_found(id))?;
                admin_channel(store, &message.channel_id, Need::See)
                    .map_err(|_| RuleError::message_not_found(id))?;
                Ok(Message::from(message))
            })
            .await
    }

    /// Every channel, hidden ones included, with messages newer than
    /// `after_id`: the one with the newest message first.
    pub async fn admin_activity(&self, after_id: i64) -> Result<Vec<ActivityRow>> {
        self.db()
            .read(move |store| {
                let rows = store.any_activity(after_id)?;
                Ok(rows.into_iter().map(ActivityRow::from).collect())
            })
            .await
    }

    /// Messages newer than `after_id` in every channel, hidden ones
    /// included, or only in `channel_ids`; oldest first. It returns at once:
    /// the admin endpoint has no waiting.
    pub async fn admin_new_messages(
        &self,
        after_id: i64,
        channel_ids: Option<Vec<String>>,
        limit: usize,
    ) -> Result<WaitResult> {
        if channel_ids.as_ref().is_some_and(Vec::is_empty) {
            return Err(RuleError::invalid(
                "`channel_ids` must list at least one channel, or be left out",
            ));
        }
        self.db()
            .read(move |store| {
                if let Some(ids) = &channel_ids {
                    for id in ids {
                        admin_channel(store, id, Need::See)?;
                    }
                }
                let messages = store.any_newer_messages(after_id, channel_ids.as_deref(), limit)?;
                let total = store.count_any_newer_messages(after_id, channel_ids.as_deref())?;
                Ok(WaitResult {
                    more_waiting: total - messages.len() as i64,
                    messages: messages.into_iter().map(Message::from).collect(),
                })
            })
            .await
    }

    /// Agents in name order, as the web page shows them: whether they are
    /// listening covers every channel.
    pub async fn admin_list_agents(
        &self,
        query: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<AgentInfo>> {
        let rules = self.clone();
        self.db()
            .read(move |store| {
                let mut items = store
                    .agents(query.as_deref(), cursor.as_deref(), limit + 1)?
                    .into_iter()
                    .map(|a| rules.observed_agent(store, a, None, None))
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = if items.len() > limit {
                    items.truncate(limit);
                    items.last().map(|a| a.name.clone())
                } else {
                    None
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// One agent as the web page's agent view shows it: what
    /// `admin_list_agents` gives, its channels in name order, how many
    /// messages it has sent, and its handoff note. Never a recovery code:
    /// issuing one stays with the human.
    pub async fn admin_agent(&self, name: String) -> Result<AgentView> {
        self.agent_view(name).await
    }

    /// One agent's handoff note, which only the agent and the operator may
    /// read.
    pub async fn admin_handoff_note(&self, name: String) -> Result<HandoffNote> {
        self.db()
            .read(move |store| super::agents::handoff_note(store, &name))
            .await
    }
}

/// The channel `id`, hidden or not, if `access::decide` lets the admin agent
/// use it for `need`.
fn admin_channel(store: &dyn Reads, id: &str, need: Need) -> Result<AnyChannelRecord> {
    let record = store.any_channel(id)?;
    let channel = match &record {
        None => access::Channel::Missing,
        Some(r) if r.hidden => access::Channel::Hidden,
        Some(r) => access::Channel::Live {
            open: r.summary.open,
        },
    };
    match (access::decide(channel, Who::Admin, need), record) {
        (Decision::Allowed, Some(record)) => Ok(record),
        _ => Err(RuleError::channel_not_found(id)),
    }
}

fn admin_view(store: &dyn Reads, record: AnyChannelRecord, preview: usize) -> Result<AdminChannel> {
    let id = record.summary.id.clone();
    Ok(AdminChannel {
        hidden: record.hidden,
        info: info_from_summary(store, &id, record.summary, preview)?,
    })
}
