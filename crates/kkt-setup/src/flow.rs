//! The `kkt setup` conversation: which questions, in what order, and what
//! follows from each answer. Everything that changes the machine goes
//! through [`Steps`], so the flow is tested against a fake.

use super::adopt::{AdoptSteps, Unrecorded, adoptable};
use super::ask::{Ask, Question};
use super::record::{DockerSetup, Mode, Record, Server, TailscaleLogin, Unfinished};
use super::switch::SwitchSteps;
use crate::StepError;
use crate::flow_error::{ResumeFrom, SetupError, Stopped, UnfinishedAdvice};
use kkt_llm_agent_config::Client;
use kkt_names::{AdminName, MAX_ADMIN_NAME_CHARS};
use std::path::{Path, PathBuf};

/// The machine name `kkt setup` asks for on the tailnet.
pub(super) const DEFAULT_TS_HOSTNAME: &str = "kkt";
/// The admin name `kkt setup` gives a new admin key when not asked, at a
/// terminal (where it is offered) or without one. It becomes
/// `admin-supervisor`.
const DEFAULT_SETUP_ADMIN_NAME: &str = "supervisor";
/// Whether `kkt setup` makes an admin key when it is not asked, at a
/// terminal or without one.
const DEFAULT_SETUP_ADMIN_KEY: bool = false;
/// The zero-based mode option Enter accepts during interactive setup (Docker).
const DEFAULT_SETUP_MODE_INDEX: usize = 0;
/// Whether Enter allows agents in local containers during interactive setup.
const DEFAULT_SETUP_CONTAINERS: bool = true;
/// The name Docker gives containers for the machine they run on, which a
/// server set up for agents in local containers also answers to.
const SETUP_DOCKER_HOST: &str = "host.docker.internal";
/// Whether Enter tries a failed step again during interactive setup.
const DEFAULT_SETUP_TRY_AGAIN: bool = true;
/// Whether Enter continues an unfinished setup during interactive setup.
const DEFAULT_SETUP_CONTINUE: bool = true;
/// Whether Enter connects a client found during interactive setup.
const DEFAULT_SETUP_CONNECT_CLIENT: bool = true;
/// Whether `kkt setup` lets the clients it connects always use kkt's tools
/// and `kkt watch` when it is not asked, at a terminal or without one.
const DEFAULT_SETUP_ALLOW_KKT: bool = false;
/// Whether Enter shares the server over Tailscale during interactive setup.
const DEFAULT_SETUP_TAILSCALE: bool = false;
/// The client-only server address Enter accepts during interactive setup
/// when the machine has no setup record to take one from: the host and port
/// of a server on this machine.
pub(super) const DEFAULT_SETUP_URL: &str = "127.0.0.1:7420";
/// The client-only access key Enter accepts (no key).
pub(super) const DEFAULT_SETUP_KEY: &str = "";

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
    /// For a client: the server's MCP endpoint, as a full address, or as a
    /// host and port that setup completes ([`answered_address`]).
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

/// What `kkt setup` does to the machine. It includes what `kkt switch` does
/// ([`SwitchSteps`]) and how setup finds a server to adopt ([`AdoptSteps`]).
pub trait Steps: SwitchSteps + AdoptSteps {
    /// The notice to give before an install that reuses Docker data.
    fn docker_data_notice(&self) -> Option<String>;

