//! Memory: named entries an avatar keeps for whoever next holds it, such as a
//! session that takes it over with `reclaim`. Only the avatar itself reads
//! and changes its memory through MCP; the operator, the human on the web
//! page or an admin agent, may read it, and nobody else.
//!
//! An entry is at most `max_memory_entry_chars` long, and one avatar's
//! entries together at most `max_memory_chars`. Saving an entry replaces any
//! with its name, so two sessions saving different entries keep both.

use super::*;
use kkt_names::MAX_MEMORY_NAME_CHARS;

impl Rules {
    /// Saves `text` as `me`'s entry named `name`, replacing any with that
    /// name. Refused, changing nothing, if the name breaks the naming rules,
    /// the text is empty or longer than `max_memory_entry_chars`
    /// (`invalid_argument`), or the memory would then hold more than
    /// `max_memory_chars` (`limit_reached`).
    pub async fn remember(&self, me: &Agent, name: String, text: String) -> Result<MemorySaved> {
        names::check_memory_name(&name, MAX_MEMORY_NAME_CHARS).map_err(RuleError::invalid)?;
        if text.is_empty() {
            return Err(RuleError::invalid(
                "the entry's text is empty; `forget` deletes an entry",
            ));
        }
        let chars = text.chars().count();
        let max_entry = self.settings().max_memory_entry_chars;
        if chars > max_entry {
            return Err(RuleError::invalid(format!(
                "the entry is {chars} characters; an entry may have at most {max_entry} \
                 (max_memory_entry_chars). Nothing was saved."
            )));
        }
        let max_chars = self.settings().max_memory_chars;
        let handle = me.handle.clone();
        let entry = MemoryEntryRecord {
            name,
            text,
            saved_at: self.now_text(),
            run_id: Some(self.inner.deps.run_id),
        };
        self.db()
            .write(move |store, swarm| {
                let entries = store.memory_entries(swarm, &handle)?;
                let replaced = entries.iter().any(|e| e.name == entry.name);
                let others: usize = entries
                    .iter()
                    .filter(|e| e.name != entry.name)
                    .map(|e| e.text.chars().count())
                    .sum();
                if others + chars > max_chars {
                    return Err(RuleError::new(
                        Code::LimitReached,
                        format!(
                            "your memory would hold {} characters; an avatar's memory may hold \
                             at most {max_chars} (max_memory_chars). Nothing was saved: \
                             `forget` entries you no longer need, or shorten this one.",
                            others + chars
                        ),
                    ));
                }
                store.save_memory_entry(swarm, &handle, &entry)?;
                Ok(MemorySaved {
                    entry: MemorySummary::from(&entry),
                    replaced,
                    entries: entries.len() + usize::from(!replaced),
                    chars: others + chars,
                    max_chars,
                })
            })
            .await
    }

    /// `me`'s entry named `name`; `not_found` if there is none.
    pub async fn recall(&self, me: &Agent, name: String) -> Result<MemoryEntry> {
        names::check_memory_name(&name, MAX_MEMORY_NAME_CHARS).map_err(RuleError::invalid)?;
        let handle = me.handle.clone();
        self.db()
            .read(move |store, swarm| memory_entry(store, swarm, &handle, &name))
            .await
    }

