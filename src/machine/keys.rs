//! Key files: where this machine's access, web and admin keys are, and
//! reading, making and writing them and the admin key's name file.

use crate::machine::record::{read_setup, read_unfinished, read_unrecorded};
use crate::machine::settings::{ServeArgs, Sources, read_config};
use kkt_admission::{Admission, SharedKey};
use kkt_config as config;
use kkt_names::{AdminName, OsEntropy};
use kkt_provenance::Build;
use kkt_settings::{self as settings, AccessKey, Env};
use kkt_setup as setup;
use std::io::Write;
use std::path::{Path, PathBuf};

/// What to say, after `label`, about the name tied to the admin key `key`
/// in the file at `path`, if anything: a Docker setup's host copy has no
/// name beside it, as the name is kept in the container.
pub fn admin_name_line(path: &Path, key: &str, label: &str) -> Result<Option<String>, String> {
    if path.file_name() == Some(std::ffi::OsStr::new(config::DOCKER_ADMIN_KEY_FILE_NAME)) {
        return Ok(None);
    }
    let make = "`kkt key create --admin --force --name <name>` makes a named one";
    Ok(Some(match read_admin_name(path, key)? {
        AdminNameFile::Named(name) => format!("{label}: {name}"),
        AdminNameFile::None => format!("{label}: none, so the key may only read; {make}"),
        AdminNameFile::OtherKey(names) => format!(
            "{label}: none, so the key may only read: {} names a different key; {make}",
            names.display()
        ),
    }))
}

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
    pub(crate) fn name(self) -> &'static str {
        match self {
            KeyKind::Access => "access key",
            KeyKind::Web => "web key",
            KeyKind::Admin => "admin key",
        }
    }

    /// The `kkt key` flag that selects it.
    pub(crate) fn flag(self) -> &'static str {
        match self {
            KeyKind::Access => "",
            KeyKind::Web => " --web",
            KeyKind::Admin => " --admin",
        }
    }

    /// The server's file for this key, given its access key's file.
    pub(crate) fn beside(self, access: &Path) -> Option<PathBuf> {
        match self {
            KeyKind::Access => Some(access.to_path_buf()),
            KeyKind::Web => Some(settings::web_key_path(access)),
            KeyKind::Admin => settings::admin_key_path(access),
        }
    }

    /// A Docker setup's host copy of this key, given the access key's copy.
    pub(crate) fn docker_copy(self, access: &Path) -> PathBuf {
        match self {
            KeyKind::Access => access.to_path_buf(),
            KeyKind::Web => access.with_file_name(config::DOCKER_WEB_KEY_FILE_NAME),
            KeyKind::Admin => access.with_file_name(config::DOCKER_ADMIN_KEY_FILE_NAME),
        }
    }
}

