//! Waiting for a server `kkt setup` just set up to answer, and how long
//! setup waits by kind of setup.

use super::{Check, Inspector, unreachable};
use kkt_clock::Clock;
use kkt_config as config;
use std::path::Path;
use std::time::Duration;

/// Where the server setup checks runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupServer {
    /// On another machine: it answers or its address is wrong, so it is not
    /// waited for.
    Elsewhere,
    /// Set up on this machine just now: it may not answer yet.
    New,
}

/// How long to keep trying a server that does not answer, and how often.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait {
    pub total: Duration,
    pub every: Duration,
}

/// How long setup waits for `server` to answer.
pub fn setup_wait(server: SetupServer) -> Wait {
    Wait {
        total: match server {
            SetupServer::Elsewhere => Duration::ZERO,
            SetupServer::New => config::SETUP_SERVER_WAIT,
        },
        every: config::SETUP_SERVER_POLL,
    }
}

/// Checks the server at `url` with the key file at `key_file`, again every
/// `wait.every` while it does not answer at all, for up to `wait.total` on
/// `clock`. Calls `waiting` once, before the first wait, if it waits.
/// Returns the last checks.
pub async fn until_reachable(
    inspector: Inspector<'_>,
    clock: &dyn Clock,
    url: &str,
    key_file: Option<&Path>,
    wait: Wait,
    waiting: &mut dyn FnMut(),
) -> Result<Vec<Check>, String> {
    let end = clock.now() + wait.total;
    let mut said = false;
    loop {
        let began = clock.now();
        let checks = inspector.server(url, key_file).await?;
        if !unreachable(&checks) || began >= end {
            return Ok(checks);
        }
        if !said {
            waiting();
            said = true;
        }
        clock.sleep_until(began + wait.every).await;
    }
}
