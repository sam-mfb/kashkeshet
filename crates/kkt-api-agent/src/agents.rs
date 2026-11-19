use super::*;

/// Who makes a change: an avatar, or the operator, through the web page or
/// the admin endpoint. Both go through the same rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Actor {
    Avatar(Agent),
    /// The operator, who may change any channel as a member could and
    /// administer avatars, lanes and the server announcement, as far as its
    /// [`Permission`]s allow. Its changes are announced as the operator's.
    Operator(Operator),
}

/// An agent that has proven which avatar it holds on this call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub handle: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Registration {
    pub server_announcement: Option<ServerAnnouncement>,
    pub handle: String,
    pub token: String,
    pub recovery_code: String,
    /// The lane the new avatar took, if it asked for one.
    pub lane: Option<HeldLane>,
}

/// A new authentication token for an existing avatar.
#[derive(Clone, Debug, Serialize)]
pub struct Reclaimed {
    pub handle: String,
    pub token: String,
    /// The current server announcement, as at registration.
    pub server_announcement: Option<ServerAnnouncement>,
    /// The lane the avatar holds, if any.
    pub lane: Option<HeldLane>,
    /// The entries in the avatar's memory, in name order, without their
    /// text: `recall` reads each.
    pub memory: Vec<super::MemorySummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentInfo {
    pub handle: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    /// When the agent last sent a message the viewer may see: in the
    /// channel listed, in any channel visible to the agent viewing, or, for
    /// the human and admin agents, anywhere. Worked out for each viewer, so
    /// it never shows activity in a closed channel the viewer is not in.
    pub last_sent: Option<String>,
    /// The server run that created the agent, if it was recorded.
    pub run_id: Option<i64>,
    /// What the agent says about itself, if anything.
    pub description: Option<String>,
    pub client: Option<RecordedClient>,
    pub listening: bool,
    pub waiting_channels: Vec<String>,
    /// The lane the agent holds, if any. It grants no permissions.
    pub lane: Option<AgentLane>,
}

/// Client-reported MCP initialization metadata, without library types.
#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// The MCP client an agent reported at its last registration or reclaim,
/// and when. Reported by the client, not verified by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecordedClient {
    pub name: String,
    pub version: String,
    pub recorded_at: String,
    /// The server run that recorded it.
    pub run_id: i64,
}

impl From<&Agent> for Actor {
    fn from(agent: &Agent) -> Actor {
        Actor::Avatar(agent.clone())
    }
}

impl From<&Operator> for Actor {
    fn from(operator: &Operator) -> Actor {
        Actor::Operator(operator.clone())
    }
}