/// Resolve a key without reading its contents or creating a file. An explicit
/// path names the selected key itself; otherwise use this machine's setup.
pub(crate) fn selected_key_path(
    env: &Env,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<PathBuf, String> {
    optional_key_path(env, serve_args, path, kind)?.ok_or_else(|| "there is no admin key".into())
}

/// None means the access key occupies the implicit admin key's place.
pub(crate) fn optional_key_path(
    env: &Env,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = path {
        return Ok(Some(path));
    }
    let access = match read_setup(env)? {
        Some(record) => {
            if kind != KeyKind::Access && matches!(record.mode, setup::Mode::Client) {
                return Err(format!(
                    "the {} is on the server's machine; run `kkt key show{}` there",
                    kind.name(),
                    kind.flag()
                ));
            }
            let access = record.key_file.ok_or("keys are disabled for this setup")?;
            if matches!(record.mode, setup::Mode::Docker(_)) {
                return Ok(Some(kind.docker_copy(&access)));
            }
            access
        }
        None => {
            // A Docker setup that has not finished, or a server kkt installed
            // with no record, keeps its keys' copies where a finished one
            // does: where `kkt status` says they are.
            if kind != KeyKind::Access {
                let found = match read_unfinished(env)? {
                    Some(unfinished) => local_operator_key(env, Some(&unfinished), None, kind)?,
                    None => match unrecorded_for_keys(env, serve_args) {
                        Some(unrecorded) => local_operator_key(env, None, Some(&unrecorded), kind)?,
                        None => None,
                    },
                };
                if let Some(path) = found {
                    return Ok(Some(path));
                }
            }
            let (_, options) = read_config(env, None)?;
            let settings = options
                .resolve(PathBuf::new(), env.access_key_file())
                .map_err(|e| e.to_string())?;
            match settings.access_key {
                AccessKey::None => return Err("keys are disabled in the serve config".into()),
                AccessKey::File(path) | AccessKey::Default(path) => path,
            }
        }
    };
    Ok(kind.beside(&access))
}

/// An access key may keep its old name, but cannot also be the admin key.
pub fn admin_key_collision() -> String {
    "cannot add an admin key: access_key_file is named admin-key, which is also the admin key's place in that folder; move one of the files to a different folder, or rename the access key and update access_key_file".into()
}

/// A server kkt installed here with no record, for finding its keys. One
/// that cannot be made out is said on standard error, as `kkt status` says
/// it, and the keys are then looked for as the serve config says.
pub(crate) fn unrecorded_for_keys(env: &Env, serve_args: ServeArgs) -> Option<setup::Unrecorded> {
    match read_unrecorded(env, serve_args) {
        Ok(found) => found,
        Err((_, e)) => {
            eprintln!("Unrecorded install: {e}");
            None
        }
    }
}

/// Whether this machine's server runs in Docker, as recorded or, with no
/// record and no unfinished setup, as found ([`unrecorded_for_keys`]).
pub(crate) fn runs_in_docker(env: &Env, serve_args: ServeArgs) -> Result<bool, String> {
    Ok(match read_setup(env)? {
        Some(record) => matches!(record.mode, setup::Mode::Docker(_)),
        None => {
            read_unfinished(env)?.is_none()
                && unrecorded_for_keys(env, serve_args)
                    .is_some_and(|found| matches!(found.mode, setup::Mode::Docker(_)))
        }
    })
}

/// This machine's web key file, if it runs a server. See
/// [`local_operator_key`].
pub fn local_web_key(
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
) -> Result<Option<PathBuf>, String> {
    local_operator_key(env, unfinished, unrecorded, KeyKind::Web)
}

/// Where this machine's web or admin key file is, if it runs a server: for
/// Docker, the host copy `kkt setup` saves, even while that setup is
/// `unfinished` or the server is `unrecorded`; otherwise the file the server
/// reads. The file need not exist.
pub fn local_operator_key(
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    let record = match read_setup(env)? {
        Some(record) => Some(record),
        None => unfinished
            .and_then(|unfinished| unfinished.record.clone())
            .or_else(|| unrecorded.map(|found| found.record(Build::CURRENT.version))),
    };
    if let Some(record) = record {
        if matches!(record.mode, setup::Mode::Client) {
            return Ok(None);
        }
        return Ok(record.key_file.as_deref().and_then(|access| {
            if matches!(record.mode, setup::Mode::Docker(_)) {
                Some(kind.docker_copy(access))
            } else {
                kind.beside(access)
            }
        }));
    }
    if let Some(unfinished) = unfinished
        && unfinished.purpose == setup::Purpose::Docker
    {
        let access = env
            .config_dir()
            .ok_or_else(|| format!("set HOME to find the {}", kind.name()))?
            .join(config::ACCESS_KEY_FILE_NAME);
        return Ok(Some(kind.docker_copy(&access)));
    }
    let (_, options) = read_config(env, None)?;
    let settings = options
        .resolve(PathBuf::new(), env.access_key_file())
        .map_err(|e| e.to_string())?;
    Ok(match settings.access_key {
        AccessKey::None => None,
        AccessKey::File(path) | AccessKey::Default(path) => kind.beside(&path),
    })
}

/// `kkt key create`: writes a new key to a file only the owner can read,
/// and says how to use it. The key itself is printed only with `--show`.
/// Makes the default key file if it is the one in use and does not exist
/// yet, and logs where it is (never the key).
pub fn make_default_key(access_key: &AccessKey, kind: &str) -> Result<(), String> {
    let AccessKey::Default(path) = access_key else {
        return Ok(());
    };
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    write_key_file(path, &SharedKey::generate(&OsEntropy), false)?;
    tracing::info!("created a new {kind} key at {}", path.display());
    Ok(())
}

/// The admin key in the file at `path`, and the file's text, if there is
/// one: with none, `/mcp-admin` is off. It must differ from the keys of
/// `others`.
pub(crate) fn read_admin_key(
    path: &Path,
    others: &[&Admission],
) -> Result<Option<(SharedKey, String)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "invalid setting: admin_key_file could not be read from {}: {e}",
                path.display()
            ));
        }
    };
    let reused = others.iter().any(|other| match other {
        Admission::SharedKey(key) => key.matches(text.trim().as_bytes()),
        Admission::Open => false,
    });
    if reused {
        return Err("invalid setting: admin_key_file must contain a different key from access_key_file and web_key_file".into());
    }
    SharedKey::new(&text, path.display().to_string())
        .map(|key| Some((key, text)))
        .map_err(|e| format!("invalid setting: admin_key_file: {}", e.rule))
}

