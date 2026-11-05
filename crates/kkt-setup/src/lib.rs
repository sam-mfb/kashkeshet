//! `kkt setup`: what this machine is set up as (a server run as a service, a
//! server in Docker, or a client of a server elsewhere), the record that
//! says so, and what follows from it: whether `kkt serve` may run here, and
//! which server this machine's clients use (`kkt switch` changes it). It
//! also decides, by how the machine is set up, what `kkt status` reports,
//! what `kkt upgrade` does, which server `kkt lanes` and `kkt backup` act
//! on, where each key file is, and which server installed with no record
//! is here.
//!
//! Like `settings/`, this part reads no files and runs nothing itself. The
//! caller reads and writes the record's text, reads what is installed,
//! answers questions through [`Ask`], and carries out what is decided here,
//! so every rule here is tested directly.

mod acts_on;
mod adopt;
mod ask;
mod flow;
mod guard;
mod keys;
mod lifecycle;
mod record;
mod switch;
#[cfg(test)]
mod tests;
mod upgrade;

pub use acts_on::{BackupOf, LanesOn, backup_of, lanes_on};
pub use adopt::{
    AdoptSteps, Unrecorded, service_runs_defaults, unreadable_service, unrecorded_here,
    unrecorded_service,
};
pub use ask::{Ask, Question, unanswered};
pub use flow::{Answers, ClientKey, Installed, Purpose, Steps, run};
pub use guard::guard;
pub use keys::{
    Here, KeyKind, KeyPlace, KeysOff, admin_key_collision, configured_key, create_asks_for_docker,
    create_checks, create_destination_check, operator_key, recorded_key,
};
pub use lifecycle::{
    Replace, RunningVersion, Status, StatusThen, UpgradePlan, running_version, status,
    status_looks_for_unrecorded, upgrade_plan,
};
pub use record::{DockerSetup, InstallCheck, Mode, Record, Server, TailscaleLogin, Unfinished};
pub use switch::{Switch, SwitchSteps, Target, switch};
pub use upgrade::{Upgrade, upgrade};

use kkt_config::{SETUP_FILE_NAME, SETUP_UNFINISHED_FILE_NAME};
use kkt_settings::Env;
use std::path::PathBuf;

/// Where the setup record is: `setup.toml` in the config directory, beside
/// the config file and the access key.
pub fn record_path(env: &Env) -> Option<PathBuf> {
    Some(env.config_dir()?.join(SETUP_FILE_NAME))
}

/// Where an unfinished setup is noted: `setup-unfinished.toml` beside the
/// record.
pub fn unfinished_path(env: &Env) -> Option<PathBuf> {
    Some(env.config_dir()?.join(SETUP_UNFINISHED_FILE_NAME))
}
