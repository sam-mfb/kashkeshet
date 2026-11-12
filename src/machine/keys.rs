//! The server's key reads, for `kkt serve` and the service check: the key
//! files its settings name, checked as kkt-admission checks them, and the
//! default key made if it is missing. (The operator's key finders and
//! writers are kkt-operate's.)
use crate::machine::settings::Sources;
use kkt_admission::server_keys::{self, KeyText, KeyTexts, ServerKeys};
use kkt_admission::{AccessKey, ServerKey, SharedKey};
use kkt_admission_os::{KeyFiles, ReadError};
use kkt_entropy_os::OsEntropy;
use std::path::Path;

/// Makes the default key file if it is the one in use and does not exist
/// yet, and logs where it is (never the key).
pub(crate) fn make_default_key(access_key: &AccessKey, kind: &str) -> Result<(), String> {
    if let Some(path) =
        kkt_admission_os::make_default_key(access_key, || SharedKey::generate(&OsEntropy))?
    {
        tracing::info!("created a new {kind} key at {}", path.display());
    }
    Ok(())
}

/// The server's keys from the files that `access`, `web` and `admin` name,
/// checked by [`server_keys::check`], which asks for each file only when
/// the keys before it passed. An admin key file that does not exist turns
/// `/mcp-admin` off. The errors name the bad setting, and the config file if
/// it set the access key.
pub(crate) fn read_server_keys(
    access: &AccessKey,
    web: &AccessKey,
    admin: Option<&Path>,
    sources: &Sources,
) -> Result<ServerKeys, String> {
    let mut files = ServerKeyFiles(KeyFiles::new(key_path(access), key_path(web), admin));
    // A name file is read only beside an admin key, so with none the name
    // refusal cannot happen; the path is then empty.
    let names = admin
        .map(kkt_admission::admin_name_path)
        .unwrap_or_default();
    server_keys::check(&mut files).map_err(|refusal| {
        let set_in = sources
            .set_in("access_key_file")
            .map(|p| p.display().to_string());
        kkt_admission::key_refusal(
            refusal,
            set_in.as_deref(),
            Some(&names.display().to_string()),
        )
    })
}

/// The selected path of an access or web key, if that key is on.
fn key_path(key: &AccessKey) -> Option<&Path> {
    match key {
        AccessKey::None => None,
        AccessKey::File(path) | AccessKey::Default(path) => Some(path),
    }
}

/// Adapts native key texts to admission, preserving the program's wording.
struct ServerKeyFiles<'a>(KeyFiles<'a>);

fn key_read_error(setting: &str, e: ReadError) -> String {
    format!(
        "invalid setting: {setting} could not be read from {}: {}",
        e.path.display(),
        e.error
    )
}
fn name_read_error(e: ReadError) -> String {
    format!(
        "invalid setting: the admin name could not be read from {}: {}",
        e.path.display(),
        e.error
    )
}
impl KeyTexts for ServerKeyFiles<'_> {
    type Error = String;
    // Serve makes a missing default first; service installation only reads.
    fn access(&mut self) -> Result<Option<KeyText>, String> {
        self.0
            .access()
            .map_err(|e| key_read_error("access_key_file", e))
    }
    fn web(&mut self) -> Result<Option<KeyText>, String> {
        self.0.web().map_err(|e| key_read_error("web_key_file", e))
    }
    fn admin(&mut self) -> Result<Option<KeyText>, String> {
        self.0
            .admin()
            .map_err(|e| key_read_error("admin_key_file", e))
    }
    fn admin_names(&mut self) -> Result<Option<String>, String> {
        self.0.admin_names().map_err(name_read_error)
    }
}

/// The text of the key file at `path`, which `setting` names.
fn read_key_text(path: &Path, setting: &str) -> Result<String, String> {
    KeyFiles::read(path).map_err(|e| key_read_error(setting, e))
}