/// What the name file beside an admin key says about that key.
pub(crate) enum AdminNameFile {
    /// The admin agent's name, tied to this key.
    Named(AdminName),
    /// There is no name file: the key may only read.
    None,
    /// The name file, at this path, is for a different key, as when the key
    /// file was replaced without its name: this key may only read.
    OtherKey(PathBuf),
}

/// The name tied to the admin key `key`, kept in the file at `path`, from
/// the name file beside it.
pub(crate) fn read_admin_name(path: &Path, key: &str) -> Result<AdminNameFile, String> {
    let names = settings::admin_name_path(path);
    match std::fs::read_to_string(&names) {
        Ok(text) => match AdminName::bound_in(&text, key, config::MAX_ADMIN_NAME_CHARS) {
            Ok(Some(name)) => Ok(AdminNameFile::Named(name)),
            Ok(None) => Ok(AdminNameFile::OtherKey(names)),
            Err(e) => Err(format!(
                "invalid setting: admin name in {}: {e}",
                names.display()
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AdminNameFile::None),
        Err(e) => Err(format!(
            "invalid setting: the admin name could not be read from {}: {e}",
            names.display()
        )),
    }
}

/// The access key in the file at `path`.
pub(crate) fn read_key(path: &Path, sources: &Sources) -> Result<SharedKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "invalid setting: access_key_file could not be read from {}: {e}",
            path.display()
        )
    })?;
    SharedKey::new(&text, path.display().to_string()).map_err(|e| sources.invalid(e))
}

/// Ties `name` to the new admin key `key`, just written to the file at
/// `path`, in the name file beside it. The name file holds a hash of the
/// key, so a name never passes to another key: one left from an earlier key
/// names nothing. With no name, such a file is removed. Returns what to tell
/// the person.
pub fn write_admin_name(
    path: &Path,
    key: &str,
    name: Option<&AdminName>,
) -> Result<String, String> {
    let names = settings::admin_name_path(path);
    let unnamed = "It has no name, so it may only read. To let it make changes, make it again with --force --name <name>.";
    match name {
        Some(name) => {
            std::fs::write(&names, name.bound_to(key)).map_err(|e| {
                format!(
                    "the key was written, but its name could not be: could not write {}: {e}. {unnamed}",
                    names.display()
                )
            })?;
            Ok(format!(
                "Its admin agent is named {name}: its changes are announced as \"operator (via {name})\"."
            ))
        }
        None => match std::fs::remove_file(&names) {
            Ok(()) => Ok(unnamed.into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(unnamed.into()),
            // The file names an earlier key, never this one, so it is only
            // clutter.
            Err(e) => Ok(format!(
                "{unnamed} The name file of an earlier key, {}, could not be removed ({e}); it does not name this key.",
                names.display()
            )),
        },
    }
}

/// Makes a new random key and writes it to `path`, creating its folder if
/// needed. Returns the key.
pub fn new_key_file(path: &Path, force: bool) -> Result<String, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let key = SharedKey::generate(&OsEntropy);
    write_key_file(path, &key, force)?;
    Ok(key)
}

/// Writes `key` to `path`, readable and writable only by its owner. Refuses
/// to replace an existing file unless `force`.
pub fn write_key_file(path: &Path, key: &str, force: bool) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(config::ACCESS_KEY_FILE_MODE);
    }
    let mut file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!(
            "{} already exists; pass --force to replace it",
            path.display()
        ),
        _ => format!("could not write {}: {e}", path.display()),
    })?;
    // The mode above applies only to a new file; an existing one replaced
    // with --force keeps its old mode unless it is set here.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(
            config::ACCESS_KEY_FILE_MODE,
        ))
        .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
    }
    writeln!(file, "{key}").map_err(|e| format!("could not write {}: {e}", path.display()))
}
