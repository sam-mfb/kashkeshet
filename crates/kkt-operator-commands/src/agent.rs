//! The operator's list and revoke commands, on the local server database.
use crate::lanes::{DatabaseError, existing_db};
use crate::{OperatorParts, PartError};
use kkt_api_agent::RuleError;
use kkt_rules::{DirectAccess, Revoked};
use std::path::PathBuf;

/// Why `kkt agent` failed: the serve config could not be read, the
/// database could not be used, or the rules refused.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Config(PartError),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Rule(#[from] RuleError),
}

/// What `kkt agent` does.
pub enum AgentAction {
    List,
    Revoke { handle: String },
}

/// `kkt agent`: the operator's commands, on the database `kkt serve` would
/// use unless `--db` names another. Never creates a database.
pub async fn run(
    parts: &OperatorParts<'_>,
    command: AgentAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), AgentError> {
    // An explicit --db needs nothing from the config file, so it is not read.
    let config_db = match db {
        Some(_) => None,
        None => parts.serve.config(config).map_err(AgentError::Config)?.db,
    };
    let path = existing_db(&parts.lane_parts(), db, config_db)?;
    let db = parts.database.open(&path).map_err(DatabaseError::Open)?;
    // Key mode: the one swarm, which every avatar is in.
    let access = DirectAccess::new(db, kkt_api_agent::SwarmId::main(), parts.clock.clone());
    // Whoever can open the database file may do anything to it already.
    let operator = kkt_api_agent::Operator::new(
        kkt_api_agent::Permissions::of(&[
            kkt_api_agent::Permission::Read,
            kkt_api_agent::Permission::Change,
            kkt_api_agent::Permission::IssueRecoveryCodes,
        ]),
        kkt_api_agent::Audit::CommandLine,
    );
    match command {
        AgentAction::List => {
            let agents = access.agents(&operator).await?;
            if agents.is_empty() {
                parts
                    .output
                    .output(format_args!("no agents in {}\n", path.display()));
            }
            for a in agents {
                let seen = a.last_seen.as_deref().unwrap_or("never");
                let revoked = match &a.revoked_at {
                    Some(at) => format!("  revoked {at}"),
                    None => String::new(),
                };
                parts.output.output(format_args!(
                    "{}  created {}  last seen {seen}{revoked}\n",
                    a.handle, a.created_at
                ));
            }
        }
        AgentAction::Revoke { handle } => match access.revoke(&operator, handle.clone()).await? {
            Revoked::Now { at } => parts
                .output
                .output(format_args!("revoked {handle} at {at}\n")),
            Revoked::Already { at } => parts
                .output
                .output(format_args!("{handle} was already revoked at {at}\n")),
        },
    }
    Ok(())
}
