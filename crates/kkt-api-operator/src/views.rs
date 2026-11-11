use super::*;

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
    /// The messages of the window the caller asked for, counted in
    /// stretches of the bucket it asked for, oldest first.
    pub recent_activity: Vec<ActivityCount>,
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