/// The access key in the file at `path`.
pub(crate) fn read_key(path: &Path, sources: &Sources) -> Result<SharedKey, String> {
    let text = read_key_text(path, "access_key_file")?;
    SharedKey::new(ServerKey::Access, &text, path.display().to_string())
        .map_err(|e| sources.invalid(e))
}

#[cfg(test)]
mod tests {
    use crate::machine::operate::NativeOperate;
    use crate::machine::settings::{ServeArgs, ServeOptions};
    use crate::machine::settings::{TEST_PLATFORM, test_env, write_default_config};
    use crate::scratch;
    use kkt_machine::{Env, Platform};
    use kkt_operate::keys::KeyKind;
    use std::path::PathBuf;

    // kkt-operate's key finders, over this program's real parts.
    fn optional_key_path(
        env: &Env,
        platform: Platform,
        serve_args: ServeArgs,
        path: Option<PathBuf>,
        kind: KeyKind,
    ) -> Result<Option<PathBuf>, String> {
        let native = NativeOperate::new(env, platform).with_service_args(serve_args);
        kkt_operate::keys::optional_key_path(&native.parts(), env, platform, path, kind)
    }

    fn local_operator_key(
        env: &Env,
        platform: Platform,
        unfinished: Option<&kkt_setup::Unfinished>,
        unrecorded: Option<&kkt_setup::Unrecorded>,
        kind: KeyKind,
    ) -> Result<Option<PathBuf>, String> {
        let native = NativeOperate::new(env, platform);
        kkt_operate::keys::local_operator_key(&native.parts(), env, unfinished, unrecorded, kind)
    }

    fn unused_args(_: &[String]) -> Result<(Option<PathBuf>, ServeOptions), String> {
        Err("not used".into())
    }

    #[test]
    fn the_key_finders_use_the_default_key_and_no_database() {
        let home = scratch::Dir::new("keys-default");
        let env = test_env(Some(&home));
        let access = crate::machine::settings::default_access_key(&env).expect("key path");
        assert_eq!(
            optional_key_path(&env, TEST_PLATFORM, unused_args, None, KeyKind::Access),
            Ok(Some(access.clone()))
        );
        assert_eq!(
            local_operator_key(&env, TEST_PLATFORM, None, None, KeyKind::Web),
            Ok(Some(kkt_admission::web_key_path(&access)))
        );
    }

    #[test]
    fn the_key_finders_say_a_bad_setting_without_naming_its_file() {
        let home = scratch::Dir::new("keys-bad-setting");
        let env = test_env(Some(&home));
        write_default_config(&home, "no_access_key = true\naccess_key_file = \"/keys/k\"");
        let said = "no_access_key cannot be set together with access_key_file".to_string();
        assert_eq!(
            optional_key_path(&env, TEST_PLATFORM, unused_args, None, KeyKind::Access),
            Err(said.clone())
        );
        assert_eq!(
            local_operator_key(&env, TEST_PLATFORM, None, None, KeyKind::Web),
            Err(said)
        );
    }

    #[test]
    fn without_a_home_the_key_finders_find_no_default_key() {
        let said = "access_key_file has no default without a home directory: set HOME, \
                    or give access_key_file or no_access_key"
            .to_string();
        let env = test_env(None);
        assert_eq!(
            optional_key_path(&env, TEST_PLATFORM, unused_args, None, KeyKind::Access),
            Err(said.clone())
        );
        assert_eq!(
            local_operator_key(&env, TEST_PLATFORM, None, None, KeyKind::Admin),
            Err(said)
        );
    }

    #[test]
    fn the_key_finders_see_keys_turned_off() {
        let home = scratch::Dir::new("keys-off");
        let env = test_env(Some(&home));
        write_default_config(&home, "no_access_key = true");
        assert_eq!(
            optional_key_path(&env, TEST_PLATFORM, unused_args, None, KeyKind::Access),
            Err("keys are disabled in the serve config".to_string())
        );
        assert_eq!(
            local_operator_key(&env, TEST_PLATFORM, None, None, KeyKind::Web),
            Ok(None)
        );
    }
}
