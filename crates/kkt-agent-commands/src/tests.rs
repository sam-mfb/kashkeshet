//! The commands over stand-in parts: how a client's address and key are
//! worked out and worded, and what a refusal says.

use super::*;
use crate::agent::lane_refused;
use futures::executor::block_on;
use futures::future::BoxFuture;
use kkt_admission::AccessKey;
use kkt_agent_client::{Address, Avatars, Failure, Reclaimed, Registered, Servers, Watched};
use kkt_setup::connection::{ConnectionOptions, ServeConnection};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::rc::Rc;

/// No setup record; the serve settings name `key`, if any.
struct NoRecord {
    key: Option<PathBuf>,
}

impl ConnectionSource for NoRecord {
    type KeySource = ();
    fn record(&self) -> Result<Option<kkt_setup::Record>, String> {
        Ok(None)
    }
    fn serve(&self, flags: ConnectionOptions) -> Result<ServeConnection<()>, String> {
        Ok(ServeConnection {
            url: flags
                .url
                .unwrap_or_else(|| "http://127.0.0.1:7420/mcp".into()),
            access_key: match flags.key_file.or(self.key.clone()) {
                Some(path) => AccessKey::File(path),
                None => AccessKey::None,
            },
            key_source: (),
        })
    }
    fn make_default_key(&self, _: &AccessKey) -> Result<(), String> {
        Ok(())
    }
    fn check_key(&self, _: &Path, _: &()) -> Result<(), String> {
        Ok(())
    }
    fn absolute(&self, path: &Path) -> Result<PathBuf, String> {
        Ok(path.to_path_buf())
    }
}

struct AdminAt(PathBuf);
impl AdminKey for AdminAt {
    fn path(&self, given: Option<PathBuf>) -> Result<PathBuf, String> {
        Ok(given.unwrap_or_else(|| self.0.clone()))
    }
}

/// Key files by path; any other is missing.
struct Keys(BTreeMap<PathBuf, String>);
impl KeyText for Keys {
    fn read(&self, path: &Path) -> Result<String, KeyUnread> {
        self.0.get(path).cloned().ok_or(KeyUnread {
            missing: true,
            reason: "No such file or directory (os error 2)".into(),
        })
    }
}

/// What the stand-ins saw: the addresses asked for and the secrets saved.
#[derive(Default)]
struct Seen {
    addresses: Vec<Address>,
    saved: Vec<(PathBuf, String)>,
    output: String,
}

/// A server that answers every avatar call with `answer`.
struct Answering {
    seen: Rc<RefCell<Seen>>,
    answer: fn() -> Result<Registered, Failure>,
}
impl Servers for Answering {
    fn avatars(&self, at: Address) -> Box<dyn Avatars> {
        self.seen.borrow_mut().addresses.push(at);
        Box::new(Answer(self.answer))
    }
    fn agent(&self, _: Address, _: String) -> Box<dyn Watched> {
        unreachable!("no test watches")
    }
    fn admin(&self, _: Address, _: Arc<dyn Clock>) -> Box<dyn Watched> {
        unreachable!("no test watches")
    }
}
struct Answer(fn() -> Result<Registered, Failure>);
impl Avatars for Answer {
    fn register<'a>(
        &'a self,
        _: Option<&'a str>,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Registered, Failure>> {
        Box::pin(async move { (self.0)() })
    }
    fn reclaim<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<Reclaimed, Failure>> {
        Box::pin(async move {
            (self.0)().map(|r| Reclaimed {
                handle: r.handle,
                token: r.token,
            })
        })
    }
    fn describe<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async { Ok(()) })
    }
}

struct Files(Rc<RefCell<Seen>>);
impl Credentials for Files {
    fn reserve(&self, path: PathBuf) -> Result<Box<dyn CredentialSlot>, String> {
        Ok(Box::new(Slot(path, self.0.clone())))
    }
    fn token(&self, _: Option<&Path>) -> Result<String, String> {
        Ok("recovery".into())
    }
    fn claim_state(&self, _: PathBuf) -> Result<Box<dyn kkt_watch::Place>, String> {
        unreachable!("no test watches")
    }
}
struct Slot(PathBuf, Rc<RefCell<Seen>>);
impl CredentialSlot for Slot {
    fn save(&mut self, secret: &str) -> Result<(), String> {
        self.1
            .borrow_mut()
            .saved
            .push((self.0.clone(), secret.into()));
        Ok(())
    }
}

