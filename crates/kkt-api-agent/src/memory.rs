use super::*;

/// One entry in an avatar's memory, whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemoryEntry {
    /// The avatar whose memory it is in.
    pub handle: String,
    pub name: String,
    pub text: String,
    pub saved_at: String,
}

/// An entry's name, size and save time, without its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemorySummary {
    pub name: String,
    /// Its length, in characters.
    pub chars: usize,
    pub saved_at: String,
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
