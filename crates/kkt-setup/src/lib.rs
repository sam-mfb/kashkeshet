//! `kkt setup`: what this machine is set up as (a server run as a service, a
//! server in Docker, or a client of a server elsewhere), the record that
//! says so, and what follows from it: whether `kkt serve` may run here, and
//! which server this machine's clients use (`kkt switch` changes it). It
//! also decides, by how the machine is set up, what `kkt status` reports,
//! what `kkt upgrade` does, which server `kkt lanes` and `kkt backup` act
//! on, which key files a local backup holds, where each key file is,
//! which server installed with no record is
//! here, and which setup `kkt doctor` goes by and which fixes it gives.
//!
//! Like `settings/`, this part reads no files and runs nothing itself. The
//! caller reads and writes the record's text, reads what is installed,
//! answers questions through [`Ask`], and carries out what is decided here,
//! so every rule here is tested directly.

mod acts_on;
mod adopt;
mod ask;
mod backup_keys;
mod doctor;
mod flow;
mod guard;
mod keys;
mod lifecycle;
mod record;
mod switch;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(test)]
mod tests;
mod upgrade;

pub use acts_on::{BackupOf, LanesOn, backup_of, lanes_on};
pub use adopt::{
    AdoptSteps, Unrecorded, service_runs_defaults, unreadable_service, unrecorded_here,
    unrecorded_service,
};
pub use ask::{Ask, Question, unanswered};
pub use backup_keys::{AccessKeyFrom, BackupKeys, Found, backup_keys};
pub use doctor::{
    KeyFix, ServerFix, clients_server_fix, clients_server_mode, doctor_record, key_fix, server_fix,
};
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

use kkt_machine::{Env, Platform};
use std::path::{Path, PathBuf};

/// The setup record `kkt setup` writes, in the config directory.
const SETUP_FILE_NAME: &str = "setup.toml";
/// Where `kkt setup` notes a server it installed but did not record, in the
/// config directory.
const SETUP_UNFINISHED_FILE_NAME: &str = "setup-unfinished.toml";
/// The `written_by` of a record `kkt setup` wrote for a service it found
/// installed with no record, whose version nothing says. `kkt upgrade`
/// always installs such a service again.
const SETUP_UNKNOWN_VERSION: &str = "unknown";

/// The database file's name in the per-user data directory, for `kkt serve`.
const DB_FILE_NAME: &str = "kashkeshet.db";
/// The per-user data directory on macOS, under the home directory.
const MACOS_DATA_DIR: &str = "Library/Application Support/kashkeshet";
/// The per-user data directory elsewhere, under `$XDG_DATA_HOME`.
const XDG_DATA_DIR: &str = "kashkeshet";
/// `$XDG_DATA_HOME` when it is not set, under the home directory.
const XDG_DATA_HOME_DEFAULT: &str = ".local/share";

/// The per-user data directory: `~/Library/Application Support/kashkeshet`
/// on macOS, `$XDG_DATA_HOME/kashkeshet` (or `~/.local/share/kashkeshet`)
/// elsewhere. The database and the managed Docker folder go here.
pub fn data_dir(env: &Env, platform: Platform) -> Option<PathBuf> {
    match platform {
        Platform::MacOs => Some(env.home.as_ref()?.join(MACOS_DATA_DIR)),
        Platform::Xdg => {
            let base = match env.absolute_xdg_data_home() {
                Some(dir) => dir.to_path_buf(),
                None => env.home.as_ref()?.join(XDG_DATA_HOME_DEFAULT),
            };
            Some(base.join(XDG_DATA_DIR))
        }
    }
}

/// The database `kkt serve` uses when none is given, in [`data_dir`].
pub fn default_db(env: &Env, platform: Platform) -> Option<PathBuf> {
    Some(data_dir(env, platform)?.join(DB_FILE_NAME))
}

/// Where the setup record is: `setup.toml` in kkt's config directory,
/// `config_dir`, beside the config file and the access key.
pub fn record_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETUP_FILE_NAME)
}

/// Where an unfinished setup is noted: `setup-unfinished.toml` beside the
/// record, in `config_dir`.
pub fn unfinished_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETUP_UNFINISHED_FILE_NAME)
}
