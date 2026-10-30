//! The `kkt setup` conversation: which questions, in what order, and what
//! follows from each answer. Everything that changes the machine goes
//! through [`Steps`], so the flow is tested against a fake.

use super::ask::{Ask, Question};
use super::record::{DockerSetup, Mode, Record, TailscaleLogin, Unfinished};
use crate::clients::Client;
use crate::config::{
    DEFAULT_SETUP_CONTAINERS, DEFAULT_SETUP_KEY, DEFAULT_SETUP_MODE_INDEX, DEFAULT_SETUP_TAILSCALE,
    DEFAULT_SETUP_URL, DEFAULT_TS_HOSTNAME,
};
use std::path::{Path, PathBuf};

/// What this machine is for, as chosen with `--mode` or asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Service,
    Docker,
    Client,
}

impl Purpose {
    pub const ALL: [Purpose; 3] = [Purpose::Docker, Purpose::Service, Purpose::Client];

    /// The `--mode` value.
    pub fn name(self) -> &'static str {
        match self {
            Purpose::Service => "service",
            Purpose::Docker => "docker",
            Purpose::Client => "client",
        }
    }

    pub fn from_name(name: &str) -> Option<Purpose> {
        Purpose::ALL.into_iter().find(|p| p.name() == name)
    }

    pub(super) fn describe(self) -> &'static str {
        match self {
            Purpose::Service => "Run the kkt server here, as a service that starts when you log in",
            Purpose::Docker => "Run the kkt server here, in Docker (Recommended)",
            Purpose::Client => "Connect this machine's clients to a kkt server elsewhere",
        }
    }
}

/// The answers given as flags. `None` means ask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    pub purpose: Option<Purpose>,
    /// Share the server over Tailscale.
    pub tailscale: Option<bool>,
    /// For a client: the server's MCP endpoint.
    pub url: Option<String>,
    /// For a client: a file holding the server's access key.
    pub key_file: Option<PathBuf>,
    /// For a server: host names it answers to besides its own, such as
    /// `host.docker.internal`. Empty means none.
    pub allow_hosts: Option<Vec<String>>,
    /// The clients to connect. Empty means none.
    pub clients: Option<Vec<Client>>,
    /// For Docker over Tailscale: the machine name to ask for;
    /// [`DEFAULT_TS_HOSTNAME`] if not given.
    pub ts_hostname: Option<String>,
    /// For Docker over Tailscale: an absolute path to a file holding an auth
    /// key. Without it the person logs in with a link.
    pub ts_authkey_file: Option<PathBuf>,
    /// Answer yes to every yes-or-no question that has no flag of its own,
    /// such as connecting each client found, or continuing an unfinished
    /// setup. It never offers to try a failed step again, even at a terminal.
    pub yes: bool,
}

/// A server set up on this machine: how its clients reach it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub url: String,
    pub key_file: Option<PathBuf>,
}

/// What `kkt setup` does to the machine. The real implementation is in
/// `main.rs`.
pub trait Steps {
    /// Installs and starts the server as a service that also answers to
    /// `allow_hosts`.
    fn install_service(&self, tailscale: bool, allow_hosts: &[String])
    -> Result<Installed, String>;
    /// Sets up and starts the server in Docker, shared over Tailscale with
    /// `tailscale` if given, and also answering to `allow_hosts`.
    fn install_docker(
        &self,
        tailscale: Option<&TailscaleLogin>,
        allow_hosts: &[String],
    ) -> Result<(Installed, DockerSetup), String>;
    /// Saves an access key typed in for a client-only machine where nothing
    /// uses it yet, readable only by its owner, and returns where.
    fn stage_key(&self, key: &str) -> Result<PathBuf, String>;
    /// Moves a staged key to where clients of the server at `url` keep it,
    /// and returns that path. Keys for other servers are left alone.
    fn keep_key(&self, staged: &Path, url: &str) -> Result<PathBuf, String>;
    /// Removes a staged key that won't be kept.
    fn discard_key(&self, staged: &Path) -> Result<(), String>;
    /// The clients installed on this machine.
    fn detect_clients(&self) -> Vec<Client>;
    /// Writes the setup record, replacing any earlier one whole.
    fn write_record(&self, record: &Record) -> Result<(), String>;
    /// Points `client` at the server in `record`. Returns what it did.
    fn connect(&self, client: Client, record: &Record) -> Result<String, String>;
    /// Checks the connection to the server in `record`. Returns the report,
    /// or the report as an error when it found problems.
    fn doctor(&self, record: &Record) -> Result<String, String>;
    /// Checks, changing nothing, that a server answers at `url` and accepts
    /// its host name. Returns the problems found as an error.
    fn check_address(&self, url: &str) -> Result<(), String>;
    /// Checks, changing nothing, that the server at `url` accepts `key`.
    /// Returns the problems found as an error.
    fn check_key(&self, url: &str, key: &ClientKey) -> Result<(), String>;
    /// The server an earlier setup installed but did not record, if one was
    /// noted and is still installed.
    fn unfinished(&self) -> Result<Option<Unfinished>, String>;
    /// Notes a server just installed, until the record is written.
    fn save_unfinished(&self, unfinished: &Unfinished) -> Result<(), String>;
    /// Removes the note, if there is one, once the record covers the server.
    fn forget_unfinished(&self) -> Result<(), String>;
}

