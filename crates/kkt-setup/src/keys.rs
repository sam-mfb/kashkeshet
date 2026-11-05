//! Where this machine's access, web and admin key files are, decided from
//! the setup record, the note of an unfinished setup, a server found
//! installed with no record, or the serve config, with the host copies a
//! Docker setup keeps; and what `kkt key create` refuses before it writes.
//! The program reads the record and the rest, one at a time, and hands each
//! result to these functions; it also reads and writes the key files.

use super::adopt::Unrecorded;
use super::flow::Purpose;
use super::record::{Mode, Record, Unfinished};
use kkt_config::{
    ACCESS_KEY_FILE_NAME, DOCKER_ADMIN_KEY_FILE_NAME, DOCKER_WEB_KEY_FILE_NAME,
    MAX_ADMIN_NAME_CHARS,
};
use kkt_names::AdminName;
use kkt_settings::{admin_key_path, web_key_path};
use std::path::{Path, PathBuf};

/// Which of the server's keys a `kkt key` command is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// The agents' access key, for `/mcp`.
    Access,
    /// The operator's web page key.
    Web,
    /// The admin agent's key, for `/mcp-admin`.
    Admin,
}

impl KeyKind {
    /// Its name in messages, such as "web key".
    pub fn name(self) -> &'static str {
        match self {
            KeyKind::Access => "access key",
            KeyKind::Web => "web key",
            KeyKind::Admin => "admin key",
        }
    }

    /// The `kkt key` flag that selects it.
    pub fn flag(self) -> &'static str {
        match self {
            KeyKind::Access => "",
            KeyKind::Web => " --web",
            KeyKind::Admin => " --admin",
        }
    }

    /// The server's file for this key, given its access key's file. `None`
    /// for the admin key when the access key is in its place.
    pub fn beside(self, access: &Path) -> Option<PathBuf> {
        match self {
            KeyKind::Access => Some(access.to_path_buf()),
            KeyKind::Web => Some(web_key_path(access)),
            KeyKind::Admin => admin_key_path(access),
        }
    }

    /// A Docker setup's host copy of this key, given the access key's copy.
    pub fn docker_copy(self, access: &Path) -> PathBuf {
        match self {
            KeyKind::Access => access.to_path_buf(),
            KeyKind::Web => access.with_file_name(DOCKER_WEB_KEY_FILE_NAME),
            KeyKind::Admin => access.with_file_name(DOCKER_ADMIN_KEY_FILE_NAME),
        }
    }

    /// Whether, with no record, `kkt key` looks for this key where an
    /// unfinished setup or a server found with no record keeps it: only for
    /// the web and admin keys. The access key is then always the serve
    /// config's.
    pub fn looks_past_record(self) -> bool {
        self != KeyKind::Access
    }
}

/// What is known about the server on this machine, for telling whether it
/// runs in Docker: in order, its record, the note of an unfinished setup,
/// or a server found installed with no record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Here {
    Recorded(Record),
    Unfinished(Unfinished),
    Unrecorded(Unrecorded),
    Nothing,
}

impl Here {
    /// Whether the server here runs in Docker, as recorded or as found. An
    /// unfinished setup does not say.
    pub fn runs_in_docker(&self) -> bool {
        match self {
            Here::Recorded(record) => matches!(record.mode, Mode::Docker(_)),
            Here::Unrecorded(found) => matches!(found.mode, Mode::Docker(_)),
            Here::Unfinished(_) | Here::Nothing => false,
        }
    }
}

/// Where an operator key file is ([`operator_key`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyPlace {
    /// This file, or `None` when there is no such key here.
    Known(Option<PathBuf>),
    /// Beside the access key the serve config names: the caller reads the
    /// serve config and asks [`configured_key`].
    Configured,
}

/// What a key is when the serve config turns keys off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeysOff {
    /// There is no such key.
    NoKey,
    /// An error: `kkt key` was asked for a key, and there are none.
    Refuse,
}

