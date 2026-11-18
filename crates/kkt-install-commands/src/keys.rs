//! Where this machine's web and admin keys are, for the install's commands,
//! as kkt-deployment's key finder decides over the setup record and the
//! serve config.

use crate::InstallParts;
use crate::deployment::{found_server_keys, server_keys, unfinished_setup};
use crate::record::read_setup;
use kkt_deployment::connection::ConnectionSource;
use kkt_deployment::{RecordedServerRead, ServeConfigReads, ServerKeys};
use kkt_machine::Env;
use kkt_setup as setup;
use std::path::PathBuf;

pub(crate) use kkt_deployment::admin_name_line;
pub(crate) use kkt_deployment::{KeyKind, admin_key_collision, new_key_file};

/// The reads finding an operator key makes, on this machine.
struct EnvReads<'a, 'p, C: ConnectionSource, D>(&'a Env, &'a InstallParts<'p, C, D>);

impl<C: ConnectionSource, D> RecordedServerRead for EnvReads<'_, '_, C, D> {
    fn recorded(&self) -> Result<Option<ServerKeys>, kkt_deployment::KeyReadError> {
        Ok(read_setup(self.0, self.1.setup_files)
            .map_err(|said| kkt_deployment::KeyReadError(said.into()))?
            .as_ref()
            .map(server_keys))
    }
}

impl<C: ConnectionSource, D> ServeConfigReads for EnvReads<'_, '_, C, D> {
    fn configured_access(&self) -> Result<Option<PathBuf>, kkt_deployment::KeyReadError> {
        self.1
            .serve
            .configured_access_key()
            .map_err(|said| kkt_deployment::KeyReadError(said.into()))
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

/// Where this machine's key of `kind` is, if it runs a server, as
/// kkt-deployment's [`kkt_deployment::find_operator_key`] decides over this machine's
/// reads and the setup facts given.
pub fn local_operator_key<D>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    kkt_deployment::find_operator_key(
        &EnvReads(env, parts),
        unfinished.map(unfinished_setup).as_ref(),
        unrecorded.map(found_server_keys).as_ref(),
        kind,
    )
    .map_err(|e| e.to_string())
}