    /// The entries in `me`'s memory, in name order, after `cursor` if given,
    /// at most `limit` of them.
    pub async fn list_memories(
        &self,
        me: &Agent,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<MemoryList> {
        let handle = me.handle.clone();
        let max_chars = self.settings().max_memory_chars;
        self.db()
            .read(move |store, swarm| {
                memory_list(store, swarm, &handle, limit, cursor.as_deref(), max_chars)
            })
            .await
    }

    /// Deletes `me`'s entry named `name`; `not_found` if there is none.
    pub async fn forget(&self, me: &Agent, name: String) -> Result<()> {
        names::check_memory_name(&name, MAX_MEMORY_NAME_CHARS).map_err(RuleError::invalid)?;
        let handle = me.handle.clone();
        self.db()
            .write(move |store, swarm| {
                if store.delete_memory_entry(swarm, &handle, &name)? {
                    Ok(())
                } else {
                    Err(memory_not_found(&name))
                }
            })
            .await
    }

    /// The entries in the memory of the avatar `handle`, for an admin agent,
    /// which reads it for the operator. `not_found` for no such avatar.
    pub async fn admin_list_memories(
        &self,
        handle: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<MemoryList> {
        let max_chars = self.settings().max_memory_chars;
        self.db()
            .read(move |store, swarm| {
                if store.agent(swarm, &handle)?.is_none() {
                    return Err(RuleError::agent_not_found(&handle));
                }
                memory_list(store, swarm, &handle, limit, cursor.as_deref(), max_chars)
            })
            .await
    }

    /// The entry named `name` in the memory of the avatar `handle`, for an
    /// admin agent. `not_found` for no such avatar or entry.
    pub async fn admin_recall(&self, handle: String, name: String) -> Result<MemoryEntry> {
        names::check_memory_name(&name, MAX_MEMORY_NAME_CHARS).map_err(RuleError::invalid)?;
        self.db()
            .read(move |store, swarm| {
                if store.agent(swarm, &handle)?.is_none() {
                    return Err(RuleError::agent_not_found(&handle));
                }
                memory_entry(store, swarm, &handle, &name)
            })
            .await
    }
}

/// Every entry in `handle`'s memory, whole, in name order: what the
/// operator's view of an avatar shows.
pub(super) fn memory(
    store: &Reader<'_>,
    swarm: &SwarmId,
    handle: &str,
) -> Result<Vec<MemoryEntry>> {
    Ok(store
        .memory_entries(swarm, handle)?
        .into_iter()
        .map(|e| MemoryEntry::new(handle, e))
        .collect())
}

/// The names and sizes of the entries in `handle`'s memory, in name order.
pub(super) fn memory_summaries(
    store: &Reader<'_>,
    swarm: &SwarmId,
    handle: &str,
) -> Result<Vec<MemorySummary>> {
    Ok(store
        .memory_entries(swarm, handle)?
        .iter()
        .map(MemorySummary::from)
        .collect())
}

fn memory_entry(
    store: &Reader<'_>,
    swarm: &SwarmId,
    handle: &str,
    name: &str,
) -> Result<MemoryEntry> {
    store
        .memory_entry(swarm, handle, name)?
        .map(|e| MemoryEntry::new(handle, e))
        .ok_or_else(|| memory_not_found(name))
}

fn memory_list(
    store: &Reader<'_>,
    swarm: &SwarmId,
    handle: &str,
    limit: usize,
    cursor: Option<&str>,
    max_chars: usize,
) -> Result<MemoryList> {
    let all = memory_summaries(store, swarm, handle)?;
    let chars = all.iter().map(|e| e.chars).sum();
    let count = all.len();
    let mut items: Vec<MemorySummary> = all
        .into_iter()
        .filter(|e| cursor.is_none_or(|c| e.name.as_str() > c))
        .take(limit + 1)
        .collect();
    let next_cursor = if items.len() > limit {
        items.truncate(limit);
        items.last().map(|e| e.name.clone())
    } else {
        None
    };
    Ok(MemoryList {
        handle: handle.to_string(),
        page: Page { items, next_cursor },
        entries: count,
        chars,
        max_chars,
    })
}

fn memory_not_found(name: &str) -> RuleError {
    RuleError::new(
        Code::NotFound,
        format!("no memory entry named {name}; `list_memories` lists them"),
    )
}

/// One entry in an avatar's memory, whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemoryEntry {
    /// The avatar whose memory it is in.
    pub handle: String,
    pub name: String,
    pub text: String,
    pub saved_at: String,
}

impl MemoryEntry {
    fn new(handle: &str, e: MemoryEntryRecord) -> MemoryEntry {
        MemoryEntry {
            handle: handle.to_string(),
            name: e.name,
            text: e.text,
            saved_at: e.saved_at,
        }
    }
}

/// An entry's name, size and save time, without its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemorySummary {
    pub name: String,
    /// Its length, in characters.
    pub chars: usize,
    pub saved_at: String,
}

impl From<&MemoryEntryRecord> for MemorySummary {
    fn from(e: &MemoryEntryRecord) -> MemorySummary {
        MemorySummary {
            name: e.name.clone(),
            chars: e.text.chars().count(),
            saved_at: e.saved_at.clone(),
        }
    }
}

/// A page of an avatar's memory entries, and the memory as a whole.
#[derive(Clone, Debug)]
pub struct MemoryList {
    pub handle: String,
    pub page: Page<MemorySummary>,
    /// How many entries the memory holds in all.
    pub entries: usize,
    /// How many characters they hold together.
    pub chars: usize,
    /// The most they may hold together: `max_memory_chars`.
    pub max_chars: usize,
}

/// What `remember` saved, and the memory after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemorySaved {
    pub entry: MemorySummary,
    /// Whether it replaced an entry with its name.
    pub replaced: bool,
    /// How many entries the memory now holds.
    pub entries: usize,
    /// How many characters they now hold together.
    pub chars: usize,
    /// The most they may hold together: `max_memory_chars`.
    pub max_chars: usize,
}