/// The key of `kind` beside `access`, the access key file the serve config
/// names (`None` when it turns keys off, which `off` decides about).
pub fn configured_key(
    access: Option<&Path>,
    kind: KeyKind,
    off: KeysOff,
) -> Result<Option<PathBuf>, String> {
    match (access, off) {
        (Some(access), _) => Ok(kind.beside(access)),
        (None, KeysOff::NoKey) => Ok(None),
        (None, KeysOff::Refuse) => Err("keys are disabled in the serve config".into()),
    }
}

/// Where the key of `kind` that `kkt key` reads or replaces is, on a
/// machine with the setup `record`: for Docker, the host copy. A client
/// machine has only the access key, and a record with no access key has no
/// keys.
pub fn recorded_key(record: &Record, kind: KeyKind) -> Result<Option<PathBuf>, String> {
    if kind != KeyKind::Access && matches!(record.mode, Mode::Client) {
        return Err(format!(
            "the {} is on the server's machine; run `kkt key show{}` there",
            kind.name(),
            kind.flag()
        ));
    }
    let access = record
        .key_file
        .as_deref()
        .ok_or("keys are disabled for this setup")?;
    if matches!(record.mode, Mode::Docker(_)) {
        return Ok(Some(kind.docker_copy(access)));
    }
    Ok(kind.beside(access))
}

/// Where this machine's key of `kind` is, if it runs a server: for Docker,
/// the host copy `kkt setup` saves, even while that setup is `unfinished` or
/// the server is `unrecorded`; otherwise the file the server reads. The file
/// need not exist. `recorded` is the setup record; `config_dir` is kkt's
/// config directory, `None` without a home.
pub fn operator_key(
    recorded: Option<&Record>,
    unfinished: Option<&Unfinished>,
    unrecorded: Option<&Unrecorded>,
    config_dir: Option<&Path>,
    kind: KeyKind,
) -> Result<KeyPlace, String> {
    let server = match recorded {
        Some(record) => Some((&record.mode, record.key_file.as_deref())),
        None => unfinished
            .and_then(|unfinished| unfinished.record.as_ref())
            .map(|record| (&record.mode, record.key_file.as_deref()))
            .or_else(|| unrecorded.map(|found| (&found.mode, found.key_file.as_deref()))),
    };
    if let Some((mode, key_file)) = server {
        if matches!(mode, Mode::Client) {
            return Ok(KeyPlace::Known(None));
        }
        return Ok(KeyPlace::Known(key_file.and_then(|access| {
            if matches!(mode, Mode::Docker(_)) {
                Some(kind.docker_copy(access))
            } else {
                kind.beside(access)
            }
        })));
    }
    if let Some(unfinished) = unfinished
        && unfinished.purpose == Purpose::Docker
    {
        let access = config_dir
            .ok_or_else(|| format!("set HOME to find the {}", kind.name()))?
            .join(ACCESS_KEY_FILE_NAME);
        return Ok(KeyPlace::Known(Some(kind.docker_copy(&access))));
    }
    Ok(KeyPlace::Configured)
}

/// An access key may keep its old name, but cannot also be the admin key.
pub fn admin_key_collision() -> String {
    "cannot add an admin key: access_key_file is named admin-key, which is also the admin key's place in that folder; move one of the files to a different folder, or rename the access key and update access_key_file".into()
}

/// Whether `kkt key create` must first ask whether the server runs in
/// Docker: for a web or admin key with no path given, as the server in
/// Docker reads those keys inside its container.
pub fn create_asks_for_docker(kind: KeyKind, path_given: bool) -> bool {
    kind != KeyKind::Access && !path_given
}

