//! The operator's list and revoke commands, on the local server database.
use crate::lanes::existing_db;
use kkt_rules::{CommandLine, Revoked};
use kkt_setup::connection::ConnectionSource;
use std::path::PathBuf;

/// What `kkt agent` does.
pub enum AgentAction {
    List,
    Revoke { handle: String },
}

/// `kkt agent`: the operator's commands, on the database `kkt serve` would
/// use unless `--db` names another. Never creates a database.
pub async fn run(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    command: AgentAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    // An explicit --db needs nothing from the config file, so it is not read.
    let config_db = match db {
        Some(_) => None,
        None => parts.serve.config(config)?.db,
    };
    let path = existing_db(parts, db, config_db)?;
    let db = parts.database.open(&path)?;
    // Key mode: the one swarm, which every avatar is in.
    let operator = CommandLine::new(db, kkt_api_agent::SwarmId::main(), parts.clock.clone());
    match command {
        AgentAction::List => {
            let agents = operator.agents().await.map_err(|e| e.to_string())?;
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
        AgentAction::Revoke { handle } => {
            match operator
                .revoke(handle.clone())
                .await
                .map_err(|e| e.to_string())?
            {
                Revoked::Now { at } => parts
                    .output
                    .output(format_args!("revoked {handle} at {at}\n")),
                Revoked::Already { at } => parts
                    .output
                    .output(format_args!("{handle} was already revoked at {at}\n")),
            }
        }
    }
    Ok(())
}
