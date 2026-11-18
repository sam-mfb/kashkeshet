//! Where this machine's access, web and admin keys are, as kkt-deployment's
//! key finders decide over what is installed here and the serve config.

use crate::OperatorParts;
use kkt_deployment::Deployment;
use kkt_deployment::{
    self, RecordedServerRead, ServeConfigReads, ServerKeys, UnfinishedSetup, UnrecordedReads,
};
use kkt_machine::Env;
use kkt_setup as setup;
use std::path::PathBuf;

pub use kkt_deployment::{KeyKind, admin_key_collision};

/// The reads finding a key makes, over the operator's parts.
struct Reads<'a, 'p> {
    parts: &'a OperatorParts<'p>,
    env: &'a Env,
}

impl RecordedServerRead for Reads<'_, '_> {
    fn recorded(&self) -> Result<Option<ServerKeys>, String> {
        Ok(self.parts.installed.record()?.as_ref().map(server_keys))
    }
}

impl ServeConfigReads for Reads<'_, '_> {
    fn configured_access(&self) -> Result<Option<PathBuf>, String> {
        self.parts.serve.configured_access_key()
    }
    fn config_dir(&self) -> Option<PathBuf> {
        kkt_config_file::config_dir(self.env.config_home().as_deref())
    }
}

impl UnrecordedReads for Reads<'_, '_> {
    fn unfinished(&self) -> Result<Option<UnfinishedSetup>, String> {
        Ok(self
            .parts
            .installed
            .unfinished()?
            .as_ref()
            .map(unfinished_setup))
    }
    /// A server kkt installed here with no record. One that cannot be made
    /// out is said on standard error, as `kkt status` says it, and the keys
    /// are then looked for as the serve config says.
    fn unrecorded(&self) -> Option<ServerKeys> {
        match self.parts.installed.unrecorded() {
            Ok(found) => found.as_ref().map(found_server_keys),
            Err((_, e)) => {
                self.parts
                    .output
                    .errors(format_args!("Unrecorded install: {e}\n"));
                None
            }
        }
    }
}

/// Resolve a key without reading its contents or creating a file. An explicit
/// path names the selected key itself; otherwise use this machine's setup.
pub fn selected_key_path(
    parts: &OperatorParts<'_>,
    env: &Env,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<PathBuf, String> {
    optional_key_path(parts, env, path, kind)?.ok_or_else(|| "there is no admin key".into())
}

/// None means the access key occupies the implicit admin key's place.
pub fn optional_key_path(
    parts: &OperatorParts<'_>,
    env: &Env,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    kkt_deployment::find_key(&Reads { parts, env }, path, kind)
}

/// Whether this machine's server runs in Docker, as recorded or, with no
/// record and no unfinished setup, as found.
pub(crate) fn runs_in_docker(parts: &OperatorParts<'_>, env: &Env) -> Result<bool, String> {
    Ok(kkt_deployment::find_here(&Reads { parts, env })?.runs_in_docker())
}

/// Where this machine's web or admin key file is, if it runs a server: for
/// Docker, the host copy `kkt setup` saves, even while that setup is
/// `unfinished` or the server is `unrecorded`; otherwise the file the server
/// reads. The file need not exist.
pub fn local_operator_key(
    parts: &OperatorParts<'_>,
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    kkt_deployment::find_operator_key(
        &Reads { parts, env },
        unfinished.map(unfinished_setup).as_ref(),
        unrecorded.map(found_server_keys).as_ref(),
        kind,
    )
}

// What the setup record says, as kkt-deployment's key facts. The program
// hands this crate kkt-setup's records through `InstalledHere` until that
// contract is this crate's own (the kkt-setup breakup's pull request 8),
// so the mapping that kkt-install-commands' `deployment` module makes is
// made here too until then.

fn deployment(mode: &setup::Mode) -> Deployment {
    match mode {
        setup::Mode::Service { tailscale } => Deployment::Service {
            tailscale: *tailscale,
        },
        setup::Mode::Docker(docker) => Deployment::Docker {
            folder: docker.folder.clone(),
            tailscale: docker.tailscale,
            hostname: docker.hostname.clone(),
            tailnet: docker.tailnet.clone(),
        },
        setup::Mode::Client => Deployment::Client,
    }
}

fn server_keys(record: &setup::Record) -> ServerKeys {
    ServerKeys {
        deployment: deployment(&record.mode),
        key_file: record.key_file.clone(),
    }
}

fn found_server_keys(found: &setup::Unrecorded) -> ServerKeys {
    ServerKeys {
        deployment: deployment(&found.mode),
        key_file: found.key_file.clone(),
    }
}

fn unfinished_setup(unfinished: &setup::Unfinished) -> UnfinishedSetup {
    UnfinishedSetup {
        server: unfinished.record.as_ref().map(server_keys),
        in_docker: unfinished.purpose == setup::Purpose::Docker,
    }
}