/// What `kkt key create` refuses before it writes anything, in order: a web
/// or admin key of a server `in_docker` (as [`create_asks_for_docker`]
/// says to ask), whose host copies only mirror the container's; and a
/// `name` for any key but the admin key, or one that is not a valid admin
/// name. Returns the admin name to tie to the new key.
pub fn create_checks(
    kind: KeyKind,
    in_docker: bool,
    name: Option<&str>,
) -> Result<Option<AdminName>, String> {
    if in_docker {
        return Err(match kind {
            KeyKind::Admin => "make or rotate the admin key inside the container: `docker compose exec kkt kkt key create --admin --force --name <name>`, then restart kkt and rerun setup with --admin-key to save the host copy".into(),
            _ => "rotate the web key inside the container: `docker compose exec kkt kkt key create --web --force`, then restart kkt and rerun setup to refresh the host copy".into(),
        });
    }
    if name.is_some() && kind != KeyKind::Admin {
        return Err("--name names the admin agent, so it goes with --admin".into());
    }
    name.map(|name| AdminName::new(name, MAX_ADMIN_NAME_CHARS))
        .transpose()
}

/// What `kkt key create --admin --path` refuses before it writes anything,
/// after [`create_checks`]: a `destination` that is the access key's file,
/// `access`. The caller resolves both paths the same way first, so that
/// relative paths and symlinks compare as the files they name.
pub fn create_destination_check(destination: &Path, access: &Path) -> Result<(), String> {
    if destination == access {
        return Err(admin_key_collision());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DockerSetup;

    const KINDS: [KeyKind; 3] = [KeyKind::Access, KeyKind::Web, KeyKind::Admin];

    fn docker_mode() -> Mode {
        Mode::Docker(DockerSetup {
            folder: PathBuf::from("/data/docker"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        })
    }

    fn record(mode: Mode, key_file: Option<&str>) -> Record {
        Record {
            mode,
            written_by: "0.5.0".into(),
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: key_file.map(PathBuf::from),
            clients: None,
        }
    }

    fn unfinished(purpose: Purpose, record: Option<Record>) -> Unfinished {
        Unfinished {
            purpose,
            tailscale: false,
            allow_hosts: Vec::new(),
            clients: Vec::new(),
            tailscale_login: None,
            record,
        }
    }

    fn unrecorded(mode: Mode, key_file: Option<&str>) -> Unrecorded {
        Unrecorded {
            mode,
            url: "http://127.0.0.1:9000/mcp".into(),
            key_file: key_file.map(PathBuf::from),
        }
    }

    fn path(path: &str) -> Option<PathBuf> {
        Some(PathBuf::from(path))
    }

    fn known(file: &str) -> KeyPlace {
        KeyPlace::Known(path(file))
    }

    fn home() -> Option<&'static Path> {
        Some(Path::new("/home/k/.config/kkt"))
    }

    #[test]
    fn each_key_beside_the_access_key_and_in_docker() {
        let access = Path::new("/keys/access-key");
        assert_eq!(KeyKind::Access.beside(access), path("/keys/access-key"));
        assert_eq!(KeyKind::Web.beside(access), path("/keys/web-key"));
        assert_eq!(KeyKind::Admin.beside(access), path("/keys/admin-key"));
        assert_eq!(KeyKind::Admin.beside(Path::new("/keys/admin-key")), None);
        assert_eq!(
            KeyKind::Access.docker_copy(access),
            PathBuf::from("/keys/access-key")
        );
        assert_eq!(
            KeyKind::Web.docker_copy(access),
            PathBuf::from("/keys/docker-web-key")
        );
        assert_eq!(
            KeyKind::Admin.docker_copy(access),
            PathBuf::from("/keys/docker-admin-key")
        );
    }

    #[test]
    fn each_key_names_itself_and_its_flag() {
        let named: Vec<_> = KINDS.iter().map(|k| (k.name(), k.flag())).collect();
        assert_eq!(
            named,
            [
                ("access key", ""),
                ("web key", " --web"),
                ("admin key", " --admin")
            ]
        );
    }

    #[test]
    fn only_the_web_and_admin_keys_look_past_a_missing_record() {
        assert!(!KeyKind::Access.looks_past_record());
        assert!(KeyKind::Web.looks_past_record());
        assert!(KeyKind::Admin.looks_past_record());
    }

    #[test]
    fn the_server_runs_in_docker_as_recorded_or_found() {
        assert!(Here::Recorded(record(docker_mode(), None)).runs_in_docker());
        assert!(!Here::Recorded(record(Mode::Service { tailscale: false }, None)).runs_in_docker());
        assert!(!Here::Recorded(record(Mode::Client, None)).runs_in_docker());
        assert!(Here::Unrecorded(unrecorded(docker_mode(), None)).runs_in_docker());
        assert!(
            !Here::Unrecorded(unrecorded(Mode::Service { tailscale: false }, None))
                .runs_in_docker()
        );
        let noted = unfinished(Purpose::Docker, Some(record(docker_mode(), None)));
        assert!(!Here::Unfinished(noted).runs_in_docker());
        assert!(!Here::Nothing.runs_in_docker());
    }

    #[test]
    fn a_configured_key_is_beside_the_configs_access_key() {
        let access = Some(Path::new("/keys/access-key"));
        for off in [KeysOff::NoKey, KeysOff::Refuse] {
            assert_eq!(
                configured_key(access, KeyKind::Access, off),
                Ok(path("/keys/access-key"))
            );
            assert_eq!(
                configured_key(access, KeyKind::Web, off),
                Ok(path("/keys/web-key"))
            );
            assert_eq!(
                configured_key(access, KeyKind::Admin, off),
                Ok(path("/keys/admin-key"))
            );
            assert_eq!(
                configured_key(Some(Path::new("/keys/admin-key")), KeyKind::Admin, off),
                Ok(None)
            );
        }
    }

    #[test]
    fn with_keys_off_in_the_config_there_is_no_key_or_a_refusal() {
        for kind in KINDS {
            assert_eq!(configured_key(None, kind, KeysOff::NoKey), Ok(None));
            assert_eq!(
                configured_key(None, kind, KeysOff::Refuse),
                Err("keys are disabled in the serve config".to_string())
            );
        }
    }

    #[test]
    fn a_recorded_key_of_a_service_is_beside_its_access_key() {
        let service = record(Mode::Service { tailscale: false }, Some("/k/access-key"));
        assert_eq!(
            recorded_key(&service, KeyKind::Access),
            Ok(path("/k/access-key"))
        );
        assert_eq!(recorded_key(&service, KeyKind::Web), Ok(path("/k/web-key")));
        assert_eq!(
            recorded_key(&service, KeyKind::Admin),
            Ok(path("/k/admin-key"))
        );
        let collision = record(Mode::Service { tailscale: false }, Some("/k/admin-key"));
        assert_eq!(recorded_key(&collision, KeyKind::Admin), Ok(None));
    }

    #[test]
    fn a_recorded_key_in_docker_is_the_host_copy() {
        let docker = record(docker_mode(), Some("/d/docker-access-key"));
        assert_eq!(
            recorded_key(&docker, KeyKind::Access),
            Ok(path("/d/docker-access-key"))
        );
        assert_eq!(
            recorded_key(&docker, KeyKind::Web),
            Ok(path("/d/docker-web-key"))
        );
        assert_eq!(
            recorded_key(&docker, KeyKind::Admin),
            Ok(path("/d/docker-admin-key"))
        );
    }

    #[test]
    fn a_record_with_no_access_key_has_no_keys() {
        for mode in [
            Mode::Service { tailscale: false },
            docker_mode(),
            Mode::Client,
        ] {
            assert_eq!(
                recorded_key(&record(mode, None), KeyKind::Access),
                Err("keys are disabled for this setup".to_string())
            );
        }
    }

    #[test]
    fn on_a_client_only_the_access_key_has_a_place() {
        let client = record(Mode::Client, Some("/k/client-access-key"));
        assert_eq!(
            recorded_key(&client, KeyKind::Access),
            Ok(path("/k/client-access-key"))
        );
        assert_eq!(
            recorded_key(&client, KeyKind::Web),
            Err("the web key is on the server's machine; run `kkt key show --web` there".into())
        );
        assert_eq!(
            recorded_key(&client, KeyKind::Admin),
            Err(
                "the admin key is on the server's machine; run `kkt key show --admin` there".into()
            )
        );
    }

    #[test]
    fn an_operator_key_of_a_recorded_server() {
        let service = record(Mode::Service { tailscale: false }, Some("/keys/access-key"));
        assert_eq!(
            operator_key(Some(&service), None, None, home(), KeyKind::Access),
            Ok(known("/keys/access-key"))
        );
        assert_eq!(
            operator_key(Some(&service), None, None, home(), KeyKind::Web),
            Ok(known("/keys/web-key"))
        );
        assert_eq!(
            operator_key(Some(&service), None, None, home(), KeyKind::Admin),
            Ok(known("/keys/admin-key"))
        );
        let docker = record(docker_mode(), Some("/d/docker-access-key"));
        assert_eq!(
            operator_key(Some(&docker), None, None, home(), KeyKind::Access),
            Ok(known("/d/docker-access-key"))
        );
        assert_eq!(
            operator_key(Some(&docker), None, None, home(), KeyKind::Web),
            Ok(known("/d/docker-web-key"))
        );
        assert_eq!(
            operator_key(Some(&docker), None, None, home(), KeyKind::Admin),
            Ok(known("/d/docker-admin-key"))
        );
        for kind in KINDS {
            let client = record(Mode::Client, Some("/k"));
            assert_eq!(
                operator_key(Some(&client), None, None, home(), kind),
                Ok(KeyPlace::Known(None))
            );
            let keyless = record(docker_mode(), None);
            assert_eq!(
                operator_key(Some(&keyless), None, None, home(), kind),
                Ok(KeyPlace::Known(None))
            );
        }
    }

    #[test]
    fn the_record_comes_before_an_unfinished_or_unrecorded_server() {
        let service = record(Mode::Service { tailscale: false }, Some("/keys/access-key"));
        let noted = unfinished(Purpose::Docker, Some(record(docker_mode(), Some("/d/k"))));
        let found = unrecorded(docker_mode(), Some("/d/k"));
        assert_eq!(
            operator_key(
                Some(&service),
                Some(&noted),
                Some(&found),
                home(),
                KeyKind::Web
            ),
            Ok(known("/keys/web-key"))
        );
    }

    #[test]
    fn an_operator_key_of_a_finished_unrecorded_install_uses_its_record() {
        let noted = unfinished(
            Purpose::Docker,
            Some(record(docker_mode(), Some("/d/docker-access-key"))),
        );
        assert_eq!(
            operator_key(None, Some(&noted), None, None, KeyKind::Admin),
            Ok(known("/d/docker-admin-key"))
        );
        let client = unfinished(Purpose::Service, Some(record(Mode::Client, Some("/k"))));
        assert_eq!(
            operator_key(None, Some(&client), None, home(), KeyKind::Web),
            Ok(KeyPlace::Known(None))
        );
    }

    #[test]
    fn an_operator_key_of_an_unfinished_docker_install_is_the_host_copy() {
        let noted = unfinished(Purpose::Docker, None);
        assert_eq!(
            operator_key(None, Some(&noted), None, home(), KeyKind::Web),
            Ok(known("/home/k/.config/kkt/docker-web-key"))
        );
        assert_eq!(
            operator_key(None, Some(&noted), None, home(), KeyKind::Admin),
            Ok(known("/home/k/.config/kkt/docker-admin-key"))
        );
        assert_eq!(
            operator_key(None, Some(&noted), None, None, KeyKind::Web),
            Err("set HOME to find the web key".to_string())
        );
        assert_eq!(
            operator_key(None, Some(&noted), None, None, KeyKind::Admin),
            Err("set HOME to find the admin key".to_string())
        );
    }

    #[test]
    fn an_operator_key_of_an_unfinished_service_or_nothing_is_the_configs() {
        let noted = unfinished(Purpose::Service, None);
        for kind in KINDS {
            assert_eq!(
                operator_key(None, Some(&noted), None, None, kind),
                Ok(KeyPlace::Configured)
            );
            assert_eq!(
                operator_key(None, None, None, None, kind),
                Ok(KeyPlace::Configured)
            );
        }
    }

    #[test]
    fn an_operator_key_of_an_unrecorded_server() {
        let docker = unrecorded(docker_mode(), Some("/d/docker-access-key"));
        assert_eq!(
            operator_key(None, None, Some(&docker), None, KeyKind::Web),
            Ok(known("/d/docker-web-key"))
        );
        let service = unrecorded(Mode::Service { tailscale: false }, Some("/k/access-key"));
        assert_eq!(
            operator_key(None, None, Some(&service), None, KeyKind::Admin),
            Ok(known("/k/admin-key"))
        );
        let keyless = unrecorded(Mode::Service { tailscale: false }, None);
        assert_eq!(
            operator_key(None, None, Some(&keyless), None, KeyKind::Web),
            Ok(KeyPlace::Known(None))
        );
    }

    #[test]
    fn the_admin_key_collision_says_how_to_fix_it() {
        assert_eq!(
            admin_key_collision(),
            "cannot add an admin key: access_key_file is named admin-key, which is also the \
             admin key's place in that folder; move one of the files to a different folder, \
             or rename the access key and update access_key_file"
        );
    }

    #[test]
    fn an_admin_key_cannot_be_created_at_the_access_keys_path() {
        assert_eq!(
            create_destination_check(Path::new("/k/access-key"), Path::new("/k/access-key")),
            Err(admin_key_collision())
        );
    }

    #[test]
    fn an_admin_key_can_be_created_beside_the_access_key() {
        assert_eq!(
            create_destination_check(Path::new("/k/admin-key"), Path::new("/k/access-key")),
            Ok(())
        );
        assert_eq!(
            create_destination_check(Path::new("/other/access-key"), Path::new("/k/access-key")),
            Ok(())
        );
    }

    #[test]
    fn key_create_asks_about_docker_only_for_web_and_admin_keys_with_no_path() {
        assert!(!create_asks_for_docker(KeyKind::Access, false));
        assert!(create_asks_for_docker(KeyKind::Web, false));
        assert!(create_asks_for_docker(KeyKind::Admin, false));
        for kind in KINDS {
            assert!(!create_asks_for_docker(kind, true));
        }
    }

    #[test]
    fn key_create_refuses_docker_keys_first() {
        assert_eq!(
            create_checks(KeyKind::Admin, true, Some("Bad Name")),
            Err(
                "make or rotate the admin key inside the container: `docker compose exec kkt \
                 kkt key create --admin --force --name <name>`, then restart kkt and rerun \
                 setup with --admin-key to save the host copy"
                    .to_string()
            )
        );
        assert_eq!(
            create_checks(KeyKind::Web, true, Some("sentinel")),
            Err(
                "rotate the web key inside the container: `docker compose exec kkt kkt key \
                 create --web --force`, then restart kkt and rerun setup to refresh the host \
                 copy"
                    .to_string()
            )
        );
    }

    #[test]
    fn key_create_takes_a_name_only_for_the_admin_key() {
        for kind in [KeyKind::Access, KeyKind::Web] {
            assert_eq!(
                create_checks(kind, false, Some("sentinel")),
                Err("--name names the admin agent, so it goes with --admin".to_string())
            );
            assert_eq!(create_checks(kind, false, None), Ok(None));
        }
        assert_eq!(
            create_checks(KeyKind::Admin, false, Some("sentinel")),
            Ok(Some(
                AdminName::new("admin-sentinel", MAX_ADMIN_NAME_CHARS).unwrap()
            ))
        );
        assert_eq!(create_checks(KeyKind::Admin, false, None), Ok(None));
        assert_eq!(
            create_checks(KeyKind::Admin, false, Some("Bad Name")),
            Err(
                "the admin name must be lowercase letters and digits, in words joined by single \
                 hyphens, such as `sentinel` (which becomes `admin-sentinel`); got \"Bad Name\""
                    .to_string()
            )
        );
    }
}
