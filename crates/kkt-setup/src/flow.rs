//! The `kkt setup` conversation: which questions, in what order, and what
//! follows from each answer. Everything that changes the machine goes
//! through [`Steps`], so the flow is tested against a fake.

use super::adopt::{AdoptSteps, Unrecorded, adoptable};
use super::ask::{Ask, Question};
use super::record::{DockerSetup, Mode, Record, Server, TailscaleLogin, Unfinished};
use super::switch::SwitchSteps;
use kkt_client_config::Client;
use kkt_config::{
    DEFAULT_SETUP_ADMIN_KEY, DEFAULT_SETUP_ADMIN_NAME, DEFAULT_SETUP_ALLOW_KKT,
    DEFAULT_SETUP_CONNECT_CLIENT, DEFAULT_SETUP_CONTAINERS, DEFAULT_SETUP_CONTINUE,
    DEFAULT_SETUP_KEY, DEFAULT_SETUP_MODE_INDEX, DEFAULT_SETUP_TAILSCALE, DEFAULT_SETUP_TRY_AGAIN,
    DEFAULT_SETUP_URL, DEFAULT_TS_HOSTNAME, MAX_ADMIN_NAME_CHARS, SETUP_DOCKER_HOST,
};
use kkt_names::AdminName;
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
    /// Let the clients connected always use kkt's tools and `kkt watch`,
    /// without asking each time. Asked at a terminal when a client that
    /// has such a setting is chosen; without a terminal, or when
    /// continuing a setup, [`DEFAULT_SETUP_ALLOW_KKT`].
    pub allow_kkt: Option<bool>,
    /// For Docker over Tailscale: the machine name to ask for;
    /// [`DEFAULT_TS_HOSTNAME`] if not given.
    pub ts_hostname: Option<String>,
    /// For Docker over Tailscale: an absolute path to a file holding an auth
    /// key. Without it the person logs in with a link.
    pub ts_authkey_file: Option<PathBuf>,
    /// For a server: give it an admin key, turning on `/mcp-admin`. Asked at
    /// a terminal; without one, [`DEFAULT_SETUP_ADMIN_KEY`].
    pub admin_key: Option<bool>,
    /// For a new admin key: its admin agent's name, as `kkt key create
    /// --admin --name` takes it. Asked at a terminal; without one, or when
    /// continuing a setup, [`DEFAULT_SETUP_ADMIN_NAME`].
    pub admin_name: Option<String>,
    /// Answer yes to every yes-or-no question that has no flag of its own,
    /// such as connecting each client found, or continuing an unfinished
    /// setup. It never offers to try a failed step again, even at a terminal.
    pub yes: bool,
    /// Record a server kkt installed here with no record ([`Unrecorded`])
    /// without asking. Without a terminal, setup adopts one only with this.
    pub adopt: bool,
    /// For a server: a folder of lane files to import once it is set up
    /// (`kkt lanes import`); `Some(None)` for none. Never asked: without
    /// it, none. Not noted with an unfinished setup, so continuing one
    /// imports only what this gives.
    pub lanes: Option<Option<PathBuf>>,
}

/// A server set up on this machine: how its clients reach it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub url: String,
    pub key_file: Option<PathBuf>,
}

