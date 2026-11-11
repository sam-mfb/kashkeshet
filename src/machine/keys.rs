//! Key files: where this machine's access, web and admin keys are, and
//! reading, making and writing them and the admin key's name file.

use crate::machine::record::{read_setup, read_unfinished, read_unrecorded};
use crate::machine::settings::{DefaultDb, Report, ServeArgs, Sources, read_config, resolve};
use kkt_admission::server_keys::{self, KeyRefusal, KeyText, KeyTexts, ServerKeys};
use kkt_admission::{ServerKey, SharedKey};
use kkt_config as config;
use kkt_names::{AdminName, OsEntropy};
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

pub use kkt_setup::{KeyKind, admin_key_collision};

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
    find_key(
        &InstallReads {
            env: EnvReads(env),
            serve_args,
        },
        path,
        kind,
    )
}

/// The reads that finding an operator key makes. Each is made when the one
/// before it leaves the answer open, so the order is part of the behavior.
trait OperatorKeyReads {
    /// This machine's setup record.
    fn setup(&self) -> Result<Option<setup::Record>, String>;
    /// The access key file the serve config names; `None` when it turns
    /// keys off.
    fn configured_access(&self) -> Result<Option<PathBuf>, String>;
    /// kkt's config directory; not a read of any file.
    fn config_dir(&self) -> Option<PathBuf>;
}

/// The reads that finding any key from `kkt key` makes, beyond
/// [`OperatorKeyReads`].
trait KeyReads: OperatorKeyReads {
    /// The note of an unfinished setup, if its install is still here.
    fn unfinished(&self) -> Result<Option<setup::Unfinished>, String>;
    /// A server installed with no record ([`unrecorded_for_keys`]).
    fn unrecorded(&self) -> Option<setup::Unrecorded>;
}

/// [`OperatorKeyReads`] on this machine.
struct EnvReads<'a>(&'a Env);

impl OperatorKeyReads for EnvReads<'_> {
    fn setup(&self) -> Result<Option<setup::Record>, String> {
        read_setup(self.0)
    }

    fn configured_access(&self) -> Result<Option<PathBuf>, String> {
        let (_, options) = read_config(self.0, None)?;
        let settings = resolve(self.0, options, DefaultDb::OrEmpty, Report::Bare)?;
        Ok(match settings.access_key {
            AccessKey::None => None,
            AccessKey::File(path) | AccessKey::Default(path) => Some(path),
        })
    }

    fn config_dir(&self) -> Option<PathBuf> {
        self.0.config_dir()
    }
}

/// [`KeyReads`] on this machine.
struct InstallReads<'a> {
    env: EnvReads<'a>,
    serve_args: ServeArgs,
}

impl OperatorKeyReads for InstallReads<'_> {
    fn setup(&self) -> Result<Option<setup::Record>, String> {
        self.env.setup()
    }

    fn configured_access(&self) -> Result<Option<PathBuf>, String> {
        self.env.configured_access()
    }

    fn config_dir(&self) -> Option<PathBuf> {
        self.env.config_dir()
    }
}

impl KeyReads for InstallReads<'_> {
    fn unfinished(&self) -> Result<Option<setup::Unfinished>, String> {
        read_unfinished(self.env.0)
    }

    fn unrecorded(&self) -> Option<setup::Unrecorded> {
        unrecorded_for_keys(self.env.0, self.serve_args)
    }
}

/// The key of `kind` that `kkt key` reads or replaces: `path` if given,
/// else as the record says. With no record, the web and admin keys of a
/// Docker setup that has not finished, or of a server kkt installed with no
/// record, are where `kkt status` says they are ([`find_operator_key`],
/// which reads the record again). Otherwise every key is beside the serve
/// config's access key, read once more, and keys turned off there are an
/// error.
fn find_key<R: KeyReads + ?Sized>(
    reads: &R,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = path {
        return Ok(Some(path));
    }
    if let Some(record) = reads.setup()? {
        return setup::recorded_key(&record, kind);
    }
    if kind.looks_past_record() {
        let found = match reads.unfinished()? {
            Some(unfinished) => find_operator_key(reads, Some(&unfinished), None, kind)?,
            None => match reads.unrecorded() {
                Some(unrecorded) => find_operator_key(reads, None, Some(&unrecorded), kind)?,
                None => None,
            },
        };
        if let Some(path) = found {
            return Ok(Some(path));
        }
    }
    setup::configured_key(
        reads.configured_access()?.as_deref(),
        kind,
        setup::KeysOff::Refuse,
    )
}

