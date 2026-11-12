//! `kkt watch`: waits for new messages from a shell, as an agent or through
//! the admin endpoint, printing one line for each.

use crate::{AgentParts, client};
use kkt_setup::connection::{ConnectionOptions, ConnectionSource};
use std::path::PathBuf;
use std::process::ExitCode;

/// The exit code of `kkt watch --once --timeout` when no message came.
const WATCH_TIMED_OUT_EXIT: u8 = 3;

/// What `kkt watch` watches, and where it keeps its place.
pub struct WatchTask {
    /// Through the admin endpoint, with the admin key, instead of as an agent.
    pub admin: bool,
    pub token_file: Option<PathBuf>,
    pub state_file: Option<PathBuf>,
    pub connection: ConnectionOptions,
    pub options: kkt_watch::Options,
}

/// Watches as `task` says, until it is done: the exit code is success
/// after messages, or [`WATCH_TIMED_OUT_EXIT`] when `--timeout` ran out.
pub async fn run<C: ConnectionSource>(
    parts: &mut AgentParts<C>,
    task: WatchTask,
) -> Result<ExitCode, String> {
    let server = if task.admin {
        parts
            .servers
            .admin(client::admin(parts, task.connection)?, parts.clock.clone())
    } else {
        let token = parts.credentials.token(task.token_file.as_deref())?;
        parts
            .servers
            .agent(client::agent(parts, task.connection)?, token)
    };
    let place = task
        .state_file
        .map(|path| parts.credentials.claim_state(path))
        .transpose()?;
    let outcome = kkt_watch::run(
        &*server,
        &*parts.clock,
        kkt_watch::Settings::DEFAULT,
        place.as_deref(),
        &mut *parts.report,
        &task.options,
    )
    .await?;
    Ok(match outcome {
        kkt_watch::Outcome::Received => ExitCode::SUCCESS,
        kkt_watch::Outcome::TimedOut => ExitCode::from(WATCH_TIMED_OUT_EXIT),
    })
}