const MODE: Question = Question {
    text: "What is this machine for?",
    flag: "--mode",
    default: None,
};
const TAILSCALE: Question = Question {
    text: "Share the server over Tailscale, so your other machines can reach it?",
    flag: "--tailscale or --no-tailscale",
    default: Some(DEFAULT_SETUP_TAILSCALE),
};
const CONTAINERS: Question = Question {
    text: "Will agents in Docker containers on this machine use this server? They reach it as \
           host.docker.internal.",
    flag: "--allow-host host.docker.internal, or --allow-host none",
    default: Some(DEFAULT_SETUP_CONTAINERS),
};
/// The name Docker gives containers for the machine they run on.
const DOCKER_HOST: &str = "host.docker.internal";
const TRY_AGAIN: Question = Question {
    text: "Try again?",
    flag: "",
    default: Some(true),
};
const CONTINUE: Question = Question {
    text: "Continue that setup?",
    flag: "",
    default: Some(true),
};
const URL: Question = Question {
    text: "The server's MCP address, such as https://kkt.tail1234.ts.net/mcp:",
    flag: "--url",
    default: None,
};
const KEY: Question = Question {
    text: "The server's access key (run `kkt key show` there), or nothing if it has none:",
    flag: "--key-file",
    default: None,
};

/// Runs `kkt setup`: asks what `answers` leaves open, sets the machine up,
/// checks the connection, connects clients, and records it. Returns the
/// record written.
///
/// Every question comes first, so a missing answer changes nothing. The one
/// exception needs no flag: at a terminal, and without `--yes`, a step that
/// fails is offered again (`Try again?`); otherwise it ends setup.
///
/// The record is written last, and only when everything worked: a setup
/// that fails at any step leaves an earlier record, and the key it names,
/// as they were. A server install is noted apart from the record
/// ([`Unfinished`]) before it starts, with the answers that shaped it, and
/// again once it has finished; a setup that then fails says so. The next
/// run finds the note and continues: from the install if it had not
/// finished, otherwise from the connection check.
pub fn run(
    answers: &Answers,
    version: &str,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, String> {
    let offer = ask.interactive() && !answers.yes;
    let plan = match resumable(answers, ask, steps)? {
        Some(unfinished) => Plan::Resume(unfinished),
        None => ask_plan(answers, ask, steps)?,
    };

    let (record, staged, chosen, unfinished) = match plan {
        Plan::Resume(mut unfinished) => {
            // The clients chosen before, unless others are given now.
            if let Some(chosen) = &answers.clients
                && *chosen != unfinished.clients
            {
                unfinished.clients = chosen.clone();
                if let Err(e) = steps.save_unfinished(&unfinished) {
                    ask.tell(&format!("Could not note the clients chosen now: {e}"));
                }
            }
            let (record, unfinished) = match unfinished.record.clone() {
                Some(record) => (record, unfinished),
                None => install(unfinished, version, offer, ask, steps)?,
            };
            (record, None, unfinished.clients.clone(), Some(unfinished))
        }
        Plan::Server(purpose, tailscale, allow_hosts) => {
            let tailscale_login =
                (purpose == Purpose::Docker && tailscale).then(|| TailscaleLogin {
                    hostname: answers
                        .ts_hostname
                        .clone()
                        .unwrap_or_else(|| DEFAULT_TS_HOSTNAME.to_string()),
                    auth_key_file: answers.ts_authkey_file.clone(),
                });
            let unfinished = Unfinished {
                purpose,
                tailscale,
                allow_hosts,
                clients: choose_clients(answers, ask, steps)?,
                tailscale_login,
                record: None,
            };
            retried(offer, ask, "Noting the install", || {
                steps.save_unfinished(&unfinished)
            })
            .map_err(|e| {
                format!(
                    "{e}
setup notes the install before starting it, so nothing was changed"
                )
            })?;
            let (record, unfinished) = install(unfinished, version, offer, ask, steps)?;
            (record, None, unfinished.clients.clone(), Some(unfinished))
        }
        Plan::Client(ClientAnswers { url, key }) => {
            let chosen = choose_clients(answers, ask, steps)?;
            let (key_file, staged) = match key {
                ClientKey::None => (None, None),
                ClientKey::File(path) => (Some(path), None),
                ClientKey::Typed(key) => {
                    let staged = retried(offer, ask, "Saving the key", || steps.stage_key(&key))?;
                    (Some(staged.clone()), Some(staged))
                }
            };
            let record = Record {
                mode: Mode::Client,
                written_by: version.to_string(),
                url,
                key_file,
            };
            (record, staged, chosen, None)
        }
    };

    finish(record, staged.as_deref(), chosen, offer, ask, steps).map_err(|e| match &unfinished {
        Some(unfinished) => format!("{e}.\n{}", unfinished_advice(unfinished)),
        None => format!("{e}; fix that and run `kkt setup` again"),
    })
}

/// Installs the server `unfinished` describes, and notes the record it
/// leads to. Returns that record and the note. On an error, the note from
/// before the install stays, so the next setup installs again.
fn install(
    mut unfinished: Unfinished,
    version: &str,
    offer: bool,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<(Record, Unfinished), String> {
    let (tailscale, allow_hosts) = (unfinished.tailscale, &unfinished.allow_hosts);
    let installed = retried(offer, ask, "Installing the server", || {
        match unfinished.purpose {
            Purpose::Docker => steps
                .install_docker(unfinished.tailscale_login.as_ref(), allow_hosts)
                .map(|(installed, docker)| (Mode::Docker(docker), installed)),
            _ => steps
                .install_service(tailscale, allow_hosts)
                .map(|installed| (Mode::Service { tailscale }, installed)),
        }
    });
    let (mode, installed) = match installed {
        Ok(done) => done,
        Err(e) => return Err(format!("{e}\n{}", unfinished_advice(&unfinished))),
    };
    let record = Record {
        mode,
        written_by: version.to_string(),
        url: installed.url,
        key_file: installed.key_file,
    };
    unfinished.record = Some(record.clone());
    if let Err(e) = steps.save_unfinished(&unfinished) {
        ask.tell(&format!(
            "Could not note that the install finished, so a later `kkt setup` installs again: {e}"
        ));
    }
    Ok((record, unfinished))
}

/// What to tell the person about a server installed, or begun, but not
/// recorded.
fn unfinished_advice(unfinished: &Unfinished) -> String {
    let from = match unfinished.record {
        Some(_) => "the connection check",
        None => "the install",
    };
    format!(
        "{} Until then, `kkt upgrade` and `kkt uninstall` do not manage it. Run `kkt setup` \
         again to finish: it continues from {from}.",
        unfinished.summary()
    )
}

/// The server an earlier setup installed, or began to, but did not record,
/// if setup should continue it: the answers given now agree with it, and at
/// a terminal the person agrees too.
fn resumable(
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Option<Unfinished>, String> {
    let unfinished = match steps.unfinished() {
        Ok(Some(unfinished)) => unfinished,
        Ok(None) => return Ok(None),
        Err(e) => {
            ask.tell(&format!("{e}; setup starts over."));
            return Ok(None);
        }
    };
    ask.tell(&unfinished.summary());
    let login = unfinished.tailscale_login.as_ref();
    let differs = answers.purpose.is_some_and(|p| p != unfinished.purpose)
        || answers.tailscale.is_some_and(|t| t != unfinished.tailscale)
        || answers
            .allow_hosts
            .as_ref()
            .is_some_and(|hosts| *hosts != unfinished.allow_hosts)
        || login.is_some_and(|login| {
            answers
                .ts_hostname
                .as_ref()
                .is_some_and(|h| *h != login.hostname)
                || answers
                    .ts_authkey_file
                    .as_ref()
                    .is_some_and(|f| login.auth_key_file.as_ref() != Some(f))
        });
    if differs {
        ask.tell("The answers given now are different, so setup starts over.");
        return Ok(None);
    }
    if ask.interactive() && !answers.yes && !ask.confirm(CONTINUE)? {
        return Ok(None);
    }
    ask.tell(match unfinished.record {
        Some(_) => "Continuing from the connection check.",
        None => "Continuing from the install.",
    });
    Ok(Some(unfinished))
}

/// Asks the questions that decide what to set up.
fn ask_plan(answers: &Answers, ask: &dyn Ask, steps: &dyn Steps) -> Result<Plan, String> {
    let purpose = match answers.purpose {
        Some(purpose) => purpose,
        None => {
            let options = Purpose::ALL.map(Purpose::describe);
            Purpose::ALL[ask.choose(MODE, &options, DEFAULT_SETUP_MODE_INDEX)?]
        }
    };
    if purpose == Purpose::Client {
        return Ok(Plan::Client(client_answers(answers, ask, steps)?));
    }
    let tailscale = match answers.tailscale {
        Some(answer) => answer,
        None => ask.confirm(TAILSCALE)?,
    };
    let allow_hosts = match &answers.allow_hosts {
        Some(hosts) => hosts.clone(),
        None if ask.confirm(CONTAINERS)? => vec![DOCKER_HOST.to_string()],
        None => Vec::new(),
    };
    Ok(Plan::Server(purpose, tailscale, allow_hosts))
}

/// Everything after the install: checks the connection, keeps a staged
/// key, connects `chosen`, and writes the record. An error says what
/// failed and that nothing was recorded.
fn finish(
    mut record: Record,
    staged: Option<&Path>,
    chosen: Vec<Client>,
    offer: bool,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, String> {
    match retried(offer, ask, "The connection check", || steps.doctor(&record)) {
        Ok(report) => ask.tell(&report),
        Err(report) => {
            // When it offered to try again, `retried` has shown it.
            if !offer {
                ask.tell(&report);
            }
            if let Some(staged) = staged
                && let Err(e) = steps.discard_key(staged)
            {
                ask.tell(&format!("Could not remove the unused key: {e}"));
            }
            return Err(
                "the connection check found problems (see the fixes above), so nothing was \
                 recorded"
                    .to_string(),
            );
        }
    }
    if let Some(staged) = staged {
        let kept = retried(offer, ask, "Saving the key", || {
            steps.keep_key(staged, &record.url)
        })?;
        record.key_file = Some(kept);
    }

    let mut failed = Vec::new();
    for client in chosen {
        let what = format!("Connecting {}", client.display_name());
        match retried(offer, ask, &what, || steps.connect(client, &record)) {
            Ok(done) => ask.tell(&done),
            Err(e) => {
                if !offer {
                    ask.tell(&format!("Could not connect {}: {e}", client.display_name()));
                }
                failed.push(client.display_name());
            }
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "could not connect {}, so nothing was recorded",
            failed.join(", ")
        ));
    }

    retried(offer, ask, "Writing the setup record", || {
        steps.write_record(&record)
    })?;
    if record.mode != Mode::Client
        && let Err(e) = steps.forget_unfinished()
    {
        ask.tell(&format!(
            "The setup is recorded, but the note of it as unfinished could not be removed: {e}"
        ));
    }
    let web = record.url.strip_suffix("/mcp").unwrap_or(&record.url);
    ask.tell(&format!(
        "This machine is set up as {}; clients use {}, and the web page is at {web}/.",
        record.mode.name(),
        record.url
    ));
    Ok(record)
}

/// Runs `step` until it works. When it fails and `offer` is set (at a
/// terminal, without `--yes`), says what failed and offers to try again, so
/// the person can fix the problem, or wait, without leaving setup. Without
/// `offer`, or when they say no, returns the error.
fn retried<T>(
    offer: bool,
    ask: &dyn Ask,
    what: &str,
    mut step: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    loop {
        let problem = match step() {
            Ok(done) => return Ok(done),
            Err(problem) => problem,
        };
        if !offer {
            return Err(problem);
        }
        ask.tell(&format!("{what} did not work:\n{problem}"));
        match ask.confirm(TRY_AGAIN) {
            Ok(true) => {}
            Ok(false) => return Err(problem),
            Err(e) => return Err(format!("{problem}\n{e}")),
        }
    }
}

/// What setup will do, once every question is answered.
enum Plan {
    /// Run the server here: as a service or in Docker, whether over
    /// Tailscale, and the other host names it answers to.
    Server(Purpose, bool, Vec<String>),
    Client(ClientAnswers),
    /// Continue a server an earlier setup installed, or began to, but did
    /// not record.
    Resume(Unfinished),
}

/// Where a client-only machine's server is, and its key.
struct ClientAnswers {
    url: String,
    key: ClientKey,
}

/// The key a client-only machine uses for its server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientKey {
    /// The server has no key.
    None,
    /// A file given with `--key-file`, used where it is.
    File(PathBuf),
    /// A key typed in, saved by setup.
    Typed(String),
}

/// Asks for the server's address and key, checking each as soon as it is
/// given and asking again on a problem. An answer given as a flag is
/// checked the same way, and a problem with it ends setup.
fn client_answers(
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<ClientAnswers, String> {
    loop {
        let url = checked_address(answers, ask, steps)?;
        if let Some(key) = checked_key(&url, answers, ask, steps)? {
            return Ok(ClientAnswers { url, key });
        }
    }
}

/// The server's address, once it passes its check.
fn checked_address(answers: &Answers, ask: &dyn Ask, steps: &dyn Steps) -> Result<String, String> {
    loop {
        let url = match &answers.url {
            Some(url) => url.trim().to_string(),
            None => ask.text(URL, DEFAULT_SETUP_URL)?.trim().to_string(),
        };
        match check_address(&url, steps) {
            Ok(()) => return Ok(url),
            Err(problem) if answers.url.is_some() => return Err(flag_failed(URL, &problem)),
            Err(problem) => ask.tell(&format!("That address did not work:\n{problem}")),
        }
    }
}

/// The key, once the server at `url` accepts it. When a key fails, the
/// address is checked again, so a problem with the address is never taken
/// for one with the key: `None` means it no longer passes and must be asked
/// for again.
fn checked_key(
    url: &str,
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Option<ClientKey>, String> {
    loop {
        let key = match &answers.key_file {
            Some(path) => ClientKey::File(path.clone()),
            None => match typed_key(&ask.text(KEY, DEFAULT_SETUP_KEY)?) {
                Ok(key) => key,
                Err(problem) => {
                    ask.tell(&problem);
                    continue;
                }
            },
        };
        let Err(problem) = steps.check_key(url, &key) else {
            return Ok(Some(key));
        };
        match steps.check_address(url) {
            Err(problem) if answers.url.is_some() => return Err(flag_failed(URL, &problem)),
            Err(problem) => {
                ask.tell(&format!("That address no longer works:\n{problem}"));
                return Ok(None);
            }
            Ok(()) if answers.key_file.is_some() => return Err(flag_failed(KEY, &problem)),
            Ok(()) => ask.tell(&format!("The server did not accept that key:\n{problem}")),
        }
    }
}

fn check_address(url: &str, steps: &dyn Steps) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!(
            "{url:?} is not a server address: it starts with http:// or https://"
        ));
    }
    steps.check_address(url)
}

/// The key typed in answer to [`KEY`]: none when blank.
fn typed_key(text: &str) -> Result<ClientKey, String> {
    match text.trim() {
        "" => Ok(ClientKey::None),
        key => {
            crate::config::check_access_key(key)
                .map_err(|e| format!("that key is not valid: {e}"))?;
            Ok(ClientKey::Typed(key.to_string()))
        }
    }
}

/// The error for an answer given as a flag that failed its check.
fn flag_failed(question: Question, problem: &str) -> String {
    format!(
        "the answer given with {} did not pass its check, so nothing was changed:\n{problem}",
        question.flag
    )
}

/// The clients to connect: those given with `--clients`, or each one found
/// that the person agrees to (all of them with `--yes`).
fn choose_clients(
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Vec<Client>, String> {
    if let Some(chosen) = &answers.clients {
        return Ok(chosen.clone());
    }
    let mut chosen = Vec::new();
    for client in steps.detect_clients() {
        let question = Question {
            text: &format!("Connect {} to kkt?", client.display_name()),
            flag: "--clients",
            default: Some(true),
        };
        if answers.yes || ask.confirm(question)? {
            chosen.push(client);
        }
    }
    Ok(chosen)
}
