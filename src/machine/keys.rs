//! The server's key reads, for `kkt serve` and the service check: the key
//! files its settings name, checked as kkt-admission checks them, and the
//! default key made if it is missing. (The key finders and writers are
//! kkt-setup's.)
use crate::machine::settings::{Invalid, Sources};
use kkt_admission::server_keys::KeyRefusal;
use kkt_admission::server_keys::{self, KeyTexts, ServerKeys};
use kkt_admission::{ServerKey, SharedKey};
use kkt_entropy::Entropy;
use kkt_entropy_os::OsEntropy;
use kkt_keys::{AccessKey, KeyFileError, KeyFiles, KeyText, ReadError, SelectedKeys};
use kkt_keys_os::OsKeyFiles;
use kkt_log::{Level, Log};
use kkt_log_tracing::TracingLog;
use std::path::Path;

/// Makes the default key file if it is the one in use and does not exist
/// yet, and logs where it is (never the key).
pub(crate) fn make_default_key(access_key: &AccessKey, kind: &str) -> Result<(), KeyFileError> {
    let mut generate =
        || OsEntropy.random_chars(kkt_keys::NEW_KEY_ALPHABET, kkt_keys::NEW_KEY_CHARS);
    if let Some(path) = OsKeyFiles.make_default_key(access_key, &mut generate)? {
        TracingLog.record(
            Level::Info,
            module_path!(),
            format_args!("created a new {kind} key at {}", path.display()),
            &[],
        );
    }
    Ok(())
}

/// A key setting whose file the server reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySetting {
    Access,
    Web,
    Admin,
}

impl std::fmt::Display for KeySetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeySetting::Access => "access_key_file",
            KeySetting::Web => "web_key_file",
            KeySetting::Admin => "admin_key_file",
        })
    }
}

/// Why the text of a key file or of the admin name file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum KeyTextError {
    #[error(
        "invalid setting: {setting} could not be read from {}: {}",
        source.path.display(),
        source.error
    )]
    Key {
        setting: KeySetting,
        #[source]
        source: ReadError,
    },
    #[error(
        "invalid setting: the admin name could not be read from {}: {}",
        .0.path.display(),
        .0.error
    )]
    AdminName(#[source] ReadError),
}

/// Why the server's keys were refused: a file that could not be read, or
/// kkt-admission's refusal of what it holds, in kkt-admission's words.
#[derive(Debug, thiserror::Error)]
pub enum ServerKeysError {
    #[error(transparent)]
    Text(#[from] KeyTextError),
    /// kkt-admission's `key_refusal` gives its words as text, until that
    /// contract gives a type.
    #[error("{0}")]
    Refused(String),
}

/// Why the access key in a file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ReadKeyError {
    #[error(transparent)]
    Text(#[from] KeyTextError),
    #[error(transparent)]
    Invalid(#[from] Invalid),
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
) -> Result<ServerKeys, ServerKeysError> {
    let mut files = ServerKeyFiles(SelectedKeys {
        access: key_path(access),
        web: key_path(web),
        admin,
    });
    // A name file is read only beside an admin key, so with none the name
    // refusal cannot happen; the path is then empty.
    let names = admin.map(kkt_keys::admin_name_path).unwrap_or_default();
    server_keys::check(&mut files).map_err(|refusal| {
        let set_in = sources
            .set_in("access_key_file")
            .map(|p| p.display().to_string());
        let refusal = match refusal {
            KeyRefusal::Text(e) => return ServerKeysError::Text(e),
            KeyRefusal::AccessKey(e) => KeyRefusal::AccessKey(e),
            KeyRefusal::WebKeyReused => KeyRefusal::WebKeyReused,
            KeyRefusal::WebKey(e) => KeyRefusal::WebKey(e),
            KeyRefusal::AdminKeyReused => KeyRefusal::AdminKeyReused,
            KeyRefusal::AdminKey(e) => KeyRefusal::AdminKey(e),
            KeyRefusal::AdminName(e) => KeyRefusal::AdminName(e),
        };
        ServerKeysError::Refused(kkt_admission::key_refusal(
            refusal,
            set_in.as_deref(),
            Some(&names.display().to_string()),
        ))
    })
}

/// The selected path of an access or web key, if that key is on.
fn key_path(key: &AccessKey) -> Option<&Path> {
    match key {
        AccessKey::None => None,
        AccessKey::File(path) | AccessKey::Default(path) => Some(path),
    }
}

/// Adapts the key files on disk to admission, preserving the program's
/// wording.
struct ServerKeyFiles<'a>(SelectedKeys<'a>);

/// The key `setting` names could not be read.
fn key_read_error(setting: KeySetting) -> impl Fn(ReadError) -> KeyTextError {
    move |source| KeyTextError::Key { setting, source }
}

impl KeyTexts for ServerKeyFiles<'_> {
    type Error = KeyTextError;
    // Serve makes a missing default first; service installation only reads.
    fn access(&mut self) -> Result<Option<KeyText>, KeyTextError> {
        OsKeyFiles
            .read_access(&self.0)
            .map_err(key_read_error(KeySetting::Access))
    }
    fn web(&mut self) -> Result<Option<KeyText>, KeyTextError> {
        OsKeyFiles
            .read_web(&self.0)
            .map_err(key_read_error(KeySetting::Web))
    }
    fn admin(&mut self) -> Result<Option<KeyText>, KeyTextError> {
        OsKeyFiles
            .read_admin(&self.0)
            .map_err(key_read_error(KeySetting::Admin))
    }
    fn admin_names(&mut self) -> Result<Option<String>, KeyTextError> {
        OsKeyFiles
            .read_admin_names(&self.0)
            .map_err(KeyTextError::AdminName)
    }
}