/// What `kkt setup` does to the machine. The real implementation is in the
/// program's `src/commands/setup/steps.rs`. It includes what `kkt switch` does ([`SwitchSteps`]) and how
/// setup finds a server to adopt ([`AdoptSteps`]).
pub trait Steps: SwitchSteps + AdoptSteps {
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
    /// Lets `client`, connected to kkt, always use kkt's tools and `kkt
    /// watch`, adding only those rules to its own files. Called only for a
    /// client that can ([`Client::can_allow_kkt`]). Returns what it did.
    fn allow_kkt(&self, client: Client) -> Result<String, String>;
    /// Checks the connection to the server `record`'s clients use. Returns
    /// the report, or the report as an error when it found problems.
    fn doctor(&self, record: &Record) -> Result<String, String>;
    /// The server an earlier setup installed but did not record, if one was
    /// noted and is still installed.
    fn unfinished(&self) -> Result<Option<Unfinished>, String>;
    /// Notes a server just installed, until the record is written.
    fn save_unfinished(&self, unfinished: &Unfinished) -> Result<(), String>;
    /// Removes the note, if there is one, once the record covers the server.
    fn forget_unfinished(&self) -> Result<(), String>;
    /// Gives the server installed as `record` an admin key named `name`,
    /// unless it has one, and restarts it so it serves `/mcp-admin`. Returns
    /// where this machine keeps the key.
    fn add_admin_key(&self, record: &Record, name: &AdminName) -> Result<PathBuf, String>;
    /// Reads the lane files in `folder` and checks every lane as `kkt lanes
    /// import` would on the `purpose` server set up here, with that server's
    /// lane text limit, changing nothing; returns how many lanes it has. An
    /// error says which file or lane is wrong.
    fn check_lanes(&self, folder: &Path, purpose: Purpose) -> Result<usize, String>;
    /// Imports the lane files in `folder` into the server installed as
    /// `record`, as `kkt lanes import` does. Returns its report.
    fn import_lanes(&self, record: &Record, folder: &Path) -> Result<String, String>;
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
const TRY_AGAIN: Question = Question {
    text: "Try again?",
    flag: "",
    default: Some(DEFAULT_SETUP_TRY_AGAIN),
};
const CONTINUE: Question = Question {
    text: "Continue that setup?",
    flag: "",
    default: Some(DEFAULT_SETUP_CONTINUE),
};
const ADMIN_KEY: Question = Question {
    text: "Make an admin key, so an agent holding it can supervise the server at /mcp-admin? \
           It sees every channel, closed ones included.",
    flag: "--admin-key or --no-admin-key",
    default: Some(DEFAULT_SETUP_ADMIN_KEY),
};
const ADMIN_NAME: Question = Question {
    text: "The admin agent's name, such as sentinel (it becomes admin-sentinel). Its changes \
           are announced as \"operator (via admin-sentinel)\":",
    flag: "--admin-name",
    default: None,
};
/// The flag that gives a folder of lanes. Setup never asks for one.
const LANES_FLAG: &str = "--lanes";
const ALLOW_KKT: Question = Question {
    text: "Allow the clients set up here to always use kkt's tools and `kkt watch`, without \
           asking each time?",
    flag: "--allow-kkt or --no-allow-kkt",
    default: Some(DEFAULT_SETUP_ALLOW_KKT),
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
///
/// With neither a record nor a note, a server kkt installed here anyway
/// (left by an older kkt) can be adopted instead ([`Unrecorded`]): its
/// record is written first, from what was found, and setup goes on to the
/// connection check and clients without installing anything.
///
/// On a machine whose record says it runs a server, the server stays
/// recorded: setting up a client there switches only the clients
/// ([`Record::switched`]), and setting up the other kind of server is
/// refused, since it would leave the first one running with nothing to
/// manage it. A server an earlier setup installed but did not record (its
/// note says so) counts the same when this setup does not continue it:
/// setting up a client records it, with only the clients switched, and
/// setting up the other kind of server is refused.
///
/// A folder of lanes, given with `--lanes`, is checked with the other
/// answers and imported last, once the setup is recorded. Setup does not
/// ask about lanes.
pub fn run(
    answers: &Answers,
    version: &str,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, String> {
    let offer = ask.interactive() && !answers.yes;
    let local = match steps.recorded() {
        Ok(record) => record.filter(Record::runs_server),
        Err(e) => {
            ask.tell(&format!("{e}; setup replaces it."));
            None
        }
    };
    let noted = match steps.unfinished() {
        Ok(noted) => noted,
        Err(e) => {
            ask.tell(&format!("{e}; setup starts over."));
            None
        }
    };
    let resumed = resumable(answers, ask, noted.clone())?;
    // A server an earlier setup installed, but did not record, that this
    // setup does not continue. It runs here like a recorded one.
    let left = match &resumed {
        Some(_) => None,
        None => noted.filter(|noted| noted.record.as_ref().is_some_and(Record::runs_server)),
    };
    // Whether to give a server an admin key, and its name, is decided with
    // the plan.
    let (plan, admin) = match resumed {
        // Continuing asks nothing more: only `--admin-key` adds one, and
        // not with `--adopt`, which installs nothing, an admin key included.
        Some(unfinished) => (
            Plan::Resume(unfinished),
            match answers.adopt {
                true => None,
                false => resumed_admin_key(answers)?,
            },
        ),
        None => match adoptable(answers, ask, steps)? {
            // Adopting installs nothing, an admin key included.
            Some(found) => (Plan::Adopt(found), None),
            None => match ask_plan(answers, ask, steps)? {
                plan @ Plan::Client(_) => (plan, None),
                plan => (plan, admin_key(answers, ask)?),
            },
        },
    };
    let lanes = lanes_folder(answers, &plan, ask, steps)?;
    let purpose = match &plan {
        Plan::Server(purpose, _, _) => Some(*purpose),
        Plan::Resume(unfinished) => Some(unfinished.purpose),
        Plan::Client(_) | Plan::Adopt(_) => None,
    };
    if let Some(purpose) = purpose {
        if let Some(local) = &local {
            keeps_local_server(local, purpose)?;
        }
        if let Some(left) = &left {
            keeps_noted_server(left, purpose)?;
        }
    }
    // With no record of a server here, a client setup keeps the noted one
    // managed by recording it, with only its clients switched.
    let (local, records_noted) = match (local, &left) {
        (Some(local), _) => (Some(local), false),
        (None, Some(left)) => (left.record.clone(), true),
        (None, None) => (None, false),
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
            let chosen = Chosen {
                allow_kkt: resumed_allow_kkt(answers, &unfinished.clients),
                clients: unfinished.clients.clone(),
            };
            (record, None, chosen, Some(unfinished))
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
            let chosen = choose(answers, ask, steps)?;
            let unfinished = Unfinished {
                purpose,
                tailscale,
                allow_hosts,
                clients: chosen.clients.clone(),
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
            (record, None, chosen, Some(unfinished))
        }
        Plan::Client(ClientAnswers { url, key }) => {
            let chosen = choose(answers, ask, steps)?;
            if records_noted {
                ask.tell(
                    "Setting up this machine's clients records that server too, so kkt manages \
                     it.",
                );
            }
            if let Some(local) = local.as_ref().filter(|local| local.url == url) {
                ask.tell(
                    "That is the server on this machine, so its clients go back to it, with \
                     its own key.",
                );
                let record = local.switched(local.own_server(), version);
                (record, None, chosen, None)
            } else {
                let (key_file, staged) = match key {
                    ClientKey::None => (None, None),
                    ClientKey::File(path) => (Some(path), None),
                    ClientKey::Typed(key) => {
                        let staged =
                            retried(offer, ask, "Saving the key", || steps.stage_key(&key))?;
                        (Some(staged.clone()), Some(staged))
                    }
                };
                let server = Server { url, key_file };
                let record = match &local {
                    Some(local) => {
                        ask.tell(&switch_note(local, &server.url));
                        local.switched(server, version)
                    }
                    None => Record {
                        mode: Mode::Client,
                        written_by: version.to_string(),
                        url: server.url,
                        key_file: server.key_file,
                        clients: None,
                    },
                };
                (record, staged, chosen, None)
            }
        }
        Plan::Adopt(found) => {
            let chosen = choose(answers, ask, steps)?;
            let record = found.record(version);
            // Recorded first: it says only what is installed, so a check
            // that fails below (such as one an upgrade would fix) leaves
            // `kkt upgrade` able to manage the server.
            retried(offer, ask, "Writing the setup record", || {
                steps.write_record(&record)
            })?;
            let record =
                finish(record, None, chosen, Finish::Adopted, offer, ask, steps).map_err(|e| {
                    format!(
                        "{e}.\nThe server is recorded, so `kkt status` and `kkt upgrade` manage \
                         it. For a server set up by an older kkt, `kkt upgrade` may fix the \
                         problem; then connect clients with `kkt connect <client> --write`."
                    )
                })?;
            return import_lanes(record, lanes.as_deref(), offer, ask, steps);
        }
    };

    if let Some(name) = &admin
        && let Some(unfinished) = &unfinished
        && let Err(e) = retried(offer, ask, "Making the admin key", || {
            steps.add_admin_key(&record, name)
        })
    {
        return Err(format!("{e}\n{}", unfinished_advice(unfinished)));
    }

    // A recorded noted server is installed, so its note goes once recorded.
    let kind = match (&unfinished, records_noted) {
        (Some(_), _) | (None, true) => Finish::Installed,
        (None, false) => Finish::Clients,
    };
    let finished = finish(record, staged.as_deref(), chosen, kind, offer, ask, steps);
    let record = finished.map_err(|e| match &unfinished {
        Some(unfinished) => format!("{e}.\n{}", unfinished_advice(unfinished)),
        None => format!("{e}; fix that and run `kkt setup` again"),
    })?;
    import_lanes(record, lanes.as_deref(), offer, ask, steps)
}

/// The folder of lanes to import into the server, if `--lanes` gives one;
/// setup never asks. It is checked now, before anything is installed,
/// against the kind of server the plan sets up, and a problem ends setup.
/// Refused for a client.
fn lanes_folder(
    answers: &Answers,
    plan: &Plan,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Option<PathBuf>, String> {
    let Some(Some(folder)) = &answers.lanes else {
        return Ok(None);
    };
    let purpose = match plan {
        Plan::Server(purpose, _, _) => *purpose,
        Plan::Resume(unfinished) => unfinished.purpose,
        Plan::Adopt(found) => found.purpose(),
        Plan::Client(_) => {
            return Err(
                "--lanes is for a server: run `kkt lanes import` on the machine where it runs"
                    .into(),
            );
        }
    };
    let count = steps
        .check_lanes(folder, purpose)
        .map_err(|problem| flag_failed(LANES_FLAG, &problem))?;
    ask.tell(&format!(
        "Found {count} lanes in {}; setup imports them once the server is set up.",
        folder.display()
    ));
    Ok(Some(folder.clone()))
}

/// Imports the lanes in `folder`, if given, into the server `record` names.
/// The setup is recorded by then, so a failure leaves it so, and says how
/// to import them later.
fn import_lanes(
    record: Record,
    folder: Option<&Path>,
    offer: bool,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, String> {
    let Some(folder) = folder else {
        return Ok(record);
    };
    match retried(offer, ask, "Importing the lanes", || {
        steps.import_lanes(&record, folder)
    }) {
        Ok(report) => {
            ask.tell(&report);
            Ok(record)
        }
        Err(e) => Err(format!(
            "{e}\nThe setup is recorded, but the lanes were not imported; import them with \
             `kkt lanes import {}`",
            folder.display()
        )),
    }
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
        clients: None,
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
    noted: Option<Unfinished>,
) -> Result<Option<Unfinished>, String> {
    let Some(unfinished) = noted else {
        return Ok(None);
    };
    ask.tell(&unfinished.summary());
    // Continuing it would install, and --adopt installs nothing.
    if answers.adopt && unfinished.record.is_none() {
        return Err(
            "--adopt installs nothing, but this setup had not finished installing its server; \
             run `kkt setup` without --adopt to continue it"
                .into(),
        );
    }
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
        None if ask.confirm(CONTAINERS)? => vec![SETUP_DOCKER_HOST.to_string()],
        None => Vec::new(),
    };
    Ok(Plan::Server(purpose, tailscale, allow_hosts))
}

/// Whether to give the server an admin key, and its admin agent's name: as
/// `--admin-key` or `--no-admin-key` says, else asked at a terminal. Without
/// a terminal it needs no flag, so setups scripted before admin keys existed
/// run as they did.
fn admin_key(answers: &Answers, ask: &dyn Ask) -> Result<Option<AdminName>, String> {
    let make = match answers.admin_key {
        Some(answer) => answer,
        None if ask.interactive() => ask.confirm(ADMIN_KEY)?,
        None => DEFAULT_SETUP_ADMIN_KEY,
    };
    if !make {
        return Ok(None);
    }
    let given = match &answers.admin_name {
        Some(name) => name.clone(),
        None if ask.interactive() => ask.text(ADMIN_NAME, DEFAULT_SETUP_ADMIN_NAME)?,
        None => DEFAULT_SETUP_ADMIN_NAME.to_string(),
    };
    AdminName::new(&given, MAX_ADMIN_NAME_CHARS).map(Some)
}

/// The admin key to add when continuing a setup: only `--admin-key` adds
/// one, named by `--admin-name` or [`DEFAULT_SETUP_ADMIN_NAME`]. Continuing
/// asks nothing more.
fn resumed_admin_key(answers: &Answers) -> Result<Option<AdminName>, String> {
    if !answers.admin_key.unwrap_or(DEFAULT_SETUP_ADMIN_KEY) {
        return Ok(None);
    }
    let given = answers
        .admin_name
        .as_deref()
        .unwrap_or(DEFAULT_SETUP_ADMIN_NAME);
    AdminName::new(given, MAX_ADMIN_NAME_CHARS).map(Some)
}

/// What [`finish`] completes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// A server installed now or by an earlier setup, which a note may
    /// still say is unfinished.
    Installed,
    /// A server adopted, already recorded.
    Adopted,
    /// Only clients: a client machine, or a server machine's clients.
    Clients,
}

/// Everything after the install: checks the connection, keeps a staged
/// key, connects the clients `chosen`, lets them always use kkt if chosen
/// so, and writes the record, unless it was adopted and so is recorded
/// already. An error says what failed, and that nothing was recorded.
fn finish(
    mut record: Record,
    staged: Option<&Path>,
    chosen: Chosen,
    kind: Finish,
    offer: bool,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, String> {
    let stopped = match kind {
        Finish::Adopted => "so setup stopped there",
        Finish::Installed | Finish::Clients => "so nothing was recorded",
    };
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
            return Err(format!(
                "the connection check found problems (see the fixes above), {stopped}"
            ));
        }
    }
    if let Some(staged) = staged {
        let url = record.for_clients().url;
        let kept = retried(offer, ask, "Saving the key", || {
            steps.keep_key(staged, &url)
        })?;
        match &mut record.clients {
            Some(server) => server.key_file = Some(kept),
            None => record.key_file = Some(kept),
        }
    }

    let mut failed = Vec::new();
    for &client in &chosen.clients {
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
            "could not connect {}, {stopped}",
            failed.join(", ")
        ));
    }
    if chosen.allow_kkt {
        allow_kkt(&chosen.clients, stopped, offer, ask, steps)?;
    }

