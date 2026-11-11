/// One agent as the operator sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentStatus {
    pub handle: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    /// When its token was revoked, if it has been.
    pub revoked_at: Option<String>,
}