/// Where this machine's key of `kind` is ([`local_operator_key`]): reads
/// the record, then, only if it does not say, the serve config.
fn find_operator_key<R: OperatorKeyReads + ?Sized>(
    reads: &R,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    let recorded = reads.setup()?;
    let place = setup::operator_key(
        recorded.as_ref(),
        unfinished,
        unrecorded,
        reads.config_dir().as_deref(),
        kind,
    )?;
    match place {
        setup::KeyPlace::Known(path) => Ok(path),
        setup::KeyPlace::Configured => setup::configured_key(
            reads.configured_access()?.as_deref(),
            kind,
            setup::KeysOff::NoKey,
        ),
    }
}

/// What is known about the server here: its record, else a note of an
/// unfinished setup, else a server installed with no record, each read only
/// if the one before is not there.
fn find_here<R: KeyReads + ?Sized>(reads: &R) -> Result<setup::Here, String> {
    if let Some(record) = reads.setup()? {
        return Ok(setup::Here::Recorded(record));
    }
    if let Some(unfinished) = reads.unfinished()? {
        return Ok(setup::Here::Unfinished(unfinished));
    }
    Ok(match reads.unrecorded() {
        Some(found) => setup::Here::Unrecorded(found),
        None => setup::Here::Nothing,
    })
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
    let reads = InstallReads {
        env: EnvReads(env),
        serve_args,
    };
    Ok(find_here(&reads)?.runs_in_docker())
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
    find_operator_key(&EnvReads(env), unfinished, unrecorded, kind)
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
    let mut files = ServerKeyFiles { access, web, admin };
    // A name file is read only beside an admin key, so with none the name
    // refusal cannot happen; the path is then empty.
    let names = admin.map(settings::admin_name_path).unwrap_or_default();
    server_keys::check(&mut files).map_err(|refusal| match refusal {
        KeyRefusal::Text(e) => e,
        KeyRefusal::AccessKey(e) => sources.invalid(e),
        KeyRefusal::WebKeyReused => "invalid setting: web_key_file must contain a different key from access_key_file".into(),
        KeyRefusal::WebKey(e) => format!("invalid setting: web_key_file: {}", e.rule),
        KeyRefusal::AdminKeyReused => "invalid setting: admin_key_file must contain a different key from access_key_file and web_key_file".into(),
        KeyRefusal::AdminKey(e) => format!("invalid setting: admin_key_file: {}", e.rule),
        KeyRefusal::AdminName(e) => {
            format!("invalid setting: admin name in {}: {e}", names.display())
        }
    })
}

/// [`KeyTexts`] from the server's key files.
struct ServerKeyFiles<'a> {
    access: &'a AccessKey,
    web: &'a AccessKey,
    admin: Option<&'a Path>,
}

impl ServerKeyFiles<'_> {
    /// The key file `key` names, if any; it must be readable.
    fn read(key: &AccessKey, setting: &str) -> Result<Option<KeyText>, String> {
        match key {
            AccessKey::None => Ok(None),
            AccessKey::File(path) | AccessKey::Default(path) => {
                read_key_text(path, setting).map(|text| {
                    Some(KeyText {
                        text,
                        source: path.display().to_string(),
                    })
                })
            }
        }
    }
}

