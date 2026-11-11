//! The server announcement: immutable versions of the text every agent is
//! given, saved by the operator (the human, or an admin agent).
use super::*;
use crate::ServerAnnouncementRecord;

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
impl From<ServerAnnouncementRecord> for ServerAnnouncement {
    fn from(v: ServerAnnouncementRecord) -> Self {
        Self {
            version: v.version,
            text: v.text,
            saved_at: v.saved_at,
            run_id: v.run_id,
            saved_by_admin: v.saved_by_admin,
        }
    }
}
impl Rules {
    pub async fn current_server_announcement(&self) -> Result<Option<ServerAnnouncement>> {
        self.db()
            .read(|s, _| {
                Ok(s.current_server_announcement()?
                    .map(ServerAnnouncement::from))
            })
            .await
    }
    pub async fn server_announcement_history(
        &self,
        operator: &Operator,
    ) -> Result<Vec<ServerAnnouncement>> {
        operator.may(Permission::Read)?;
        self.db()
            .read(|s, _| {
                Ok(s.server_announcement_history()?
                    .into_iter()
                    .map(ServerAnnouncement::from)
                    .collect())
            })
            .await
    }
    /// Empty text is a new version which clears the current announcement.
    /// The human or an admin agent may save it; the version records which,
    /// in the same write.
    pub async fn save_server_announcement(
        &self,
        actor: Actor,
        text: String,
    ) -> Result<ServerAnnouncement> {
        actor.may_change()?;
        let saved_by_admin = match actor {
            Actor::Operator(operator) => operator.audit.author(),
            Actor::Avatar(_) => {
                return Err(RuleError::operator_only("save the server announcement"));
            }
        };
        if text.chars().count() > self.limits().max_server_announcement_chars {
            return Err(RuleError::invalid(format!(
                "the server announcement must be at most {} characters",
                self.limits().max_server_announcement_chars
            )));
        }
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        self.db()
            .write(move |s, _| {
                Ok(ServerAnnouncement::from(s.append_server_announcement(
                    &text,
                    &at,
                    run_id,
                    saved_by_admin.as_deref(),
                )?))
            })
            .await
    }
}
