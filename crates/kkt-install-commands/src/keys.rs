//! Where this machine's web and admin keys are, for the install's commands,
//! as kkt-setup's key finder decides over the setup record and the serve
//! config.

use crate::InstallParts;
use crate::record::read_setup;
use kkt_deployment::connection::ConnectionSource;
use kkt_machine::Env;
use kkt_setup as setup;
use std::path::PathBuf;

pub(crate) use kkt_deployment::admin_name_line;
pub(crate) use kkt_setup::{
    KeyKind, admin_key_collision, new_key_file, write_admin_name, write_key_file,
};

/// The reads finding an operator key makes, on this machine.
struct EnvReads<'a, 'p, C: ConnectionSource, D>(&'a Env, &'a InstallParts<'p, C, D>);

impl<C: ConnectionSource, D> setup::OperatorKeyReads for EnvReads<'_, '_, C, D> {
    fn setup(&self) -> Result<Option<setup::Record>, String> {
        read_setup(self.0, self.1.setup_files)
    }
    fn configured_access(&self) -> Result<Option<PathBuf>, String> {
        self.1.serve.configured_access_key()
    }
    fn config_dir(&self) -> Option<PathBuf> {
        kkt_config_file::config_dir(self.0.config_home().as_deref())
    }
}

/// This machine's web key file, if it runs a server. See
/// [`local_operator_key`].
pub fn local_web_key<D>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
) -> Result<Option<PathBuf>, String> {
    local_operator_key(parts, env, unfinished, unrecorded, KeyKind::Web)
}

/// Where this machine's web or admin key file is, if it runs a server: for
/// Docker, the host copy `kkt setup` saves, even while that setup is
/// `unfinished` or the server is `unrecorded`; otherwise the file the server
/// reads. The file need not exist.
pub fn local_operator_key<D>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    setup::find_operator_key(&EnvReads(env, parts), unfinished, unrecorded, kind)
}