    if kind != Finish::Adopted {
        retried(offer, ask, "Writing the setup record", || {
            steps.write_record(&record)
        })?;
    }
    if kind != Finish::Clients
        && let Err(e) = steps.forget_unfinished()
    {
        ask.tell(&format!(
            "The setup is recorded, but the note of it as unfinished could not be removed: {e}"
        ));
    }
    let web = record.url.strip_suffix("/mcp").unwrap_or(&record.url);
    ask.tell(&match (&record.clients, record.local_server()) {
        (Some(clients), Some(local)) => format!(
            "This machine's clients use {}. {local}, at {}, with the web page at {web}/.",
            clients.url, record.url
        ),
        _ => format!(
            "This machine is set up as {}; clients use {}, and the web page is at {web}/.",
            record.mode.name(),
            record.url
        ),
    });
    Ok(record)
}

/// Lets each of `clients`, just connected, always use kkt's tools and `kkt
/// watch`. A client with no such setting is named and passed over; one that
/// fails stops setup, after the others are tried, as a failed connection
/// does. `stopped` says what that leaves.
fn allow_kkt(
    clients: &[Client],
    stopped: &str,
    offer: bool,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<(), String> {
    let mut failed = Vec::new();
    for &client in clients {
        if !client.can_allow_kkt() {
            ask.tell(&format!(
                "{} has no setting that lets kkt's tools and `kkt watch` run without asking, \
                 so it still asks each time.",
                client.display_name()
            ));
            continue;
        }
        let what = format!("Allowing kkt in {}", client.display_name());
        match retried(offer, ask, &what, || steps.allow_kkt(client)) {
            Ok(done) => ask.tell(&done),
            Err(e) => {
                if !offer {
                    ask.tell(&format!(
                        "Could not allow kkt in {}: {e}",
                        client.display_name()
                    ));
                }
                failed.push(client.display_name());
            }
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    Err(format!(
        "could not allow kkt in {}, {stopped}. Their connections to kkt are made; running \
         setup again tries again",
        failed.join(", ")
    ))
}

/// Refuses to set up a `purpose` server on a machine whose record says a
/// `local` server of the other kind runs here: it would keep running, but
/// kkt would no longer manage it.
fn keeps_local_server(local: &Record, purpose: Purpose) -> Result<(), String> {
    let same = matches!(
        (&local.mode, purpose),
        (Mode::Service { .. }, Purpose::Service) | (Mode::Docker(_), Purpose::Docker)
    );
    if same {
        return Ok(());
    }
    let how = match purpose {
        Purpose::Docker => "in Docker",
        _ => "as a service",
    };
    Err(format!(
        "{} (see `kkt status`). Setting kkt up {how} instead would leave that server running \
         with nothing to manage it, so nothing was changed. Run `kkt uninstall` first to remove \
         it, or `kkt switch <url>` to point only this machine's clients at another server",
        local
            .local_server()
            .unwrap_or_else(|| "kkt runs here".to_string())
    ))
}

/// Refuses to set up a `purpose` server on a machine where an earlier setup
/// installed a server of the other kind, as `noted`, but did not record it:
/// as with a recorded one, it would keep running with nothing to manage it.
fn keeps_noted_server(noted: &Unfinished, purpose: Purpose) -> Result<(), String> {
    if noted.purpose == purpose {
        return Ok(());
    }
    let how = match purpose {
        Purpose::Docker => "in Docker",
        _ => "as a service",
    };
    Err(format!(
        "{} Setting kkt up {how} instead would leave that server running with nothing to \
         manage it, so nothing was changed. Run `kkt setup --mode {}` to finish that setup \
         first; then `kkt uninstall` removes it, or `kkt switch <url>` points only this \
         machine's clients at another server",
        noted.summary(),
        noted.purpose.name()
    ))
}

/// What setup says before it switches the clients of the `local` server's
/// machine to `url`.
pub(super) fn switch_note(local: &Record, url: &str) -> String {
    format!(
        "{}, and stays as it is: `kkt status` and `kkt upgrade` still manage it. Only this \
         machine's clients move, to {url}; `kkt switch --local` points them back.",
        local
            .local_server()
            .unwrap_or_else(|| "kkt runs here".to_string())
    )
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
    /// Record a server kkt installed here with no record or note.
    Adopt(Unrecorded),
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
            Err(problem) if answers.url.is_some() => return Err(flag_failed(URL.flag, &problem)),
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
            Err(problem) if answers.url.is_some() => return Err(flag_failed(URL.flag, &problem)),
            Err(problem) => {
                ask.tell(&format!("That address no longer works:\n{problem}"));
                return Ok(None);
            }
            Ok(()) if answers.key_file.is_some() => return Err(flag_failed(KEY.flag, &problem)),
            Ok(()) => ask.tell(&format!("The server did not accept that key:\n{problem}")),
        }
    }
}

/// Checks that `url` is an address and that a server answers there.
pub(super) fn check_address(url: &str, steps: &dyn SwitchSteps) -> Result<(), String> {
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
            kkt_config::check_access_key(key).map_err(|e| format!("that key is not valid: {e}"))?;
            Ok(ClientKey::Typed(key.to_string()))
        }
    }
}