/// The access key in the file at `path`.
pub(crate) fn read_key(path: &Path, sources: &Sources) -> Result<SharedKey, ReadKeyError> {
    let text = OsKeyFiles
        .read_key(path)
        .map_err(key_read_error(KeySetting::Access))?;
    Ok(
        SharedKey::new(ServerKey::Access, &text, path.display().to_string())
            .map_err(|e| sources.invalid(e))?,
    )
}

#[cfg(test)]
mod tests {
    use crate::machine::native::NativeParts;
    use crate::machine::settings::{ServeArgs, ServeOptions};
    use crate::machine::settings::{TEST_PLATFORM, test_env, write_default_config};
    use crate::scratch;
    use kkt_deployment::KeyKind;
    use kkt_machine::{Env, Platform};
    use std::path::PathBuf;

    // The key finders, over this program's real parts.
    fn optional_key_path(
        env: &Env,
        platform: Platform,
        serve_args: ServeArgs,
        path: Option<PathBuf>,
        kind: KeyKind,
    ) -> Result<Option<PathBuf>, String> {
        let native = NativeParts::new(env, platform).with_service_args(serve_args);
        let setup_lanes = native.setup_lanes();
        let install = native.install_parts(&setup_lanes);
        let installed = kkt_install_commands::record::InstallReads::new(&install, env, platform);
        let installed = crate::machine::native::InstalledOnThisMachine(&installed);
        let lane_servers = native.lane_servers(&installed);
        kkt_operator_commands::keys::optional_key_path(
            &native.operator_parts(&installed, &lane_servers),
            path,
            kind,
        )
        .map_err(|e| e.to_string())
    }

    fn local_operator_key(
        env: &Env,
        platform: Platform,
        unfinished: Option<&kkt_setup::Unfinished>,
        unrecorded: Option<&kkt_setup::Unrecorded>,
        kind: KeyKind,
    ) -> Result<Option<PathBuf>, String> {
        let native = NativeParts::new(env, platform);
        let setup_lanes = native.setup_lanes();
        kkt_install_commands::keys::local_operator_key(
            &native.install_parts(&setup_lanes),
            unfinished,
            unrecorded,
            kind,
        )
        .map_err(|e| e.to_string())
    }

    fn unused_args(
        _: &[String],
    ) -> Result<(Option<PathBuf>, ServeOptions), Box<dyn std::error::Error + Send + Sync>> {
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
            Ok(Some(kkt_keys::web_key_path(&access)))
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

    /// A Docker setup whose install finished but whose record waits for
    /// its remaining steps: what is installed here reads it as unfinished,
    /// never as recorded, so the access key is still the serve config's,
    /// while the web key is the host copy that setup saves.
    #[test]
    fn an_unfinished_setup_is_not_read_as_recorded() {
        use kkt_setup::{DockerSetup, Mode, Purpose, Record, Unfinished};
        let home = scratch::Dir::new("keys-unfinished");
        let env = test_env(Some(&home));
        let config_dir =
            kkt_config_file::config_dir(env.config_home().as_deref()).expect("config dir");
        let folder = home.join("docker");
        std::fs::create_dir_all(&folder).expect("docker folder");
        std::fs::write(folder.join(".env"), "KKT_VERSION=0.10.0\n").expect("settings file");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        let unfinished = Unfinished {
            purpose: Purpose::Docker,
            tailscale: false,
            allow_hosts: vec![],
            clients: vec![],
            tailscale_login: None,
            record: Some(Record {
                mode: Mode::Docker(DockerSetup {
                    folder,
                    tailscale: false,
                    hostname: None,
                    tailnet: None,
                }),
                written_by: "0.10.0".into(),
                url: "http://127.0.0.1:7420/mcp".into(),
                key_file: Some(config_dir.join("recorded-access-key")),
                clients: None,
            }),
        };
        std::fs::write(
            kkt_setup::unfinished_path(&config_dir),
            unfinished.render().expect("note"),
        )
        .expect("unfinished note");
        let access = crate::machine::settings::default_access_key(&env).expect("key path");
        assert_eq!(
            optional_key_path(&env, TEST_PLATFORM, unused_args, None, KeyKind::Access),
            Ok(Some(access))
        );
        assert_eq!(
            optional_key_path(&env, TEST_PLATFORM, unused_args, None, KeyKind::Web),
            Ok(Some(config_dir.join(kkt_keys::WEB_KEY_COPY_FILE_NAME)))
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
