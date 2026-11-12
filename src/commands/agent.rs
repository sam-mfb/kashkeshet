//! `kkt agent`, the operator's commands on agents in the local database.
//! (`kkt register` and `kkt reclaim` are kkt-agent-commands'.)

use crate::commands::lanes::existing_db;
use crate::machine::settings::read_config;
use kkt_clock_system::SystemClock;
use kkt_machine::{Env, Platform};
use kkt_rules::{CommandLine, Revoked};
use kkt_terminal_os::Terminal;
use std::path::PathBuf;
use std::sync::Arc;

/// What `kkt agent` does.
pub enum AgentAction {
    List,
    Revoke { handle: String },
}

/// `kkt agent`: the operator's commands, on the database `kkt serve` would
/// use unless `--db` names another. Never creates a database.
pub async fn run(
    env: &Env,
    platform: Platform,
    command: AgentAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    // An explicit --db needs nothing from the config file, so it is not read.
    let config_db = match db {
        Some(_) => None,
        None => read_config(env, config)?.1.db,
    };
    let path = existing_db(env, platform, db, config_db)?;
    let db = crate::machine::database::open(&path)?;
    // Key mode: the one swarm, which every avatar is in.
    let operator = CommandLine::new(db, kkt_api_agent::SwarmId::main(), Arc::new(SystemClock));
    match command {
        AgentAction::List => {
            let agents = operator.agents().await.map_err(|e| e.to_string())?;
            if agents.is_empty() {
                Terminal.output(format_args!("no agents in {}\n", path.display()));
            }
            for a in agents {
                let seen = a.last_seen.as_deref().unwrap_or("never");
                let revoked = match &a.revoked_at {
                    Some(at) => format!("  revoked {at}"),
                    None => String::new(),
                };
                Terminal.output(format_args!(
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
                Revoked::Now { at } => Terminal.output(format_args!("revoked {handle} at {at}\n")),
                Revoked::Already { at } => {
                    Terminal.output(format_args!("{handle} was already revoked at {at}\n"))
                }
            }
        }
    }
    Ok(())
}
