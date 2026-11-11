//! What the server's operator can do from the command line, against the
//! database file directly, while a server may be running on it: list agents
//! and revoke an agent's token. Importing and exporting lanes is in
//! `lane_import.rs`.
//!
//! A running server looks a token up in the database on every call and
//! keeps no copy, so a revocation takes effect on the agent's next call.

use super::*;

/// The operator's view of the agents of one swarm. Needs only the database,
/// the swarm and a clock, not a running server.
pub struct CommandLine {
    pub(super) db: Bound,
    pub(super) clock: Arc<dyn Clock>,
}

/// The result of a revocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Revoked {
    /// Revoked now, at this time.
    Now { at: String },
    /// Already revoked earlier, at this time; nothing changed.
    Already { at: String },
}

impl CommandLine {
    /// The operator of the avatars and channels of `swarm` in `db`. Lanes
    /// belong to the server, not to a swarm: importing them reaches every
    /// swarm's avatars alike.
    pub fn new(db: Db, swarm: SwarmId, clock: Arc<dyn Clock>) -> CommandLine {
        CommandLine {
            db: Bound::new(db, swarm),
            clock,
        }
    }

    /// Every agent of the swarm, most recently seen first, revoked ones
    /// included.
    pub async fn agents(&self) -> Result<Vec<AgentStatus>> {
        self.db
            .read(|store, swarm| {
                Ok(store
                    .agents_by_last_seen(swarm)?
                    .into_iter()
                    .map(|a| AgentStatus {
                        handle: a.name,
                        created_at: a.created_at,
                        last_seen: a.last_seen,
                        revoked_at: a.revoked_at,
                    })
                    .collect())
            })
            .await
    }

    /// Revokes the token of the agent with the handle `handle`, so every
    /// later call with it is refused as `unauthenticated`. The agent's
    /// handle, memberships and messages are kept. `not_found` if there is no
    /// such agent.
    pub async fn revoke(&self, handle: String) -> Result<Revoked> {
        let now = clock::to_text(self.clock.now());
        self.db
            .write(move |store, swarm| {
                let agent = store
                    .agent(swarm, &handle)?
                    .ok_or_else(|| RuleError::agent_not_found(&handle))?;
                match agent.revoked_at {
                    Some(at) => Ok(Revoked::Already { at }),
                    None => {
                        store.set_revoked(swarm, &handle, &now)?;
                        Ok(Revoked::Now { at: now })
                    }
                }
            })
            .await
    }
}