impl KeyTexts for ServerKeyFiles<'_> {
    type Error = String;

    // Only reads the key: `serve` makes a missing default key first, and
    // `kkt service install` checks settings without making anything.
    fn access(&mut self) -> Result<Option<KeyText>, String> {
        Self::read(self.access, "access_key_file")
    }

    fn web(&mut self) -> Result<Option<KeyText>, String> {
        Self::read(self.web, "web_key_file")
    }

    fn admin(&mut self) -> Result<Option<KeyText>, String> {
        let Some(path) = self.admin else {
            return Ok(None);
        };
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(KeyText {
                text,
                source: path.display().to_string(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!(
                "invalid setting: admin_key_file could not be read from {}: {e}",
                path.display()
            )),
        }
    }

    fn admin_names(&mut self) -> Result<Option<String>, String> {
        match self.admin {
            Some(path) => read_admin_names(&settings::admin_name_path(path)),
            None => Ok(None),
        }
    }
}

/// The text of the admin name file at `names`; `None` if there is none.
fn read_admin_names(names: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(names) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!(
            "invalid setting: the admin name could not be read from {}: {e}",
            names.display()
        )),
    }
}

/// The text of the key file at `path`, which `setting` names.
fn read_key_text(path: &Path, setting: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| {
        format!(
            "invalid setting: {setting} could not be read from {}: {e}",
            path.display()
        )
    })
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
    match read_admin_names(&names)? {
        Some(text) => match AdminName::bound_in(&text, key, kkt_names::MAX_ADMIN_NAME_CHARS) {
            Ok(Some(name)) => Ok(AdminNameFile::Named(name)),
            Ok(None) => Ok(AdminNameFile::OtherKey(names)),
            Err(e) => Err(format!(
                "invalid setting: admin name in {}: {e}",
                names.display()
            )),
        },
        None => Ok(AdminNameFile::None),
    }
}