struct NoClients;
impl kkt_client_config::Host for NoClients {
    fn read(&self, _: &Path) -> io::Result<String> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn exists(&self, _: &Path) -> bool {
        false
    }
    fn create_private(&self, _: &Path, _: &str) -> io::Result<()> {
        unreachable!("no test writes a client config")
    }
    fn restrict(&self, _: &Path) -> io::Result<()> {
        unreachable!("no test writes a client config")
    }
    fn create_dir_all(&self, _: &Path) -> io::Result<()> {
        unreachable!("no test writes a client config")
    }
    fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
        unreachable!("no test writes a client config")
    }
    fn remove_file(&self, _: &Path) -> io::Result<()> {
        unreachable!("no test writes a client config")
    }
    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
    fn process_id(&self) -> u32 {
        1
    }
}

struct Kkt;
impl Program for Kkt {
    fn kkt_path(&self) -> Result<PathBuf, String> {
        Ok("/usr/bin/kkt".into())
    }
}

struct Lines;
impl kkt_watch::Report for Lines {
    fn message(&mut self, _: &str) -> Result<(), String> {
        unreachable!("no test watches")
    }
    fn note(&mut self, _: &str) {}
}

struct Out(Rc<RefCell<Seen>>);
impl Output for Out {
    fn output(&self, text: &str) {
        self.0.borrow_mut().output.push_str(text);
    }
}

/// Parts whose serve settings name the access key `access` (if any), with
/// the key files `keys` and the admin key at `/cfg/admin-key` unless given.
fn parts(
    access: Option<&str>,
    keys: &[(&str, &str)],
    answer: fn() -> Result<Registered, Failure>,
) -> (AgentParts<NoRecord>, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let parts = AgentParts {
        connection: NoRecord {
            key: access.map(PathBuf::from),
        },
        admin_key: Box::new(AdminAt("/cfg/admin-key".into())),
        admin_path: "/mcp-admin",
        servers: Box::new(Answering {
            seen: seen.clone(),
            answer,
        }),
        credentials: Box::new(Files(seen.clone())),
        key_files: Box::new(Keys(
            keys.iter()
                .map(|(path, text)| (PathBuf::from(path), text.to_string()))
                .collect(),
        )),
        clients: Box::new(NoClients),
        client_command: Box::new(kkt_client_command::testing::Recorder::default()),
        program: Box::new(Kkt),
        report: Box::new(Lines),
        output: Box::new(Out(seen.clone())),
        clock: Arc::new(kkt_clock::testing::FixedClock::new(
            kkt_clock::Time::from_unix_seconds(0).expect("a time"),
        )),
    };
    (parts, seen)
}

fn registered() -> Result<Registered, Failure> {
    Ok(Registered {
        handle: "amber-falcon".into(),
        token: "act_token".into(),
        recovery_code: "rc_code".into(),
    })
}

fn lane_gone() -> Result<Registered, Failure> {
    Err(Failure::Refused {
        code: "lane_unavailable".into(),
        message: "the lane gone is retired, so it can no longer be taken.".into(),
    })
}

fn lost() -> Result<Registered, Failure> {
    Err(Failure::Lost("connection refused".into()))
}

fn new_avatar(lane: Option<&str>) -> NewAvatar<'_> {
    NewAvatar {
        handle: None,
        lane,
        description: None,
    }
}

#[test]
fn the_access_key_is_read_trimmed_and_named_by_its_file() {
    let (parts, seen) = parts(Some("/cfg/key"), &[("/cfg/key", "secret\n")], registered);
    block_on(register(
        &parts,
        ConnectionOptions::NONE,
        "/me/token".into(),
        Some("/me/recovery".into()),
        new_avatar(None),
    ))
    .expect("registered");
    let seen = seen.borrow();
    assert_eq!(
        seen.addresses,
        [Address {
            url: "http://127.0.0.1:7420/mcp".into(),
            key: Some("secret".into()),
            key_name: Some("the access key in /cfg/key".into()),
        }]
    );
    assert_eq!(
        seen.saved,
        [
            (PathBuf::from("/me/token"), "act_token".to_string()),
            (PathBuf::from("/me/recovery"), "rc_code".to_string()),
        ]
    );
    assert_eq!(seen.output, "amber-falcon\n");
}

