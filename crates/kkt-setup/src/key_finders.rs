//! Finding this machine's key files: the order of the reads that finding
//! a key makes (the record, the note of an unfinished setup, a server
//! installed with no record, the serve config), each made only when the
//! one before leaves the answer open. The caller supplies the reads.

use crate::KeyKind;
use std::path::PathBuf;

/// The reads that finding an operator key makes. Each is made when the one
/// before it leaves the answer open, so the order is part of the behavior.
pub trait OperatorKeyReads {
    /// This machine's setup record.
    fn setup(&self) -> Result<Option<crate::Record>, String>;
    /// The access key file the serve config names; `None` when it turns
    /// keys off.
    fn configured_access(&self) -> Result<Option<PathBuf>, String>;
    /// kkt's config directory; not a read of any file.
    fn config_dir(&self) -> Option<PathBuf>;
}

/// The reads that finding any key from `kkt key` makes, beyond
/// [`OperatorKeyReads`].
pub trait KeyReads: OperatorKeyReads {
    /// The note of an unfinished setup, if its install is still here.
    fn unfinished(&self) -> Result<Option<crate::Unfinished>, String>;
    /// A server installed with no record, or `None` when there is none or
    /// it cannot be made out (the reader says why, if it says anything).
    fn unrecorded(&self) -> Option<crate::Unrecorded>;
}

/// The key of `kind` that `kkt key` reads or replaces: `path` if given,
/// else as the record says. With no record, the web and admin keys of a
/// Docker setup that has not finished, or of a server kkt installed with no
/// record, are where `kkt status` says they are ([`find_operator_key`],
/// which reads the record again). Otherwise every key is beside the serve
/// config's access key, read once more, and keys turned off there are an
/// error.
pub fn find_key<R: KeyReads + ?Sized>(
    reads: &R,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = path {
        return Ok(Some(path));
    }
    if let Some(record) = reads.setup()? {
        return crate::recorded_key(&record, kind);
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
    crate::configured_key(
        reads.configured_access()?.as_deref(),
        kind,
        crate::KeysOff::Refuse,
    )
}

/// Where this machine's key of `kind` is: reads the record, then, only if
/// it does not say, the serve config.
pub fn find_operator_key<R: OperatorKeyReads + ?Sized>(
    reads: &R,
    unfinished: Option<&crate::Unfinished>,
    unrecorded: Option<&crate::Unrecorded>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    let recorded = reads.setup()?;
    let place = crate::operator_key(
        recorded.as_ref(),
        unfinished,
        unrecorded,
        reads.config_dir().as_deref(),
        kind,
    )?;
    match place {
        crate::KeyPlace::Known(path) => Ok(path),
        crate::KeyPlace::Configured => crate::configured_key(
            reads.configured_access()?.as_deref(),
            kind,
            crate::KeysOff::NoKey,
        ),
    }
}

/// What is known about the server here: its record, else a note of an
/// unfinished setup, else a server installed with no record, each read only
/// if the one before is not there.
pub fn find_here<R: KeyReads + ?Sized>(reads: &R) -> Result<crate::Here, String> {
    if let Some(record) = reads.setup()? {
        return Ok(crate::Here::Recorded(record));
    }
    if let Some(unfinished) = reads.unfinished()? {
        return Ok(crate::Here::Unfinished(unfinished));
    }
    Ok(match reads.unrecorded() {
        Some(found) => crate::Here::Unrecorded(found),
        None => crate::Here::Nothing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as setup;
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
