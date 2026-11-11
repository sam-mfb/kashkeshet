//! Agents: `kkt register` and `kkt reclaim`, which get an identity from a
//! server and save its credentials, and `kkt agent`, the operator's
//! commands on agents in the local database.

use crate::commands::lanes::existing_db;
use crate::machine::connection::{ConnectionOptions, remote_client};
use crate::machine::credentials::{CredentialFile, watch_token};
use crate::machine::settings::read_config;
use kkt_clock_system::SystemClock;
use kkt_machine::{Env, Platform};
use kkt_names as names;
use kkt_rules::{CommandLine, Revoked};
use kkt_terminal_os::Terminal;
use std::path::PathBuf;
use std::sync::Arc;

/// What `kkt agent` does.
pub enum AgentAction {
    List,
    Revoke { handle: String },
}

/// What `kkt register` asks the server for: a handle, a lane to take, and
/// a description, each if given.
pub struct NewAvatar<'a> {
    pub handle: Option<&'a str>,
    pub lane: Option<&'a str>,
    pub description: Option<String>,
}

pub async fn register(
    env: &Env,
    platform: Platform,
    connection: ConnectionOptions,
    token_file: PathBuf,
    recovery_file: Option<PathBuf>,
    asked: NewAvatar<'_>,
) -> Result<(), String> {
    let NewAvatar {
        handle,
        lane,
        description,
    } = asked;
    // The server checks the handle and lane too; checking first gives the
    // reason, and leaves no files behind.
    if let Some(handle) = handle {
        names::check_proposed_handle(handle, names::MAX_HANDLE_CHARS)?;
    }
    if let Some(lane) = lane {
        names::check_lane_name(lane, names::MAX_LANE_NAME_CHARS)?;
    }
    let client = remote_client(env, platform, connection)?;
    let mut token = CredentialFile::create(token_file)?;
    let mut recovery = recovery_file.map(CredentialFile::create).transpose()?;
    let registered = client
        .register(handle, lane)
        .await
        .map_err(|e| match (&e, lane) {
            // Which lane, and why, is not a credential.
            (kkt_agent_client::Failure::Refused { code, message }, Some(_))
                if lane_refused(code, message) =>
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
    Terminal.output(format_args!("{}\n", registered.handle));
    Ok(())
}

pub async fn reclaim(
    env: &Env,
    platform: Platform,
    connection: ConnectionOptions,
    handle: &str,
    recovery_file: PathBuf,
    token_file: PathBuf,
) -> Result<(), String> {
    let recovery = watch_token(Some(&recovery_file))?;
    let client = remote_client(env, platform, connection)?;
    let mut token = CredentialFile::create(token_file)?;
    let reclaimed = client
        .reclaim(handle, &recovery)
        .await
        .map_err(avatar_error)?;
    token.save(&reclaimed.token)?;
    Terminal.output(format_args!("{}\n", reclaimed.handle));
    Ok(())
}

/// Whether `register` refused the lane it was asked to take, rather than the
/// avatar, judging by the refusal's code and message.
fn lane_refused(code: &str, message: &str) -> bool {
    match code {
        "lane_unavailable" => true,
        // For older servers, which refused an unknown lane with `not_found`
        // and a retired one with `invalid_argument` naming the lane. A newer
        // server's `register` sends neither to `kkt register` for any other
        // reason, since it checks the handle and lane names first.
        "not_found" => true,
        "invalid_argument" => message.contains("lane"),
        _ => false,
    }
}

// Do not echo server or transport error details for credential-bearing calls.
fn avatar_error(error: kkt_agent_client::Failure) -> String {
    match error {
        // A refused key keeps the message it had when it counted as lost.
        kkt_agent_client::Failure::Lost(_) | kkt_agent_client::Failure::KeyRefused { .. } => {
            "could not reach the server or read its reply".into()
        }
        kkt_agent_client::Failure::Refused { code, .. } => {
            format!("the server refused the avatar request ({code})")
        }
    }
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

#[cfg(test)]
mod tests {
    use super::lane_refused;

    #[test]
    fn a_lane_refusal_is_known_by_its_code() {
        assert!(lane_refused(
            "lane_unavailable",
            "no lane named nope: `list_lanes` shows the lanes you can take."
        ));
        assert!(lane_refused(
            "lane_unavailable",
            "the lane gone is retired, so it can no longer be taken."
        ));
    }

    #[test]
    fn another_refusal_that_mentions_a_lane_is_about_the_avatar() {
        for code in ["handle_taken", "rate_limited", "limit_reached", "internal"] {
            assert!(
                !lane_refused(code, "the handle lane-keeper is taken"),
                "{code}"
            );
        }
    }

    #[test]
    fn an_older_servers_lane_refusals_are_still_known() {
        assert!(lane_refused(
            "not_found",
            "no lane named nope: `list_lanes` shows the lanes you can take."
        ));
        assert!(lane_refused(
            "invalid_argument",
            "the lane gone is retired, so it can no longer be taken."
        ));
        assert!(!lane_refused("invalid_argument", "the handle is too long"));
    }
}