/// The access key in the file at `path`.
pub(crate) fn read_key(path: &Path, sources: &Sources) -> Result<SharedKey, String> {
    let text = read_key_text(path, "access_key_file")?;
    SharedKey::new(ServerKey::Access, &text, path.display().to_string())
        .map_err(|e| sources.invalid(e))
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
        options.mode(kkt_admission::ACCESS_KEY_FILE_MODE);
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
            kkt_admission::ACCESS_KEY_FILE_MODE,
        ))
        .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
    }
    writeln!(file, "{key}").map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{test_env, write_default_config};
    use crate::scratch;
    use kkt_settings::ServeOptions;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    fn unused_args(_: &[String]) -> Result<(Option<PathBuf>, ServeOptions), String> {
        Err("not used".into())
    }

    #[test]
    fn the_key_finders_use_the_default_key_and_no_database() {
        let home = scratch::Dir::new("keys-default");
        let env = test_env(Some(&home));
        let access = env.access_key_file().expect("key path");
        assert_eq!(
            optional_key_path(&env, unused_args, None, KeyKind::Access),
            Ok(Some(access.clone()))
        );
        assert_eq!(
            local_operator_key(&env, None, None, KeyKind::Web),
            Ok(Some(settings::web_key_path(&access)))
        );
    }

    #[test]
    fn the_key_finders_say_a_bad_setting_without_naming_its_file() {
        let home = scratch::Dir::new("keys-bad-setting");
        let env = test_env(Some(&home));
        write_default_config(&home, "no_access_key = true\naccess_key_file = \"/keys/k\"");
        let said = "no_access_key cannot be set together with access_key_file".to_string();
        assert_eq!(
            optional_key_path(&env, unused_args, None, KeyKind::Access),
            Err(said.clone())
        );
        assert_eq!(
            local_operator_key(&env, None, None, KeyKind::Web),
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
            optional_key_path(&env, unused_args, None, KeyKind::Access),
            Err(said.clone())
        );
        assert_eq!(
            local_operator_key(&env, None, None, KeyKind::Admin),
            Err(said)
        );
    }

    #[test]
    fn the_key_finders_see_keys_turned_off() {
        let home = scratch::Dir::new("keys-off");
        let env = test_env(Some(&home));
        write_default_config(&home, "no_access_key = true");
        assert_eq!(
            optional_key_path(&env, unused_args, None, KeyKind::Access),
            Err("keys are disabled in the serve config".to_string())
        );
        assert_eq!(local_operator_key(&env, None, None, KeyKind::Web), Ok(None));
    }

    /// Reads that answer from a script and log each read, to pin which
    /// reads finding a key makes, and in what order.
    struct Scripted {
        /// One answer per read of the setup record, in order.
        setups: RefCell<VecDeque<Option<setup::Record>>>,
        unfinished: Option<setup::Unfinished>,
        unrecorded: Option<setup::Unrecorded>,
        /// The serve config's access key file.
        access: Option<PathBuf>,
        log: RefCell<Vec<&'static str>>,
    }

    impl Scripted {
        fn new(setups: Vec<Option<setup::Record>>) -> Scripted {
            Scripted {
                setups: RefCell::new(setups.into()),
                unfinished: None,
                unrecorded: None,
                access: Some(PathBuf::from("/k/access-key")),
                log: RefCell::new(Vec::new()),
            }
        }

        fn reads(&self) -> Vec<&'static str> {
            self.log.borrow().clone()
        }
    }

    impl OperatorKeyReads for Scripted {
        fn setup(&self) -> Result<Option<setup::Record>, String> {
            self.log.borrow_mut().push("setup");
            Ok(self
                .setups
                .borrow_mut()
                .pop_front()
                .expect("a scripted setup record"))
        }

        fn configured_access(&self) -> Result<Option<PathBuf>, String> {
            self.log.borrow_mut().push("config");
            Ok(self.access.clone())
        }

        fn config_dir(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/k/.config/kkt"))
        }
    }

    impl KeyReads for Scripted {
        fn unfinished(&self) -> Result<Option<setup::Unfinished>, String> {
            self.log.borrow_mut().push("unfinished");
            Ok(self.unfinished.clone())
        }

        fn unrecorded(&self) -> Option<setup::Unrecorded> {
            self.log.borrow_mut().push("unrecorded");
            self.unrecorded.clone()
        }
    }

    fn service_record(key_file: &str) -> setup::Record {
        setup::Record {
            mode: setup::Mode::Service { tailscale: false },
            written_by: "0.5.0".into(),
            url: "http://127.0.0.1:7880/mcp".into(),
            key_file: Some(PathBuf::from(key_file)),
            clients: None,
        }
    }

    fn unfinished_install(purpose: setup::Purpose) -> setup::Unfinished {
        setup::Unfinished {
            purpose,
            tailscale: false,
            allow_hosts: Vec::new(),
            clients: Vec::new(),
            tailscale_login: None,
            record: None,
        }
    }

    fn unrecorded_service(key_file: Option<&str>) -> setup::Unrecorded {
        setup::Unrecorded {
            mode: setup::Mode::Service { tailscale: false },
            url: "http://127.0.0.1:7880/mcp".into(),
            key_file: key_file.map(PathBuf::from),
        }
    }

    #[test]
    fn a_given_path_reads_nothing() {
        let reads = Scripted::new(Vec::new());
        assert_eq!(
            find_key(&reads, Some(PathBuf::from("/given")), KeyKind::Admin),
            Ok(Some(PathBuf::from("/given")))
        );
        assert!(reads.reads().is_empty());
    }

    #[test]
    fn a_recorded_key_reads_only_the_record() {
        let reads = Scripted::new(vec![Some(service_record("/r/access-key"))]);
        assert_eq!(
            find_key(&reads, None, KeyKind::Web),
            Ok(Some(PathBuf::from("/r/web-key")))
        );
        assert_eq!(reads.reads(), ["setup"]);
    }

    #[test]
    fn with_no_record_the_access_key_reads_only_the_config() {
        let mut reads = Scripted::new(vec![None]);
        reads.unfinished = Some(unfinished_install(setup::Purpose::Docker));
        assert_eq!(
            find_key(&reads, None, KeyKind::Access),
            Ok(Some(PathBuf::from("/k/access-key")))
        );
        assert_eq!(reads.reads(), ["setup", "config"]);
    }

    #[test]
    fn an_unfinished_docker_setup_reads_the_record_again_and_not_the_config() {
        let mut reads = Scripted::new(vec![None, None]);
        reads.unfinished = Some(unfinished_install(setup::Purpose::Docker));
        assert_eq!(
            find_key(&reads, None, KeyKind::Web),
            Ok(Some(PathBuf::from("/home/k/.config/kkt/docker-web-key")))
        );
        assert_eq!(reads.reads(), ["setup", "unfinished", "setup"]);
    }

    #[test]
    fn a_record_made_while_looking_for_an_install_is_taken() {
        let mut reads = Scripted::new(vec![None, Some(service_record("/new/access-key"))]);
        reads.unfinished = Some(unfinished_install(setup::Purpose::Service));
        assert_eq!(
            find_key(&reads, None, KeyKind::Web),
            Ok(Some(PathBuf::from("/new/web-key")))
        );
        assert_eq!(reads.reads(), ["setup", "unfinished", "setup"]);
    }

    #[test]
    fn keys_off_with_an_unfinished_service_read_the_config_twice() {
        let mut reads = Scripted::new(vec![None, None]);
        reads.unfinished = Some(unfinished_install(setup::Purpose::Service));
        reads.access = None;
        assert_eq!(
            find_key(&reads, None, KeyKind::Web),
            Err("keys are disabled in the serve config".to_string())
        );
        assert_eq!(
            reads.reads(),
            ["setup", "unfinished", "setup", "config", "config"]
        );
    }

    #[test]
    fn an_implicit_admin_key_collision_reads_the_config_twice() {
        let mut reads = Scripted::new(vec![None, None]);
        reads.unfinished = Some(unfinished_install(setup::Purpose::Service));
        reads.access = Some(PathBuf::from("/k/admin-key"));
        assert_eq!(find_key(&reads, None, KeyKind::Admin), Ok(None));
        assert_eq!(
            reads.reads(),
            ["setup", "unfinished", "setup", "config", "config"]
        );
    }

    #[test]
    fn an_unrecorded_service_with_no_key_reads_the_config_once() {
        let mut reads = Scripted::new(vec![None, None]);
        reads.unrecorded = Some(unrecorded_service(None));
        assert_eq!(
            find_key(&reads, None, KeyKind::Web),
            Ok(Some(PathBuf::from("/k/web-key")))
        );
        assert_eq!(
            reads.reads(),
            ["setup", "unfinished", "unrecorded", "setup", "config"]
        );
    }

    #[test]
    fn an_unrecorded_service_with_a_key_reads_no_config() {
        let mut reads = Scripted::new(vec![None, None]);
        reads.unrecorded = Some(unrecorded_service(Some("/u/access-key")));
        assert_eq!(
            find_key(&reads, None, KeyKind::Admin),
            Ok(Some(PathBuf::from("/u/admin-key")))
        );
        assert_eq!(
            reads.reads(),
            ["setup", "unfinished", "unrecorded", "setup"]
        );
    }

    #[test]
    fn with_nothing_found_a_web_key_reads_the_config_once() {
        let reads = Scripted::new(vec![None]);
        assert_eq!(
            find_key(&reads, None, KeyKind::Web),
            Ok(Some(PathBuf::from("/k/web-key")))
        );
        assert_eq!(
            reads.reads(),
            ["setup", "unfinished", "unrecorded", "config"]
        );
    }

    #[test]
    fn an_operator_key_reads_the_record_then_the_config() {
        let mut reads = Scripted::new(vec![None]);
        reads.access = None;
        assert_eq!(
            find_operator_key(&reads, None, None, KeyKind::Web),
            Ok(None)
        );
        assert_eq!(reads.reads(), ["setup", "config"]);
    }

    #[test]
    fn whether_it_runs_in_docker_stops_at_the_first_answer() {
        let mut reads = Scripted::new(vec![None]);
        reads.unfinished = Some(unfinished_install(setup::Purpose::Docker));
        assert_eq!(
            find_here(&reads).map(|here| here.runs_in_docker()),
            Ok(false)
        );
        assert_eq!(reads.reads(), ["setup", "unfinished"]);
    }
}