    /// Installs and starts the server as a service that also answers to
    /// `allow_hosts`.
    fn install_service(
        &self,
        tailscale: bool,
        allow_hosts: &[String],
    ) -> Result<Installed, StepError>;
    /// Sets up and starts the server in Docker, shared over Tailscale with
    /// `tailscale` if given, and also answering to `allow_hosts`.
    fn install_docker(
        &self,
        tailscale: Option<&TailscaleLogin>,
        allow_hosts: &[String],
    ) -> Result<(Installed, DockerSetup), StepError>;
    /// Saves an access key typed in for a client-only machine where nothing
    /// uses it yet, readable only by its owner, and returns where.
    fn stage_key(&self, key: &str) -> Result<PathBuf, StepError>;
    /// Moves a staged key to where clients of the server at `url` keep it,
    /// and returns that path. Keys for other servers are left alone.
    fn keep_key(&self, staged: &Path, url: &str) -> Result<PathBuf, StepError>;
    /// Removes a staged key that won't be kept.
    fn discard_key(&self, staged: &Path) -> Result<(), StepError>;
    /// The clients installed on this machine.
    fn detect_clients(&self) -> Vec<Client>;
    /// Lets `client`, connected to kkt, always use kkt's tools and `kkt
    /// watch`, adding only those rules to its own files. Called only for a
    /// client that can ([`Client::can_allow_kkt`]). Returns what it did.
    fn allow_kkt(&self, client: Client) -> Result<String, StepError>;
    /// Checks the connection to the server `record`'s clients use. Returns
    /// the report, or the report as an error when it found problems.
    fn doctor(&self, record: &Record) -> Result<String, StepError>;
    /// The server an earlier setup installed but did not record, if one was
    /// noted and is still installed.
    fn unfinished(&self) -> Result<Option<Unfinished>, StepError>;
    /// Notes a server just installed, until the record is written.
    fn save_unfinished(&self, unfinished: &Unfinished) -> Result<(), StepError>;
    /// Removes the note, if there is one, once the record covers the server.
    fn forget_unfinished(&self) -> Result<(), StepError>;
    /// Gives the server installed as `record` an admin key named `name`,
    /// unless it has one, and restarts it so it serves `/mcp-admin`. Returns
    /// where this machine keeps the key.
    fn add_admin_key(&self, record: &Record, name: &AdminName) -> Result<PathBuf, StepError>;
    /// Reads the lane files in `folder` and checks every lane as `kkt lanes
    /// import` would on the `purpose` server set up here, with that server's
    /// lane text limit, changing nothing; returns how many lanes it has. An
    /// error says which file or lane is wrong.
    fn check_lanes(&self, folder: &Path, purpose: Purpose) -> Result<usize, StepError>;
    /// Imports the lane files in `folder` into the server installed as
    /// `record`, as `kkt lanes import` does. Returns its report.
    fn import_lanes(&self, record: &Record, folder: &Path) -> Result<String, StepError>;
    /// Checks, changing nothing, that `key`, typed in for a client and
    /// trimmed, is a valid access key. Returns why not as an error.
    fn check_typed_key(&self, key: &str) -> Result<(), StepError>;
    /// Whether there is no https at `url`, an https address: nothing listens
    /// there, or what answers does not speak TLS, as a plain-http server
    /// does. Any other failure, such as a certificate that is not trusted,
    /// is not an absence. Sends no key. An error says why it could not ask.
    fn https_absent(&self, url: &str) -> Result<bool, StepError>;
}