#[test]
fn without_an_access_key_the_server_is_reached_with_none() {
    let (parts, seen) = parts(None, &[], registered);
    block_on(reclaim(
        &parts,
        ConnectionOptions::NONE,
        "amber-falcon",
        "/me/recovery".into(),
        "/me/token".into(),
    ))
    .expect("reclaimed");
    let seen = seen.borrow();
    assert_eq!(seen.addresses[0].key, None);
    assert_eq!(seen.addresses[0].key_name, None);
    assert_eq!(seen.output, "amber-falcon\n");
}

#[test]
fn an_unreadable_access_key_is_named_before_any_file_is_made() {
    let (parts, seen) = parts(Some("/cfg/key"), &[], registered);
    let err = block_on(register(
        &parts,
        ConnectionOptions::NONE,
        "/me/token".into(),
        None,
        new_avatar(None),
    ))
    .expect_err("no key");
    assert_eq!(
        err,
        "could not read key file /cfg/key: No such file or directory (os error 2)"
    );
    assert!(seen.borrow().addresses.is_empty());
    assert!(seen.borrow().saved.is_empty());
}

#[test]
fn a_refused_lane_is_said_and_no_credential_is_saved() {
    let (parts, seen) = parts(None, &[], lane_gone);
    let err = block_on(register(
        &parts,
        ConnectionOptions::NONE,
        "/me/token".into(),
        None,
        new_avatar(Some("gone")),
    ))
    .expect_err("refused");
    assert_eq!(
        err,
        "the server refused the lane: the lane gone is retired, so it can no longer be taken."
    );
    assert!(seen.borrow().saved.is_empty());
}

#[test]
fn a_lost_server_is_said_without_its_details() {
    let (parts, _) = parts(None, &[], lost);
    let err = block_on(register(
        &parts,
        ConnectionOptions::NONE,
        "/me/token".into(),
        None,
        new_avatar(None),
    ))
    .expect_err("lost");
    assert_eq!(err, "could not reach the server or read its reply");
}

#[test]
fn a_bad_handle_is_refused_before_the_server_is_asked() {
    let (parts, seen) = parts(None, &[], registered);
    let asked = NewAvatar {
        handle: Some("Not A Handle"),
        lane: None,
        description: None,
    };
    block_on(register(
        &parts,
        ConnectionOptions::NONE,
        "/me/token".into(),
        None,
        asked,
    ))
    .expect_err("bad handle");
    assert!(seen.borrow().addresses.is_empty());
}

#[test]
fn the_admin_endpoint_is_reached_with_the_admin_key_not_the_access_key() {
    let (parts, _) = parts(
        Some("/cfg/key"),
        &[("/cfg/key", "access"), ("/cfg/admin-key", "admin\n")],
        registered,
    );
    let flags = ConnectionOptions {
        url: Some("http://example.test:7420/mcp".into()),
        ..ConnectionOptions::NONE
    };
    assert_eq!(
        client::admin(&parts, flags),
        Ok(Address {
            url: "http://example.test:7420/mcp-admin".into(),
            key: Some("admin".into()),
            key_name: Some("the admin key in /cfg/admin-key".into()),
        })
    );
}

#[test]
fn a_missing_admin_key_says_how_to_make_one() {
    let (parts, _) = parts(None, &[], registered);
    let flags = ConnectionOptions {
        key_file: Some("/elsewhere/admin".into()),
        ..ConnectionOptions::NONE
    };
    assert_eq!(
        client::admin(&parts, flags),
        Err("there is no admin key at /elsewhere/admin; \
             `kkt key create --admin` makes one on the server"
            .into())
    );
}

#[test]
fn a_lane_refusal_is_known_by_its_code() {
    assert!(lane_refused(
        "lane_unavailable",
        "no lane named nope: `list_lanes` shows the lanes you can take."
    ));
    assert!(lane_refused(
        "lane_unavailable",
        "the lane gone is retired, so it can no longer be taken."
    ));
}

#[test]
fn another_refusal_that_mentions_a_lane_is_about_the_avatar() {
    for code in ["handle_taken", "rate_limited", "limit_reached", "internal"] {
        assert!(
            !lane_refused(code, "the handle lane-keeper is taken"),
            "{code}"
        );
    }
}

#[test]
fn an_older_servers_lane_refusals_are_still_known() {
    assert!(lane_refused(
        "not_found",
        "no lane named nope: `list_lanes` shows the lanes you can take."
    ));
    assert!(lane_refused(
        "invalid_argument",
        "the lane gone is retired, so it can no longer be taken."
    ));
    assert!(!lane_refused("invalid_argument", "the handle is too long"));
}
