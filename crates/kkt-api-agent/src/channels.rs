use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct ChannelNote {
    pub channel_id: String,
    pub text: String,
    pub edited_by: Option<String>,
    pub edited_at: Option<String>,
}

/// The `list_channels` cursor that continues after `c`. It holds the
/// channel's name (as hex, since names can hold any character) and ID, so it
/// carries the position itself, and using it never reveals whether some
/// other channel exists. kkt-rules reads it back.
pub fn channel_cursor(c: &ChannelInfo) -> String {
    let hex: String = c.name.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.{}", c.id)
}

#[derive(Clone, Debug, Serialize)]
pub struct ChannelInfo {
    pub note: ChannelNote,
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub open: bool,
    pub created_by: String,
    pub created_at: String,
    /// The agent that may remove members and pass the ownership on, if any.
    pub owner: Option<String>,
    pub member_count: i64,
    /// The first few members, in the order they joined.
    pub members_preview: Vec<String>,
    pub is_member: bool,
    /// Messages newer than the `after_id` the agent gave, if it gave one.
    pub new_messages: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct ChannelQuery {
    pub query: Option<String>,
    pub member_only: bool,
    pub with_agent: Option<String>,
    pub after_id: Option<i64>,
    pub limit: usize,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewChannel {
    pub name: String,
    pub open: bool,
    pub purpose: Option<String>,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Members {
    pub channel_id: String,
    pub total: i64,
    /// The channel's owner, if it has one.
    pub owner: Option<String>,
    pub page: Page<AgentInfo>,
}