const MODE: Question = Question {
    text: "What is this machine for?",
    topic: "what this machine is for",
    flag: "--mode",
    default: None,
};
const TAILSCALE: Question = Question {
    text: "Share the server over Tailscale, so your other machines can reach it?",
    topic: "whether to share the server over Tailscale",
    flag: "--tailscale or --no-tailscale",
    default: Some(DEFAULT_SETUP_TAILSCALE),
};
const CONTAINERS: Question = Question {
    text: "Will agents in Docker containers on this machine use this server? They reach it as \
           host.docker.internal.",
    topic: "whether agents in Docker containers will use this server",
    flag: "--allow-host host.docker.internal, or --allow-host none",
    default: Some(DEFAULT_SETUP_CONTAINERS),
};
const TRY_AGAIN: Question = Question {
    text: "Try again?",
    topic: "whether to try again",
    flag: "",
    default: Some(DEFAULT_SETUP_TRY_AGAIN),
};
const CONTINUE: Question = Question {
    text: "Continue that setup?",
    topic: "whether to continue the unfinished setup",
    flag: "",
    default: Some(DEFAULT_SETUP_CONTINUE),
};
const ADMIN_KEY: Question = Question {
    text: "Make an admin key, so an agent holding it can supervise the server at /mcp-admin? \
           It sees every channel, closed ones included.",
    topic: "whether to make an admin key",
    flag: "--admin-key or --no-admin-key",
    default: Some(DEFAULT_SETUP_ADMIN_KEY),
};
const ADMIN_NAME: Question = Question {
    text: "The admin agent's name, such as sentinel (it becomes admin-sentinel). Its changes \
           are announced as \"operator (via admin-sentinel)\":",
    topic: "for the admin agent's name",
    flag: "--admin-name",
    default: None,
};
/// The flag that gives a folder of lanes. Setup never asks for one.
const LANES_FLAG: &str = "--lanes";
const ALLOW_KKT: Question = Question {
    text: "Allow the clients set up here to always use kkt's tools and `kkt watch`, without \
           asking each time?",
    topic: "whether clients may always use kkt",
    flag: "--allow-kkt or --no-allow-kkt",
    default: Some(DEFAULT_SETUP_ALLOW_KKT),
};
const URL: Question = Question {
    text: "The server's host and port, such as kkt.tail1234.ts.net or 127.0.0.1:7420 \
           (a full address such as https://kkt.tail1234.ts.net/mcp also works):",
    topic: "for the server's MCP address",
    flag: "--url",
    default: None,
};
const KEY: Question = Question {
    text: "The server's access key (run `kkt key show` there), or nothing if it has none:",
    topic: "for the server's access key",
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
///
/// `version` is the running kkt's, written into the record, and `mcp_path`
/// is the path of the server's MCP endpoint, which the web page's address
/// leaves off.
pub fn run(
    answers: &Answers,
    version: &str,
    mcp_path: &str,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, SetupError> {
    let offer = ask.interactive() && !answers.yes;
    let earlier = match steps.recorded() {
        Ok(record) => record,
        Err(e) => {
            ask.tell(&format!("{e}; setup replaces it."));
            None
        }
    };
    // The address a client setup offers: the server this machine's record
    // names, if it has one.
    let recorded_url = earlier.as_ref().map(|record| record.url.clone());
    let local = earlier.filter(Record::runs_server);
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
        None => match adoptable(answers, ask, steps).map_err(SetupError::Adopt)? {
            // Adopting installs nothing, an admin key included.
            Some(found) => (Plan::Adopt(found), None),
            None => match ask_plan(answers, recorded_url.as_deref(), mcp_path, ask, steps)? {
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
    if purpose == Some(Purpose::Docker)
        && !matches!(&plan, Plan::Resume(note) if note.record.is_some())
        && let Some(notice) = steps.docker_data_notice()
    {
        ask.tell(&notice);
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
                allow_kkt: resumed_allow_kkt(answers, &unfinished.clients, ask),
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
            .map_err(|e| SetupError::NotNoted(Box::new(e)))?;
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
            let record = finish(record, None, chosen, Finish::Adopted, offer, ask, steps)
                .map_err(|e| SetupError::Adopted(Box::new(e)))?;
            ask.tell(&set_up_line(&record, mcp_path));
            return import_lanes(record, lanes.as_deref(), offer, ask, steps);
        }
    };

    if let Some(name) = &admin
        && let Some(unfinished) = &unfinished
        && let Err(e) = retried(offer, ask, "Making the admin key", || {
            steps.add_admin_key(&record, name)
        })
    {
        return Err(SetupError::Unfinished {
            cause: Box::new(e),
            advice: unfinished_advice(unfinished),
        });
    }

    // A recorded noted server is installed, so its note goes once recorded.
    let kind = match (&unfinished, records_noted) {
        (Some(_), _) | (None, true) => Finish::Installed,
        (None, false) => Finish::Clients,
    };
    let finished = finish(record, staged.as_deref(), chosen, kind, offer, ask, steps);
    let record = finished.map_err(|e| match &unfinished {
        Some(unfinished) => SetupError::NotRecorded {
            cause: Box::new(e),
            advice: unfinished_advice(unfinished),
        },
        None => SetupError::FixAndRerun(Box::new(e)),
    })?;
    ask.tell(&set_up_line(&record, mcp_path));
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
) -> Result<Option<PathBuf>, SetupError> {
    let Some(Some(folder)) = &answers.lanes else {
        return Ok(None);
    };
    let purpose = match plan {
        Plan::Server(purpose, _, _) => *purpose,
        Plan::Resume(unfinished) => unfinished.purpose,
        Plan::Adopt(found) => found.purpose(),
        Plan::Client(_) => {
            return Err(SetupError::LanesForClient);
        }
    };
    let count = steps
        .check_lanes(folder, purpose)
        .map_err(|problem| SetupError::FlagFailed {
            flag: LANES_FLAG,
            problem,
        })?;
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
) -> Result<Record, SetupError> {
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
        Err(e) => Err(SetupError::LanesNotImported {
            cause: Box::new(e),
            folder: folder.to_path_buf(),
        }),
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
) -> Result<(Record, Unfinished), SetupError> {
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
        Err(e) => {
            return Err(SetupError::Unfinished {
                cause: Box::new(e),
                advice: unfinished_advice(&unfinished),
            });
        }
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
fn unfinished_advice(unfinished: &Unfinished) -> UnfinishedAdvice {
    UnfinishedAdvice {
        summary: unfinished.summary(),
        from: match unfinished.record {
            Some(_) => ResumeFrom::TheConnectionCheck,
            None => ResumeFrom::TheInstall,
        },
    }
}

/// The server an earlier setup installed, or began to, but did not record,
/// if setup should continue it: the answers given now agree with it, and at
/// a terminal the person agrees too.
fn resumable(
    answers: &Answers,
    ask: &dyn Ask,
    noted: Option<Unfinished>,
) -> Result<Option<Unfinished>, SetupError> {
    let Some(unfinished) = noted else {
        return Ok(None);
    };
    ask.tell(&unfinished.summary());
    // Continuing it would install, and --adopt installs nothing.
    if answers.adopt && unfinished.record.is_none() {
        return Err(SetupError::AdoptUnfinishedInstall);
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
    if ask.interactive() && !answers.yes && !ask.confirm(CONTINUE).map_err(SetupError::Ask)? {
        return Ok(None);
    }
    ask.tell(match unfinished.record {
        Some(_) => "Continuing from the connection check.",
        None => "Continuing from the install.",
    });
    Ok(Some(unfinished))
}

/// Asks the questions that decide what to set up.
/// `recorded_url` is the server address the machine's record names, which a
/// client setup offers; `mcp_path` completes one given as a host and port.
fn ask_plan(
    answers: &Answers,
    recorded_url: Option<&str>,
    mcp_path: &str,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Plan, SetupError> {
    let purpose = match answers.purpose {
        Some(purpose) => purpose,
        None => {
            let options = Purpose::ALL.map(Purpose::describe);
            Purpose::ALL[ask
                .choose(MODE, &options, DEFAULT_SETUP_MODE_INDEX)
                .map_err(SetupError::Ask)?]
        }
    };
    if purpose == Purpose::Client {
        return Ok(Plan::Client(client_answers(
            answers,
            recorded_url,
            mcp_path,
            ask,
            steps,
        )?));
    }
    let tailscale = match answers.tailscale {
        Some(answer) => answer,
        None => ask.confirm(TAILSCALE).map_err(SetupError::Ask)?,
    };
    let allow_hosts = match &answers.allow_hosts {
        Some(hosts) => hosts.clone(),
        None if ask.confirm(CONTAINERS).map_err(SetupError::Ask)? => {
            vec![SETUP_DOCKER_HOST.to_string()]
        }
        None => Vec::new(),
    };
    Ok(Plan::Server(purpose, tailscale, allow_hosts))
}

/// Whether to give the server an admin key, and its admin agent's name: as
/// `--admin-key` or `--no-admin-key` says, else asked at a terminal. Without
/// a terminal it needs no flag, so setups scripted before admin keys existed
/// run as they did.
fn admin_key(answers: &Answers, ask: &dyn Ask) -> Result<Option<AdminName>, SetupError> {
    let make = match answers.admin_key {
        Some(answer) => answer,
        None if ask.interactive() => ask.confirm(ADMIN_KEY).map_err(SetupError::Ask)?,
        None => DEFAULT_SETUP_ADMIN_KEY,
    };
    if !make {
        return Ok(None);
    }
    let given = match &answers.admin_name {
        Some(name) => name.clone(),
        None if ask.interactive() => ask
            .text(ADMIN_NAME, DEFAULT_SETUP_ADMIN_NAME)
            .map_err(SetupError::Ask)?,
        None => DEFAULT_SETUP_ADMIN_NAME.to_string(),
    };
    AdminName::new(&given, MAX_ADMIN_NAME_CHARS)
        .map(Some)
        .map_err(SetupError::AdminName)
}

/// The admin key to add when continuing a setup: only `--admin-key` adds
/// one, named by `--admin-name` or [`DEFAULT_SETUP_ADMIN_NAME`]. Continuing
/// asks nothing more.
fn resumed_admin_key(answers: &Answers) -> Result<Option<AdminName>, SetupError> {
    if !answers.admin_key.unwrap_or(DEFAULT_SETUP_ADMIN_KEY) {
        return Ok(None);
    }
    let given = answers
        .admin_name
        .as_deref()
        .unwrap_or(DEFAULT_SETUP_ADMIN_NAME);
    AdminName::new(given, MAX_ADMIN_NAME_CHARS)
        .map(Some)
        .map_err(SetupError::AdminName)
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
/// so (a failure there is only reported), and writes the record, unless it
/// was adopted and so is recorded already. An error says what failed, and
/// that nothing was recorded.
fn finish(
    mut record: Record,
    staged: Option<&Path>,
    chosen: Chosen,
    kind: Finish,
    offer: bool,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Record, SetupError> {
    let stopped = match kind {
        Finish::Adopted => Stopped::SetupStopped,
        Finish::Installed | Finish::Clients => Stopped::NothingRecorded,
    };
    match retried(offer, ask, "The connection check", || steps.doctor(&record)) {
        Ok(report) => ask.tell(&report),
        Err(report) => {
            // When it offered to try again, `retried` has shown it.
            if !offer {
                ask.tell(&report.to_string());
            }
            if let Some(staged) = staged
                && let Err(e) = steps.discard_key(staged)
            {
                ask.tell(&format!("Could not remove the unused key: {e}"));
            }
            return Err(SetupError::CheckFailed { stopped });
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
        return Err(SetupError::NotConnected {
            clients: failed.iter().map(|c| c.to_string()).collect(),
            stopped,
        });
    }
    if chosen.allow_kkt {
        allow_kkt(&chosen.clients, offer, ask, steps);
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
    Ok(record)
}

/// The line setup ends with: who uses which server, and the web page's
/// address, which is the server's address without `mcp_path`.
fn set_up_line(record: &Record, mcp_path: &str) -> String {
    let web = record.url.strip_suffix(mcp_path).unwrap_or(&record.url);
    match (&record.clients, record.local_server()) {
        (Some(clients), Some(local)) => format!(
            "This machine's clients use {}. {local}, at {}, with the web page at {web}/.",
            clients.url, record.url
        ),
        _ => format!(
            "This machine is set up as {}; clients use {}, and the web page is at {web}/.",
            record.mode.name(),
            record.url
        ),
    }
}

/// Lets each of `clients`, just connected, always use kkt's tools and `kkt
/// watch`. A client with no such setting is named and passed over. A
/// client whose settings cannot be changed is named with the reason and
/// how to try again, and setup goes on: the server and the connections
/// work without it.
fn allow_kkt(clients: &[Client], offer: bool, ask: &dyn Ask, steps: &dyn Steps) {
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
            // When it offered to try again, `retried` has shown the reason.
            Err(e) => ask.tell(&format!(
                "Could not allow kkt in {}{}. Setup goes on without it; \
                 `kkt setup --allow-kkt` tries again.",
                client.display_name(),
                if offer {
                    String::new()
                } else {
                    format!(": {e}")
                }
            )),
        }
    }
}

/// Refuses to set up a `purpose` server on a machine whose record says a
/// `local` server of the other kind runs here: it would keep running, but
/// kkt would no longer manage it.
fn keeps_local_server(local: &Record, purpose: Purpose) -> Result<(), SetupError> {
    let same = matches!(
        (&local.mode, purpose),
        (Mode::Service { .. }, Purpose::Service) | (Mode::Docker(_), Purpose::Docker)
    );
    if same {
        return Ok(());
    }
    Err(SetupError::KeepsLocalServer {
        server: local
            .local_server()
            .unwrap_or_else(|| "kkt runs here".to_string()),
        purpose,
    })
}

/// Refuses to set up a `purpose` server on a machine where an earlier setup
/// installed a server of the other kind, as `noted`, but did not record it:
/// as with a recorded one, it would keep running with nothing to manage it.
fn keeps_noted_server(noted: &Unfinished, purpose: Purpose) -> Result<(), SetupError> {
    if noted.purpose == purpose {
        return Ok(());
    }
    Err(SetupError::KeepsNotedServer {
        summary: noted.summary(),
        purpose,
        noted: noted.purpose,
    })
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
    mut step: impl FnMut() -> Result<T, StepError>,
) -> Result<T, SetupError> {
    loop {
        let problem = match step() {
            Ok(done) => return Ok(done),
            Err(problem) => problem,
        };
        if !offer {
            return Err(SetupError::Step(problem));
        }
        ask.tell(&format!("{what} did not work:\n{problem}"));
        match ask.confirm(TRY_AGAIN) {
            Ok(true) => {}
            Ok(false) => return Err(SetupError::Step(problem)),
            Err(ask) => return Err(SetupError::RetryUnanswered { problem, ask }),
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
/// checked the same way, and a problem with it ends setup. The address
/// offered is `recorded_url`, else [`DEFAULT_SETUP_URL`]; `mcp_path`
/// completes an address given as a host and port.
fn client_answers(
    answers: &Answers,
    recorded_url: Option<&str>,
    mcp_path: &str,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<ClientAnswers, SetupError> {
    let offered = recorded_url.unwrap_or(DEFAULT_SETUP_URL);
    loop {
        let url = checked_address(answers, offered, mcp_path, ask, steps)?;
        if let Some(key) = checked_key(&url, answers, ask, steps)? {
            return Ok(ClientAnswers { url, key });
        }
    }
}

/// The server's full address, once it passes its check. `offered` is what
/// Enter accepts, and `mcp_path` completes a host and port.
fn checked_address(
    answers: &Answers,
    offered: &str,
    mcp_path: &str,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<String, SetupError> {
    loop {
        let given = match &answers.url {
            Some(url) => url.trim().to_string(),
            None => ask
                .text(URL, offered)
                .map_err(SetupError::Ask)?
                .trim()
                .to_string(),
        };
        match answered_address(&given, mcp_path, steps) {
            Ok(url) => {
                if url != given {
                    ask.tell(&format!("Using {url}"));
                }
                return Ok(url);
            }
            Err(problem) if answers.url.is_some() => return Err(flag_failed(URL.flag, problem)),
            Err(problem) => ask.tell(&format!("That address did not work:\n{problem}")),
        }
    }
}

/// The full address a server address given to setup stands for, once a
/// server answers there.
///
/// An address with a scheme (`://`) is full, and stands for itself,
/// unchanged. Anything else is a host and port, such as
/// `kkt.tail1234.ts.net` or `127.0.0.1:7420`, with a path if one is given,
/// else `mcp_path`. It is tried over https. Only when there is no https
/// there at all ([`Steps::https_absent`]: nothing listens, or the server
/// does not speak TLS) and the host is on this machine
/// ([`on_this_machine`]) is it then tried over http. Any other https
/// failure, such as an untrusted certificate or a server that fails the
/// check, is the answer. Setup never chooses plain http for a server across
/// a network, where the key it checks next would cross in clear: a person
/// who means that gives the `http://` address.
///
/// Both addresses are checked by the address rule before any is tried or
/// said, so a refused one, such as one holding a password, is refused in
/// the rule's own words, which never repeat it.
fn answered_address(given: &str, mcp_path: &str, steps: &dyn Steps) -> Result<String, StepError> {
    if given.contains("://") {
        return steps.check_address(given).map(|()| given.to_string());
    }
    let path = match given.contains('/') {
        true => "",
        false => mcp_path,
    };
    let https = format!("https://{given}{path}");
    let http = format!("http://{given}{path}");
    steps.validate_address(&https)?;
    steps.validate_address(&http)?;
    let refused = match steps.check_address(&https) {
        Ok(()) => return Ok(https),
        Err(problem) => problem,
    };
    if !steps.https_absent(&https)? {
        return Err(refused);
    }
    if !on_this_machine(given) {
        return Err(StepError(Box::new(NoHttps {
            https,
            problem: refused,
            http,
        })));
    }
    match steps.check_address(&http) {
        Ok(()) => Ok(http),
        Err(problem) => Err(StepError(Box::new(NoneAnswered(vec![
            (https, refused),
            (http, problem),
        ])))),
    }
}

/// Whether the host of `given`, a host and port with any path, is this
/// machine: `localhost`, or a loopback address such as `127.0.0.1` or
/// `[::1]`. Only an address the rule has accepted is asked about.
fn on_this_machine(given: &str) -> bool {
    let authority = given.split('/').next().unwrap_or(given);
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(host, _port)| host),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// There is no https at a host on another machine, which setup does not try
/// over plain http: the address tried, the problem found there, and the
/// http address a person may give instead.
#[derive(Debug)]
struct NoHttps {
    https: String,
    problem: StepError,
    http: String,
}

impl std::fmt::Display for NoHttps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}\nFor a server on another machine, setup tries only https. If the \
             server uses plain http, give its full address, such as {}",
            self.https, self.problem, self.http
        )
    }
}

impl std::error::Error for NoHttps {}

/// No server answered at either address a host on this machine stands
/// for: each address, with the problem found there.
#[derive(Debug)]
struct NoneAnswered(Vec<(String, StepError)>);

impl std::fmt::Display for NoneAnswered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for (url, problem) in &self.0 {
            if !first {
                writeln!(f)?;
            }
            first = false;
            write!(f, "{url}: {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for NoneAnswered {}

/// The key, once the server at `url` accepts it. When a key fails, the
/// address is checked again, so a problem with the address is never taken
/// for one with the key: `None` means it no longer passes and must be asked
/// for again.
fn checked_key(
    url: &str,
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Option<ClientKey>, SetupError> {
    loop {
        let key = match &answers.key_file {
            Some(path) => ClientKey::File(path.clone()),
            None => match typed_key(
                &ask.text(KEY, DEFAULT_SETUP_KEY).map_err(SetupError::Ask)?,
                steps,
            ) {
                Ok(key) => key,
                Err(problem) => {
                    ask.tell(&problem.to_string());
                    continue;
                }
            },
        };
        let Err(problem) = steps.check_key(url, &key) else {
            return Ok(Some(key));
        };
        match steps.check_address(url) {
            Err(problem) if answers.url.is_some() => return Err(flag_failed(URL.flag, problem)),
            Err(problem) => {
                ask.tell(&format!("That address no longer works:\n{problem}"));
                return Ok(None);
            }
            Ok(()) if answers.key_file.is_some() => return Err(flag_failed(KEY.flag, problem)),
            Ok(()) => ask.tell(&format!("The server did not accept that key:\n{problem}")),
        }
    }
}

/// The key typed in answer to [`KEY`]: none when blank. A key the steps
/// find malformed is refused before any server is asked.
fn typed_key(text: &str, steps: &dyn Steps) -> Result<ClientKey, SetupError> {
    match text.trim() {
        "" => Ok(ClientKey::None),
        key => {
            steps.check_typed_key(key).map_err(SetupError::InvalidKey)?;
            Ok(ClientKey::Typed(key.to_string()))
        }
    }
}

/// The error for an answer given as `flag` that failed its check.
fn flag_failed(flag: &'static str, problem: StepError) -> SetupError {
    SetupError::FlagFailed { flag, problem }
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
fn choose(answers: &Answers, ask: &dyn Ask, steps: &dyn Steps) -> Result<Chosen, SetupError> {
    let clients = choose_clients(answers, ask, steps)?;
    let allow_kkt = match answers.allow_kkt {
        Some(answer) => answer,
        None if ask.interactive() && clients.iter().any(|c| c.can_allow_kkt()) => {
            ask.confirm(ALLOW_KKT).map_err(SetupError::Ask)?
        }
        None => DEFAULT_SETUP_ALLOW_KKT,
    };
    Ok(Chosen { clients, allow_kkt })
}

/// Whether to let `clients` always use kkt when continuing a setup:
/// continuing asks nothing more, so only `--allow-kkt` does. An answer
/// given to the earlier setup is not kept, so without either flag, when a
/// client could be allowed, setup says so and names the flag.
fn resumed_allow_kkt(answers: &Answers, clients: &[Client], ask: &dyn Ask) -> bool {
    if !clients.iter().any(|c| c.can_allow_kkt()) {
        return false;
    }
    match answers.allow_kkt {
        Some(answer) => answer,
        None => {
            ask.tell(
                "Continuing does not ask whether the clients may always use kkt's tools and \
                 `kkt watch`, so their permissions stay as they are; `kkt setup --allow-kkt` \
                 changes them.",
            );
            DEFAULT_SETUP_ALLOW_KKT
        }
    }
}

/// The clients to connect: those given with `--clients`, or each one found
/// that the person agrees to (all of them with `--yes`).
fn choose_clients(
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Vec<Client>, SetupError> {
    if let Some(chosen) = &answers.clients {
        return Ok(chosen.clone());
    }
    let mut chosen = Vec::new();
    for client in steps.detect_clients() {
        let question = Question {
            text: &format!("Connect {} to kkt?", client.display_name()),
            topic: &format!("whether to connect {} to kkt", client.display_name()),
            flag: "--clients",
            default: Some(DEFAULT_SETUP_CONNECT_CLIENT),
        };
        if answers.yes || ask.confirm(question).map_err(SetupError::Ask)? {
            chosen.push(client);
        }
    }
    Ok(chosen)
}
