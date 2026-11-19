//! Output for an avatar's memory: saving an entry, reading one, and listing
//! them. The admin endpoint reads memory with the same formats.

use super::fit::{Row, short_time};
use super::{Format, Formatter};
use kkt_api_agent::{MemoryEntry, MemoryList, MemorySaved, MemorySummary};
use serde_json::{Value, json};

/// How many of an avatar's memory entries `reclaim` names; `list_memories`
/// lists the rest. At least 1.
pub(crate) const RECLAIM_MEMORY_NAMES: usize = 50;

impl Formatter {
    /// What `remember` saved, and how full the memory now is.
    pub fn memory_saved(&self, f: Format, s: &MemorySaved) -> String {
        let out = match f {
            Format::Text => format!(
                "{} `{}` ({} characters)\nmemory: {} entries, {} of {} characters",
                if s.replaced { "replaced" } else { "saved" },
                s.entry.name,
                s.entry.chars,
                s.entries,
                s.chars,
                s.max_chars
            ),
            Format::Json => json!({
                "name": s.entry.name,
                "chars": s.entry.chars,
                "saved_at": s.entry.saved_at,
                "replaced": s.replaced,
                "entries": s.entries,
                "memory_chars": s.chars,
                "max_memory_chars": s.max_chars,
            })
            .to_string(),
        };
        self.clip(out)
    }

    /// A lossless part of one entry, from character `offset` on, as `note`
    /// gives a channel's note.
    pub fn memory_entry(&self, f: Format, e: &MemoryEntry, offset: usize) -> String {
        self.text_in_parts(&e.text, offset, |text, offset, next| match f {
            Format::Json => json!({"handle": e.handle, "name": e.name, "text": text,
                "saved_at": e.saved_at, "offset": offset, "next_offset": next})
            .to_string(),
            Format::Text => {
                let mut out = format!(
                    "memory entry `{}` of {}\nsaved at {}\noffset: {offset}\n{text}",
                    e.name, e.handle, e.saved_at
                );
                if let Some(next) = next {
                    out.push_str(&format!(
                        "\nnext_offset: {next} (recall name={} offset={next})",
                        e.name
                    ));
                }
                out
            }
        })
    }

    /// One page of an avatar's entries, without their text, as many as fit.
    pub fn memory_list(&self, f: Format, list: &MemoryList) -> String {
        let now = self.clock.now();
        let rows: Vec<Row> = list
            .page
            .items
            .iter()
            .map(|e| Row {
                text: format!(
                    "{}  {} characters, saved {}",
                    e.name,
                    e.chars,
                    short_time(&e.saved_at, now)
                ),
                json: summary_json(e),
                resume: format!("cursor={}", e.name),
            })
            .collect();
        let header = (
            format!(
                "memory of {}: {} entries, {} of {} characters",
                list.handle, list.entries, list.chars, list.max_chars
            ),
            json!({
                "handle": list.handle,
                "entries": list.entries,
                "memory_chars": list.chars,
                "max_memory_chars": list.max_chars,
            }),
        );
        let tail = list
            .page
            .next_cursor
            .as_ref()
            .map(|c| format!("cursor={c}"));
        self.list(f, Some(header), "memory", &rows, tail)
    }
}

pub(super) fn summary_json(e: &MemorySummary) -> Value {
    json!({ "name": e.name, "chars": e.chars, "saved_at": e.saved_at })
}

/// `base`, followed by the names and sizes of the avatar's memory entries,
/// the first [`RECLAIM_MEMORY_NAMES`] of them, so a new session knows what
/// it can `recall`. Nothing is added when the memory is empty.
pub(super) fn with_memory(base: String, memory: &[MemorySummary]) -> String {
    if memory.is_empty() {
        return base;
    }
    let mut lines = vec![format!(
        "{base}\n\nYour memory ({} entries; `recall` reads one):",
        memory.len()
    )];
    lines.extend(memory.iter().take(RECLAIM_MEMORY_NAMES).map(|e| {
        format!(
            "- {} ({} characters, saved {})",
            e.name, e.chars, e.saved_at
        )
    }));
    if let Some(last) = memory.get(RECLAIM_MEMORY_NAMES - 1)
        && memory.len() > RECLAIM_MEMORY_NAMES
    {
        lines.push(format!(
            "and {} more: `list_memories` with cursor={}",
            memory.len() - RECLAIM_MEMORY_NAMES,
            last.name
        ));
    }
    lines.join("\n")
}

/// The avatar's memory entries for a JSON reply, the first
/// [`RECLAIM_MEMORY_NAMES`] of them, the count, and the cursor for the rest.
pub(super) fn memory_json(memory: &[MemorySummary]) -> (Value, usize, Value) {
    let shown: Vec<Value> = memory
        .iter()
        .take(RECLAIM_MEMORY_NAMES)
        .map(summary_json)
        .collect();
    let more = match memory.get(RECLAIM_MEMORY_NAMES - 1) {
        Some(last) if memory.len() > RECLAIM_MEMORY_NAMES => json!(last.name),
        _ => Value::Null,
    };
    (Value::Array(shown), memory.len(), more)
}
