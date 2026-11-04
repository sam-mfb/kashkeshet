//! Where this machine's access, web and admin key files are, decided from
//! the setup record, the note of an unfinished setup, a server found
//! installed with no record, or the serve config, with the host copies a
//! Docker setup keeps; and what `kkt key create` refuses before it writes.
//! The program reads the record and the rest, and reads and writes the key
//! files.

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

/// What is known about the server on this machine, for finding its keys: in
/// order, its record, the note of an unfinished setup, or a server found
/// installed with no record.
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

/// Where a key file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyPlace {
    /// This file, or `None` when there is no such key here.
    Known(Option<PathBuf>),
    /// Beside the access key the serve config names
    /// ([`KeyKind::beside`]). When the serve config turns keys off, there
    /// is no such key, or, if `off` is given, that is the error.
    Configured { off: Option<String> },
}

/// What `kkt key` says when the serve config turns keys off.
const KEYS_OFF: &str = "keys are disabled in the serve config";

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
    Ok(KeyPlace::Configured { off: None })
}

/// Where the key of `kind` that `kkt key` reads or replaces is, when no path
/// was given, from what is `here` (read as [`KeyKind::looks_past_record`]
/// says). With no record, the web and admin keys are where `kkt status` says
/// they are, if it says; otherwise every key is beside the serve config's
/// access key, and keys turned off there are an error.
pub fn key_place(
    here: &Here,
    config_dir: Option<&Path>,
    kind: KeyKind,
) -> Result<KeyPlace, String> {
    let past_record = match here {
        Here::Recorded(record) => {
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
                return Ok(KeyPlace::Known(Some(kind.docker_copy(access))));
            }
            return Ok(KeyPlace::Known(kind.beside(access)));
        }
        Here::Unfinished(unfinished) if kind.looks_past_record() => {
            operator_key(None, Some(unfinished), None, config_dir, kind)?
        }
        Here::Unrecorded(found) if kind.looks_past_record() => {
            operator_key(None, None, Some(found), config_dir, kind)?
        }
        _ => KeyPlace::Known(None),
    };
    Ok(match past_record {
        KeyPlace::Known(Some(path)) => KeyPlace::Known(Some(path)),
        _ => KeyPlace::Configured {
            off: Some(KEYS_OFF.to_string()),
        },
    })
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

    fn known(path: &str) -> KeyPlace {
        KeyPlace::Known(Some(PathBuf::from(path)))
    }

    fn configured_or_off() -> KeyPlace {
        KeyPlace::Configured {
            off: Some(KEYS_OFF.to_string()),
        }
    }

    fn home() -> Option<&'static Path> {
        Some(Path::new("/home/k/.config/kkt"))
    }

    #[test]
    fn each_key_beside_the_access_key_and_in_docker() {
        let access = Path::new("/keys/access-key");
        assert_eq!(KeyKind::Access.beside(access), Some(access.to_path_buf()));
        assert_eq!(KeyKind::Web.beside(access), Some(web_key_path(access)));
        assert_eq!(KeyKind::Admin.beside(access), admin_key_path(access));
        assert_eq!(KeyKind::Admin.beside(Path::new("/keys/admin-key")), None);
        assert_eq!(KeyKind::Access.docker_copy(access), access);
        assert_eq!(
            KeyKind::Web.docker_copy(access),
            Path::new("/keys").join(DOCKER_WEB_KEY_FILE_NAME)
        );
        assert_eq!(
            KeyKind::Admin.docker_copy(access),
            Path::new("/keys").join(DOCKER_ADMIN_KEY_FILE_NAME)
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
    fn an_operator_key_of_a_recorded_server() {
        let service = record(Mode::Service { tailscale: false }, Some("/keys/access-key"));
        let docker = record(docker_mode(), Some("/keys/docker-access-key"));
        for kind in KINDS {
            assert_eq!(
                operator_key(Some(&service), None, None, home(), kind),
                Ok(KeyPlace::Known(kind.beside(Path::new("/keys/access-key"))))
            );
            assert_eq!(
                operator_key(Some(&docker), None, None, home(), kind),
                Ok(KeyPlace::Known(Some(
                    kind.docker_copy(Path::new("/keys/docker-access-key"))
                )))
            );
            assert_eq!(
                operator_key(
                    Some(&record(Mode::Client, Some("/k"))),
                    None,
                    None,
                    home(),
                    kind
                ),
                Ok(KeyPlace::Known(None))
            );
            assert_eq!(
                operator_key(Some(&record(docker_mode(), None)), None, None, home(), kind),
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
            Ok(KeyPlace::Known(Some(web_key_path(Path::new(
                "/keys/access-key"
            )))))
        );
    }

    #[test]
    fn an_operator_key_of_a_finished_unrecorded_install_uses_its_record() {
        let noted = unfinished(
            Purpose::Docker,
            Some(record(docker_mode(), Some("/d/access"))),
        );
        assert_eq!(
            operator_key(None, Some(&noted), None, None, KeyKind::Admin),
            Ok(KeyPlace::Known(Some(
                Path::new("/d").join(DOCKER_ADMIN_KEY_FILE_NAME)
            )))
        );
    }

    #[test]
    fn an_operator_key_of_an_unfinished_docker_install_is_the_host_copy() {
        let noted = unfinished(Purpose::Docker, None);
        assert_eq!(
            operator_key(None, Some(&noted), None, home(), KeyKind::Web),
            Ok(KeyPlace::Known(Some(
                Path::new("/home/k/.config/kkt").join(DOCKER_WEB_KEY_FILE_NAME)
            )))
        );
        assert_eq!(
            operator_key(None, Some(&noted), None, None, KeyKind::Web),
            Err("set HOME to find the web key".into())
        );
    }

    #[test]
    fn an_operator_key_of_an_unfinished_service_or_nothing_is_the_configs() {
        let noted = unfinished(Purpose::Service, None);
        for kind in KINDS {
            assert_eq!(
                operator_key(None, Some(&noted), None, None, kind),
                Ok(KeyPlace::Configured { off: None })
            );
            assert_eq!(
                operator_key(None, None, None, None, kind),
                Ok(KeyPlace::Configured { off: None })
            );
        }
    }

    #[test]
    fn an_operator_key_of_an_unrecorded_server() {
        let docker = unrecorded(docker_mode(), Some("/d/access"));
        assert_eq!(
            operator_key(None, None, Some(&docker), None, KeyKind::Web),
            Ok(KeyPlace::Known(Some(
                Path::new("/d").join(DOCKER_WEB_KEY_FILE_NAME)
            )))
        );
        let service = unrecorded(Mode::Service { tailscale: false }, None);
        assert_eq!(
            operator_key(None, None, Some(&service), None, KeyKind::Web),
            Ok(KeyPlace::Known(None))
        );
    }

    #[test]
    fn a_key_place_from_the_record() {
        let service = Here::Recorded(record(
            Mode::Service { tailscale: false },
            Some("/k/access"),
        ));
        let docker = Here::Recorded(record(docker_mode(), Some("/d/access")));
        for kind in KINDS {
            assert_eq!(
                key_place(&service, home(), kind),
                Ok(KeyPlace::Known(kind.beside(Path::new("/k/access"))))
            );
            assert_eq!(
                key_place(&docker, home(), kind),
                Ok(KeyPlace::Known(Some(
                    kind.docker_copy(Path::new("/d/access"))
                )))
            );
            assert_eq!(
                key_place(&Here::Recorded(record(docker_mode(), None)), home(), kind),
                Err("keys are disabled for this setup".into())
            );
        }
    }

    #[test]
    fn on_a_client_only_the_access_key_has_a_place() {
        let client = Here::Recorded(record(Mode::Client, Some("/k/client-access-key")));
        assert_eq!(
            key_place(&client, home(), KeyKind::Access),
            Ok(known("/k/client-access-key"))
        );
        assert_eq!(
            key_place(&client, home(), KeyKind::Web),
            Err("the web key is on the server's machine; run `kkt key show --web` there".into())
        );
        assert_eq!(
            key_place(&client, home(), KeyKind::Admin),
            Err(
                "the admin key is on the server's machine; run `kkt key show --admin` there".into()
            )
        );
    }

    #[test]
    fn with_no_record_the_web_and_admin_keys_are_where_status_says() {
        let noted = Here::Unfinished(unfinished(Purpose::Docker, None));
        assert_eq!(
            key_place(&noted, home(), KeyKind::Admin),
            Ok(KeyPlace::Known(Some(
                Path::new("/home/k/.config/kkt").join(DOCKER_ADMIN_KEY_FILE_NAME)
            )))
        );
        assert_eq!(
            key_place(&noted, None, KeyKind::Admin),
            Err("set HOME to find the admin key".into())
        );
        let found = Here::Unrecorded(unrecorded(docker_mode(), Some("/d/access")));
        assert_eq!(
            key_place(&found, None, KeyKind::Web),
            Ok(KeyPlace::Known(Some(
                Path::new("/d").join(DOCKER_WEB_KEY_FILE_NAME)
            )))
        );
    }

    #[test]
    fn with_no_record_the_access_key_is_always_the_configs() {
        for here in [
            Here::Unfinished(unfinished(Purpose::Docker, None)),
            Here::Unrecorded(unrecorded(docker_mode(), Some("/d/access"))),
            Here::Nothing,
        ] {
            assert_eq!(
                key_place(&here, home(), KeyKind::Access),
                Ok(configured_or_off())
            );
        }
    }

    #[test]
    fn with_nowhere_said_a_key_is_the_configs_and_keys_off_is_an_error() {
        for kind in [KeyKind::Web, KeyKind::Admin] {
            for here in [
                Here::Unfinished(unfinished(Purpose::Service, None)),
                Here::Unfinished(unfinished(
                    Purpose::Service,
                    Some(record(Mode::Client, None)),
                )),
                Here::Unrecorded(unrecorded(Mode::Service { tailscale: false }, None)),
                Here::Nothing,
            ] {
                assert_eq!(key_place(&here, home(), kind), Ok(configured_or_off()));
            }
        }
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
        assert!(
            create_checks(KeyKind::Admin, true, Some("Bad Name"))
                .unwrap_err()
                .starts_with("make or rotate the admin key inside the container")
        );
        assert!(
            create_checks(KeyKind::Web, true, Some("sentinel"))
                .unwrap_err()
                .starts_with("rotate the web key inside the container")
        );
    }

    #[test]
    fn key_create_takes_a_name_only_for_the_admin_key() {
        for kind in [KeyKind::Access, KeyKind::Web] {
            assert_eq!(
                create_checks(kind, false, Some("sentinel")),
                Err("--name names the admin agent, so it goes with --admin".into())
            );
            assert_eq!(create_checks(kind, false, None), Ok(None));
        }
        assert_eq!(
            create_checks(KeyKind::Admin, false, Some("sentinel")),
            Ok(Some(
                AdminName::new("sentinel", MAX_ADMIN_NAME_CHARS).unwrap()
            ))
        );
        assert_eq!(create_checks(KeyKind::Admin, false, None), Ok(None));
        assert!(
            create_checks(KeyKind::Admin, false, Some("Bad Name"))
                .unwrap_err()
                .contains("the admin name must be")
        );
    }
}
