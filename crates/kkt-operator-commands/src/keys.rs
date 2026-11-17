//! Where this machine's access, web and admin keys are, as kkt-setup's key
//! finders decide over what is installed here and the serve config.

use crate::OperatorParts;
use kkt_machine::Env;
use kkt_setup as setup;
use std::path::PathBuf;

pub use kkt_setup::{KeyKind, admin_key_collision};

/// The reads finding a key makes, over the operator's parts.
struct Reads<'a, 'p> {
    parts: &'a OperatorParts<'p>,
    env: &'a Env,
}

impl setup::OperatorKeyReads for Reads<'_, '_> {
    fn setup(&self) -> Result<Option<setup::Record>, String> {
        self.parts.installed.record()
    }
    fn configured_access(&self) -> Result<Option<PathBuf>, String> {
        self.parts.serve.configured_access_key()
    }
    fn config_dir(&self) -> Option<PathBuf> {
        kkt_config_file::config_dir(self.env.config_home().as_deref())
    }
}

impl setup::KeyReads for Reads<'_, '_> {
    fn unfinished(&self) -> Result<Option<setup::Unfinished>, String> {
        self.parts.installed.unfinished()
    }
    /// A server kkt installed here with no record. One that cannot be made
    /// out is said on standard error, as `kkt status` says it, and the keys
    /// are then looked for as the serve config says.
    fn unrecorded(&self) -> Option<setup::Unrecorded> {
        match self.parts.installed.unrecorded() {
            Ok(found) => found,
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
    setup::find_key(&Reads { parts, env }, path, kind)
}

/// Whether this machine's server runs in Docker, as recorded or, with no
/// record and no unfinished setup, as found.
pub(crate) fn runs_in_docker(parts: &OperatorParts<'_>, env: &Env) -> Result<bool, String> {
    Ok(setup::find_here(&Reads { parts, env })?.runs_in_docker())
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
    setup::find_operator_key(&Reads { parts, env }, unfinished, unrecorded, kind)
}
