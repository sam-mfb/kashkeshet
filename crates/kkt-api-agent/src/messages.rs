use super::*;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
    pub created_at: String,
    /// The server run that stored the message, if it was recorded.
    pub run_id: Option<i64>,
}

/// Where to read from in a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// The newest messages.
    Latest,
    /// Messages with IDs above this one, oldest first.
    After(i64),
    /// Messages with IDs below this one, the nearest first.
    Before(i64),
}

#[derive(Clone, Debug)]
pub struct ReadQuery {
    pub channel_id: String,
    pub position: Position,
    pub from_agent: Option<String>,
    pub limit: usize,
}

/// Messages in ID order, plus where to continue if more exist in the
/// direction being read.
#[derive(Clone, Debug, Serialize)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    #[serde(skip)]
    pub more: Option<Position>,
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    /// Words that must all appear.
    pub query: Option<String>,
    /// Words that must appear together and in this order.
    pub phrase: Option<String>,
    pub channel_id: Option<String>,
    pub from_agent: Option<String>,
    pub after_id: Option<i64>,
    pub before_id: Option<i64>,
    /// An RFC 3339 time, as the agent gave it.
    pub since: Option<String>,
    /// An RFC 3339 time, as the agent gave it.
    pub until: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub created_at: String,
    pub excerpt: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActivityRow {
    pub channel_id: String,
    pub channel_name: String,
    pub new_messages: i64,
    pub latest_id: i64,
}

#[derive(Clone, Debug)]
pub struct WaitQuery {
    pub after_id: i64,
    pub channel_ids: Option<Vec<String>>,
    /// How long to wait, cut to `settings.max_wait`. `None` waits
    /// `settings.default_wait`.
    pub timeout: Option<std::time::Duration>,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct WaitResult {
    pub messages: Vec<Message>,
    /// Newer messages beyond the ones returned.
    pub more_waiting: i64,
    /// The lane the waiting agent holds, when it has a newer version than
    /// the one the agent took. Always `None` for the admin endpoint.
    pub newer_lane: Option<AgentLane>,
}
