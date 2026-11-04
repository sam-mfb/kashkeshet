//! Agents: `kkt register` and `kkt reclaim`, which get an identity from a
//! server and save its credentials, and `kkt agent`, the operator's
//! commands on agents in the local database.

use crate::commands::lanes::existing_db;
use crate::machine::connection::{ConnectionOptions, remote_client};
use crate::machine::credentials::{CredentialFile, watch_token};
use crate::machine::settings::read_config;
use kkt_agent_client as mcp_client;
use kkt_clock::SystemClock;
use kkt_config as config;
use kkt_names as names;
use kkt_rules::{Operator, Revoked};
use kkt_settings::Env;
use kkt_store::Db;
use std::path::PathBuf;
use std::sync::Arc;

/// What `kkt agent` does.
pub enum AgentAction {
    List,
    Revoke { handle: String },
}

pub async fn register(
    env: &Env,
    connection: ConnectionOptions,
    token_file: PathBuf,
    recovery_file: Option<PathBuf>,
    handle: Option<&str>,
    lane: Option<&str>,
    description: Option<String>,
) -> Result<(), String> {
    // The server checks the handle and lane too; checking first gives the
    // reason, and leaves no files behind.
    if let Some(handle) = handle {
        names::check_proposed_handle(handle, config::MAX_HANDLE_CHARS)?;
    }
    if let Some(lane) = lane {
        names::check_lane_name(lane, config::MAX_LANE_NAME_CHARS)?;
    }
    let client = remote_client(env, connection)?;
    let mut token = CredentialFile::create(token_file)?;
    let mut recovery = recovery_file.map(CredentialFile::create).transpose()?;
    let registered = client
        .register(handle, lane)
        .await
        .map_err(|e| match (&e, lane) {
            // Which lane, and why, is not a credential.
            (mcp_client::Failure::Refused { code, message }, Some(_))
                if code == "not_found" || message.contains("lane") =>
            {
                format!("the server refused the lane: {message}")
            }
            _ => avatar_error(e),
        })?;
    // Save credentials before any optional call that might fail.
    token.save(&registered.token)?;
    if let Some(recovery) = &mut recovery {
        recovery.save(&registered.recovery_code)?;
    }
    if let Some(description) = description {
        client
            .set_description(&registered.token, &description)
            .await
            .map_err(avatar_error)?;
    }
    println!("{}", registered.handle);
    Ok(())
}

pub async fn reclaim(
    env: &Env,
    connection: ConnectionOptions,
    handle: &str,
    recovery_file: PathBuf,
    token_file: PathBuf,
) -> Result<(), String> {
    let recovery = watch_token(Some(&recovery_file))?;
    let client = remote_client(env, connection)?;
    let mut token = CredentialFile::create(token_file)?;
    let reclaimed = client
        .reclaim(handle, &recovery)
        .await
        .map_err(avatar_error)?;
    token.save(&reclaimed.token)?;
    println!("{}", reclaimed.handle);
    Ok(())
}

// Do not echo server or transport error details for credential-bearing calls.
fn avatar_error(error: mcp_client::Failure) -> String {
    match error {
        mcp_client::Failure::Lost(_) => "could not reach the server or read its reply".into(),
        mcp_client::Failure::Refused { code, .. } => {
            format!("the server refused the avatar request ({code})")
        }
    }
}

/// `kkt agent`: the operator's commands, on the database `kkt serve` would
/// use unless `--db` names another. Never creates a database.
pub async fn run(
    env: &Env,
    command: AgentAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    // An explicit --db needs nothing from the config file, so it is not read.
    let config_db = match db {
        Some(_) => None,
        None => read_config(env, config)?.1.db,
    };
    let path = existing_db(env, db, config_db)?;
    let db = Db::open(&path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let operator = Operator::new(db, Arc::new(SystemClock));
    match command {
        AgentAction::List => {
            let agents = operator.agents().await.map_err(|e| e.to_string())?;
            if agents.is_empty() {
                println!("no agents in {}", path.display());
            }
            for a in agents {
                let seen = a.last_seen.as_deref().unwrap_or("never");
                let revoked = match &a.revoked_at {
                    Some(at) => format!("  revoked {at}"),
                    None => String::new(),
                };
                println!(
                    "{}  created {}  last seen {seen}{revoked}",
                    a.handle, a.created_at
                );
            }
        }
        AgentAction::Revoke { handle } => {
            match operator
                .revoke(handle.clone())
                .await
                .map_err(|e| e.to_string())?
            {
                Revoked::Now { at } => println!("revoked {handle} at {at}"),
                Revoked::Already { at } => println!("{handle} was already revoked at {at}"),
            }
        }
    }
    Ok(())
}
