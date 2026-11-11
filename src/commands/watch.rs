//! `kkt watch`: waits for new messages from a shell, as an agent or through
//! the admin endpoint, printing one line for each.

mod server;
pub use server::WatchServer;

use crate::machine::connection::{ConnectionOptions, admin_client, remote_client};
use crate::machine::credentials::{StateFile, watch_token};
use crate::machine::settings::ServeArgs;
use kkt_clock::SystemClock;
use kkt_config as config;
use kkt_settings::Env;
use kkt_watch as watch;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

/// What `kkt watch` watches, and where it keeps its place.
pub struct WatchTask {
    /// Through the admin endpoint, with the admin key, instead of as an agent.
    pub admin: bool,
    pub token_file: Option<PathBuf>,
    pub state_file: Option<PathBuf>,
    pub connection: ConnectionOptions,
    pub options: watch::Options,
}

pub async fn run(
    env: &Env,
    serve_args: ServeArgs,
    task: WatchTask,
) -> Result<std::process::ExitCode, String> {
    let server = if task.admin {
        WatchServer::Admin(kkt_agent_client::AdminRemote::new(
            admin_client(env, serve_args, task.connection)?,
            Arc::new(SystemClock),
        ))
    } else {
        let token = watch_token(task.token_file.as_deref())?;
        WatchServer::Agent(kkt_agent_client::Remote::from_client(
            remote_client(env, task.connection)?,
            token,
        ))
    };
    let place = task.state_file.map(StateFile::claim).transpose()?;
    let outcome = watch::run(
        &server,
        &SystemClock,
        place.as_ref().map(|p| p as &dyn watch::Place),
        &mut WatchOutput,
        &task.options,
    )
    .await?;
    Ok(match outcome {
        watch::Outcome::Received => std::process::ExitCode::SUCCESS,
        watch::Outcome::TimedOut => std::process::ExitCode::from(config::WATCH_TIMED_OUT_EXIT),
    })
}

/// Message lines to standard output, flushed one by one; notes to
/// standard error.
struct WatchOutput;

impl watch::Report for WatchOutput {
    fn message(&mut self, line: &str) -> Result<(), String> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}")
            .and_then(|()| out.flush())
            .map_err(|e| format!("could not print a message: {e}"))
    }

    fn note(&mut self, text: &str) {
        eprintln!("kkt watch: {text}");
    }
}
