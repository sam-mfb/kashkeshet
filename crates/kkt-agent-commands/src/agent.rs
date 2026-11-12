//! `kkt register` and `kkt reclaim`: an avatar from a server, with its
//! credentials saved.

use crate::{AgentParts, client};
use kkt_agent_client::Failure;
use kkt_names as names;
use kkt_setup::connection::{ConnectionOptions, ConnectionSource};
use std::path::PathBuf;

/// What `kkt register` asks the server for: a handle, a lane to take, and
/// a description, each if given.
pub struct NewAvatar<'a> {
    pub handle: Option<&'a str>,
    pub lane: Option<&'a str>,
    pub description: Option<String>,
}

/// Registers a new avatar and saves its token to `token_file`, and its
/// recovery code to `recovery_file` if given, then prints its handle.
pub async fn register<C: ConnectionSource>(
    parts: &AgentParts<C>,
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
    let server = parts.servers.avatars(client::agent(parts, connection)?);
    let mut token = parts.credentials.reserve(token_file)?;
    let mut recovery = recovery_file
        .map(|path| parts.credentials.reserve(path))
        .transpose()?;
    let registered = server
        .register(handle, lane)
        .await
        .map_err(|e| match (&e, lane) {
            // Which lane, and why, is not a credential.
            (Failure::Refused { code, message }, Some(_)) if lane_refused(code, message) => {
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
        server
            .describe(&registered.token, &description)
            .await
            .map_err(avatar_error)?;
    }
    parts.output.output(&format!("{}\n", registered.handle));
    Ok(())
}

/// Takes over the avatar `handle` with the recovery code in
/// `recovery_file`, saves its new token to `token_file`, and prints its
/// handle.
pub async fn reclaim<C: ConnectionSource>(
    parts: &AgentParts<C>,
    connection: ConnectionOptions,
    handle: &str,
    recovery_file: PathBuf,
    token_file: PathBuf,
) -> Result<(), String> {
    let recovery = parts.credentials.token(Some(&recovery_file))?;
    let server = parts.servers.avatars(client::agent(parts, connection)?);
    let mut token = parts.credentials.reserve(token_file)?;
    let reclaimed = server
        .reclaim(handle, &recovery)
        .await
        .map_err(avatar_error)?;
    token.save(&reclaimed.token)?;
    parts.output.output(&format!("{}\n", reclaimed.handle));
    Ok(())
}

/// Whether `register` refused the lane it was asked to take, rather than the
/// avatar, judging by the refusal's code and message.
pub(crate) fn lane_refused(code: &str, message: &str) -> bool {
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
fn avatar_error(error: Failure) -> String {
    match error {
        // A refused key keeps the message it had when it counted as lost.
        Failure::Lost(_) | Failure::KeyRefused { .. } => {
            "could not reach the server or read its reply".into()
        }
        Failure::Refused { code, .. } => {
            format!("the server refused the avatar request ({code})")
        }
    }
}