/// The error for an answer given as `flag` that failed its check.
fn flag_failed(flag: &str, problem: &str) -> String {
    format!(
        "the answer given with {flag} did not pass its check, so nothing was changed:\n{problem}"
    )
}

/// The clients to connect, and whether to let them always use kkt.
struct Chosen {
    clients: Vec<Client>,
    allow_kkt: bool,
}

/// The clients to connect ([`choose_clients`]), then whether to let them
/// always use kkt: as `--allow-kkt` or `--no-allow-kkt` says, else asked at
/// a terminal, only when a chosen client has such a setting. Without a
/// terminal it needs no flag, so setups scripted before it existed run as
/// they did.
fn choose(answers: &Answers, ask: &dyn Ask, steps: &dyn Steps) -> Result<Chosen, String> {
    let clients = choose_clients(answers, ask, steps)?;
    let allow_kkt = match answers.allow_kkt {
        Some(answer) => answer,
        None if ask.interactive() && clients.iter().any(|c| c.can_allow_kkt()) => {
            ask.confirm(ALLOW_KKT)?
        }
        None => DEFAULT_SETUP_ALLOW_KKT,
    };
    Ok(Chosen { clients, allow_kkt })
}

/// Whether to let `clients` always use kkt when continuing a setup:
/// continuing asks nothing more, so only `--allow-kkt` does.
fn resumed_allow_kkt(answers: &Answers, clients: &[Client]) -> bool {
    !clients.is_empty() && answers.allow_kkt.unwrap_or(DEFAULT_SETUP_ALLOW_KKT)
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
            default: Some(DEFAULT_SETUP_CONNECT_CLIENT),
        };
        if answers.yes || ask.confirm(question)? {
            chosen.push(client);
        }
    }
    Ok(chosen)
}
