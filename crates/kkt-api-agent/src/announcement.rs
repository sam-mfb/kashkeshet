use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServerAnnouncement {
    pub version: i64,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent that saved this version, acting as the operator,
    /// such as `admin-sentinel`; `None` when the human operator saved it.
    pub saved_by_admin: Option<String>,
}
