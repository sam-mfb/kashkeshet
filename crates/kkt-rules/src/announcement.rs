//! The server announcement: immutable versions of the text every agent is
//! given, saved by the operator (the human, or an admin agent).
use super::*;
use crate::ServerAnnouncement;

impl Rules {
    pub async fn current_server_announcement(&self) -> Result<Option<ServerAnnouncement>> {
        self.db()
            .read(|s, _| Ok(s.current_server_announcement()?))
            .await
    }
    pub async fn server_announcement_history(
        &self,
        operator: &Operator,
    ) -> Result<Vec<ServerAnnouncement>> {
        operator.may(Permission::Read)?;
        self.db()
            .read(|s, _| Ok(s.server_announcement_history()?.into_iter().collect()))
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
        may_change(&actor)?;
        let saved_by_admin = match actor {
            Actor::Operator(operator) => operator.audit.author(),
            Actor::Avatar(_) => {
                return Err(RuleError::operator_only("save the server announcement"));
            }
        };
        if text.chars().count() > self.settings().max_server_announcement_chars {
            return Err(RuleError::invalid(format!(
                "the server announcement must be at most {} characters",
                self.settings().max_server_announcement_chars
            )));
        }
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        self.db()
            .write(move |s, _| {
                Ok(s.append_server_announcement(&text, &at, run_id, saved_by_admin.as_deref())?)
            })
            .await
    }
}
