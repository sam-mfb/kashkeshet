//! Key files: where this machine's access, web and admin keys are, and
//! reading, making and writing them and the admin key's name file.

use crate::record::{read_setup, read_unfinished, read_unrecorded};
use kkt_machine::{Env, Platform};
use kkt_names::AdminName;
use kkt_setup as setup;
use kkt_setup::connection::ConnectionSource;
use std::path::{Path, PathBuf};

pub fn admin_name_line(
    files: &dyn crate::KeyFiles,
    path: &Path,
    key: &str,
    label: &str,
) -> Result<Option<String>, String> {
    let docker_host_copy = path.file_name()
        == Some(std::ffi::OsStr::new(
            kkt_container::DOCKER_ADMIN_KEY_FILE_NAME,
        ));
    kkt_admission::server_keys::admin_name_line(path, key, label, docker_host_copy, |names| {
        files.read_optional(names).map_err(|e| {
            format!(
                "invalid setting: the admin name could not be read from {}: {e}",
                names.display()
            )
        })
    })
}

pub use kkt_setup::{KeyKind, admin_key_collision};

/// Resolve a key without reading its contents or creating a file. An explicit
/// path names the selected key itself; otherwise use this machine's setup.
pub fn selected_key_path(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<PathBuf, String> {
    optional_key_path(parts, env, platform, path, kind)?
        .ok_or_else(|| "there is no admin key".into())
}

/// None means the access key occupies the implicit admin key's place.
pub fn optional_key_path(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    find_key(
        &InstallReads {
            env: EnvReads(env, parts),
            platform,
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
struct EnvReads<'a, 'p, C: ConnectionSource>(&'a Env, &'a crate::OperateParts<'p, C>);

impl<C: ConnectionSource> OperatorKeyReads for EnvReads<'_, '_, C> {
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

struct InstallReads<'a, 'p, C: ConnectionSource> {
    env: EnvReads<'a, 'p, C>,
    platform: Platform,
}
impl<C: ConnectionSource> OperatorKeyReads for InstallReads<'_, '_, C> {
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
impl<C: ConnectionSource> KeyReads for InstallReads<'_, '_, C> {
    fn unfinished(&self) -> Result<Option<setup::Unfinished>, String> {
        let parts = self.env.1;
        read_unfinished(
            self.env.0,
            parts.setup_files,
            parts.container,
            parts.service,
        )
    }
    fn unrecorded(&self) -> Option<setup::Unrecorded> {
        unrecorded_for_keys(self.env.1, self.env.0, self.platform)
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
fn unrecorded_for_keys(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
) -> Option<setup::Unrecorded> {
    match read_unrecorded(parts, env, platform) {
        Ok(found) => found,
        Err((_, e)) => {
            parts
                .output
                .errors(format_args!("Unrecorded install: {e}\n"));
            None
        }
    }
}

/// Whether this machine's server runs in Docker, as recorded or, with no
/// record and no unfinished setup, as found ([`unrecorded_for_keys`]).
pub(crate) fn runs_in_docker(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
) -> Result<bool, String> {
    let reads = InstallReads {
        env: EnvReads(env, parts),
        platform,
    };
    Ok(find_here(&reads)?.runs_in_docker())
}

/// This machine's web key file, if it runs a server. See
/// [`local_operator_key`].
pub fn local_web_key(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
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
pub fn local_operator_key(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    unfinished: Option<&setup::Unfinished>,
    unrecorded: Option<&setup::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    find_operator_key(&EnvReads(env, parts), unfinished, unrecorded, kind)
}

/// Ties `name` to the new admin key `key`, just written to the file at
/// `path`, in the name file beside it. The name file holds a hash of the
/// key, so a name never passes to another key: one left from an earlier key
/// names nothing. With no name, such a file is removed. Returns what to tell
/// the person.
pub fn write_admin_name(
    files: &dyn crate::KeyFiles,
    path: &Path,
    key: &str,
    name: Option<&AdminName>,
) -> Result<String, String> {
    let bound = name.map(|name| kkt_admission::server_keys::bound_to(name, key));
    let written = files.write_admin_name(path, bound.as_deref());
    kkt_admission::server_keys::admin_name_written(path, name, written)
}

/// Makes a new random key and writes it to `path`, creating its folder if
/// needed. Returns the key.
pub fn new_key_file(
    files: &dyn crate::KeyFiles,
    path: &Path,
    force: bool,
) -> Result<String, String> {
    files.new_key(path, force)
}

pub fn write_key_file(
    files: &dyn crate::KeyFiles,
    path: &Path,
    key: &str,
    force: bool,
) -> Result<(), String> {
    files.write_key(path, key, force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

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
