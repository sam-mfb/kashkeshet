//! `kkt agent`, the operator's commands on agents in the local database,
//! which kkt-operate runs. (`kkt register` and `kkt reclaim` are
//! kkt-agent-commands'.)

use kkt_machine::{Env, Platform};
use std::path::PathBuf;

pub use kkt_operate::agent::AgentAction;

/// `kkt agent`: the operator's commands, on the database `kkt serve` would
/// use unless `--db` names another. Never creates a database.
pub async fn run(
    env: &Env,
    platform: Platform,
    command: AgentAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform);
    kkt_operate::agent::run(&native.parts(), command, db, config).await
}
