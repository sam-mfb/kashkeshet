//! `kkt setup`: what this machine is set up as (a server run as a service, a
//! server in Docker, or a client of a server elsewhere), the record that
//! says so, and which server this machine's clients use (`kkt switch`
//! changes it). It also decides, by how the machine is set up, where each
//! key file is, and which server installed with no record is here.
//!
//! The caller reads what is installed, answers questions through [`Ask`],
//! and carries out what is decided here, so every rule here is tested
//! directly.

mod adopt;
pub mod ask;
mod program;
pub use program::{
    admin_key_line, client_key, configured_name_pack_line, docker_status_lines,
    service_upgrade_error, staged_client_key, uninstall_line, uninstall_record, waiting_note,
    web_key_line,
};
mod flow;
mod installed_here;
mod key_finders;
mod key_writes;
mod keys;
mod local;
mod record;
mod switch;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(test)]
mod tests;

pub use adopt::{
    AdoptSteps, Unrecorded, service_runs_defaults, unreadable_service, unrecorded_here,
    unrecorded_service,
};
pub use ask::{Ask, Lines, Question, unanswered};
pub use flow::{Answers, ClientKey, Installed, Purpose, Steps, run};
pub use installed_here::InstalledHere;
pub use key_finders::{KeyReads, OperatorKeyReads, find_here, find_key, find_operator_key};
pub use key_writes::{new_key_file, write_admin_name, write_key_file};
pub use keys::{
    Here, KeyKind, KeyPlace, KeysOff, admin_key_collision, configured_key, create_asks_for_docker,
    create_checks, create_destination_check, docker_key_copies, operator_key, recorded_key,
};
pub use local::recorded_setup;
pub use record::{DockerSetup, InstallCheck, Mode, Record, Server, TailscaleLogin, Unfinished};
pub use switch::{Switch, SwitchSteps, Target, switch};

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

/// Whether `written_by`, the version a record names, says that version is
/// not known: the record adopts a service whose version nothing says.
pub fn version_not_known(written_by: &str) -> bool {
    written_by == SETUP_UNKNOWN_VERSION
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
