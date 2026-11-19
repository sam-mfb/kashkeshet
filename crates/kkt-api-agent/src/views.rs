use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct AgentView {
    pub agent: AgentInfo,
    pub channels: Vec<ChannelInfo>,
    pub message_count: i64,
    /// Every entry in its memory, whole, in name order.
    pub memory: Vec<super::MemoryEntry>,
}
