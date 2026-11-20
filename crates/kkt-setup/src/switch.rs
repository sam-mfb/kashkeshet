//! `kkt switch`: pointing this machine's clients at another server, or
//! back at the one that runs here, without the rest of `kkt setup`.

use super::ask::{Ask, Question};
use super::flow::{ClientKey, switch_note};
use super::record::{Record, Server};
use crate::{AskError, StepError};
use kkt_llm_agent_config::Client;
use std::path::PathBuf;

/// Why `kkt switch` changed nothing.
#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    /// There is no setup record to switch.
    #[error("there is no recorded local server for --local; run `kkt setup` first")]
    NotSetUp,
    /// The address or key path cannot be used in a client setup command.
    #[error(transparent)]
    InvalidArgument(StepError),
    /// A URL was supplied on a machine without a setup record.
    #[error("this machine has no setup record; run `{command}`")]
    ClientSetup { command: String },
    /// `--local` was given on a client machine.
    #[error(
        "--local is for a machine that runs a kkt server, and this one is a client of {url}; give the server's address instead"
    )]
    LocalOnClient { url: String },
    /// The new server did not pass its check.
    #[error("the server at {url} did not pass its check, so nothing was changed:\n{problem}")]
    CheckFailed {
        url: String,
        #[source]
        problem: StepError,
    },
    /// Some clients could not be pointed at it, so the record was kept.
    #[error(
        "could not connect {}, so the record still names {before}; fix that and run `kkt switch` again",
        clients.join(", ")
    )]
    NotConnected {
        clients: Vec<String>,
        before: String,
    },
    /// The setup record could not be read or written.
    #[error(transparent)]
    Record(StepError),
    /// The clients with a kkt entry could not be found.
    #[error(transparent)]
    Clients(StepError),
    /// The key file question got no answer.
    #[error(transparent)]
    Ask(AskError),
}

/// What `kkt switch` does to the machine: the part of [`super::Steps`] it
/// uses.
pub trait SwitchSteps {
    /// The clients whose config has a kkt entry now, whatever server it
    /// names.
    fn connected_clients(&self) -> Result<Vec<Client>, StepError>;
    /// The kkt server URL currently in `client`'s config, if known and safe
    /// to print (no credentials, query, fragment, or control characters).
    fn client_url(&self, client: Client) -> Result<Option<String>, StepError>;
    /// The setup record, if there is one.
    fn recorded(&self) -> Result<Option<Record>, StepError>;
    /// Writes the setup record, replacing any earlier one whole.
    fn write_record(&self, record: &Record) -> Result<(), StepError>;
    /// Points `client` at the server `record`'s clients use
    /// ([`Record::for_clients`]). Returns what it did.
    fn connect(&self, client: Client, record: &Record) -> Result<String, StepError>;
    /// Checks the address syntax without contacting the server.
    fn validate_address(&self, url: &str) -> Result<(), StepError>;
    /// Checks, changing nothing, that `url` is a server address a client
    /// may be given, then that a server answers there and accepts its host
    /// name. A refused address is refused before any server is asked.
    /// Returns the problems found as an error.
    fn check_address(&self, url: &str) -> Result<(), StepError>;
    /// Checks, changing nothing, that the server at `url` accepts `key`.
    /// Returns the problems found as an error.
    fn check_key(&self, url: &str, key: &ClientKey) -> Result<(), StepError>;
}

/// What to switch to, as given with `kkt switch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    pub to: Target,
    /// The new server's access key file (`--key-file`). Without it, a
    /// terminal is asked, offering the current one; with no terminal, the
    /// current one is used.
    pub key_file: Option<PathBuf>,
    /// The clients to point at it (`--clients`). Without it, every client
    /// with a kkt entry now.
    pub clients: Option<Vec<Client>>,
}

/// The server to switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The server at this MCP address.
    Url(String),
    /// The server that runs on this machine (`--local`).
    Local,
}

const KEY_FILE: Question = Question {
    text: "The new server's access key file (an absolute path, or none if it has no key):",
    topic: "for the new server's access key file",
    flag: "--key-file",
    default: None,
};

/// Switches this machine's clients to the server `how` names: checks it,
/// changing nothing if it fails; points each client at it; and records it.
/// On a machine that runs a server, that server stays recorded and is never
/// touched. Returns the record written.
pub fn switch(
    how: &Switch,
    version: &str,
    ask: &dyn Ask,
    steps: &dyn SwitchSteps,
) -> Result<Record, SwitchError> {
    let Some(record) = steps.recorded().map_err(SwitchError::Record)? else {
        let Target::Url(url) = &how.to else {
            return Err(SwitchError::NotSetUp);
        };
        let command = client_setup_command(url, how, steps)?;
        return Err(SwitchError::ClientSetup { command });
    };
    let before = record.for_clients();
    let server = match &how.to {
        Target::Local if !record.runs_server() => {
            return Err(SwitchError::LocalOnClient {
                url: record.url.clone(),
            });
        }
        Target::Local => record.own_server(),
        Target::Url(url) if record.runs_server() && url.trim() == record.url => {
            ask.tell("That is the server on this machine; its clients go back to it.");
            record.own_server()
        }
        Target::Url(url) => Server {
            url: url.trim().to_string(),
            key_file: key_file(how, &before, ask)?,
        },
    };

    let key = match &server.key_file {
        Some(path) => ClientKey::File(path.clone()),
        None => ClientKey::None,
    };
    steps
        .check_address(&server.url)
        .and_then(|()| steps.check_key(&server.url, &key))
        .map_err(|problem| SwitchError::CheckFailed {
            url: server.url.clone(),
            problem,
        })?;

    let chosen = match &how.clients {
        Some(chosen) => chosen.clone(),
        None => steps.connected_clients().map_err(SwitchError::Clients)?,
    };
    let switched = record.switched(server, version);
    if record.runs_server() && switched.clients.is_some() {
        ask.tell(&switch_note(&record, &switched.for_clients().url));
    }
    let mut failed = Vec::new();
    for client in &chosen {
        let previous = steps.client_url(*client).ok().flatten();
        match steps.connect(*client, &switched) {
            Ok(done) => match previous {
                Some(prev) => ask.tell(&with_previous(&done, &prev)),
                None => ask.tell(&done),
            },
            Err(e) => {
                ask.tell(&format!("Could not connect {}: {e}", client.display_name()));
                failed.push(client.display_name());
            }
        }
    }
    if !failed.is_empty() {
        return Err(SwitchError::NotConnected {
            clients: failed.iter().map(|c| c.to_string()).collect(),
            before: before.url.clone(),
        });
    }
    steps.write_record(&switched).map_err(SwitchError::Record)?;

    let after = switched.for_clients();
    let all_moved = how.clients.is_none()
        || steps
            .connected_clients()
            .ok()
            .is_some_and(|all| all.iter().all(|c| chosen.contains(c)));
    if chosen.is_empty() {
        if how.clients.is_none() {
            ask.tell(
                "No client here has kkt set up, so no client config was changed; \
                 `kkt connect <client> --write` adds one.",
            );
            ask.tell(&format!(
                "This machine's clients now use {}. `kkt watch`, `kkt register`, \
                 `kkt reclaim` and `kkt doctor` use it too.",
                after.url
            ));
        } else {
            ask.tell(&format!(
                "No client configs were changed. `kkt watch`, `kkt register`, \
                 `kkt reclaim` and `kkt doctor` now use {}.",
                after.url
            ));
        }
    } else if all_moved {
        ask.tell(&format!(
            "This machine's clients now use {}. `kkt watch`, `kkt register`, \
             `kkt reclaim` and `kkt doctor` use it too.",
            after.url
        ));
    } else {
        ask.tell(&format!(
            "{} now {} {}. `kkt watch`, `kkt register`, `kkt reclaim` and `kkt doctor` use it too.",
            join_clients(&chosen),
            if chosen.len() == 1 { "uses" } else { "use" },
            after.url
        ));
    }
    Ok(switched)
}

/// Joins client display names in plain English: `"Codex"`, `"Codex and OpenCode"`,
/// `"Claude Code, Codex and OpenCode"`.
fn join_clients(clients: &[Client]) -> String {
    let names: Vec<&str> = clients.iter().map(|c| c.display_name()).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// Places `(before: <previous>)` on the first line of `done` (the
/// `Configured: <path>` line), leaving any following lines (such as a key
/// permissions note) intact.
fn with_previous(done: &str, previous: &str) -> String {
    match done.split_once('\n') {
        Some((first, rest)) => format!("{first} (before: {previous})\n{rest}"),
        None => format!("{done} (before: {previous})"),
    }
}

/// The client setup command for a URL on a machine without a record.
fn client_setup_command(
    url: &str,
    how: &Switch,
    steps: &dyn SwitchSteps,
) -> Result<String, SwitchError> {
    steps
        .validate_address(url)
        .map_err(SwitchError::InvalidArgument)?;
    let quote = |word: &str| format!("'{}'", word.replace('\'', "'\\''"));
    let mut command = format!("kkt setup --mode client --url {}", quote(url.trim()));
    if let Some(path) = &how.key_file {
        let Some(path) = path.to_str() else {
            return Err(SwitchError::InvalidArgument(StepError(
                kkt_llm_agent_config::Text::KeyPath.into(),
            )));
        };
        command.push_str(&format!(" --key-file {}", quote(path)));
    }
    let clients = match &how.clients {
        Some(clients) => clients.clone(),
        None => steps.connected_clients().map_err(SwitchError::Clients)?,
    };
    let names = if clients.is_empty() {
        "none".to_owned()
    } else {
        clients
            .iter()
            .map(|c| c.name())
            .collect::<Vec<_>>()
            .join(",")
    };
    command.push_str(&format!(" --clients {names} --no-allow-kkt"));
    Ok(command)
}

/// The key file for a new server: `--key-file`, else asked at a terminal
/// offering the current one, else the current one.
fn key_file(how: &Switch, current: &Server, ask: &dyn Ask) -> Result<Option<PathBuf>, SwitchError> {
    if let Some(path) = &how.key_file {
        return Ok(Some(path.clone()));
    }
    if !ask.interactive() {
        return Ok(current.key_file.clone());
    }
    let offered = current
        .key_file
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    loop {
        let answer = ask.text(KEY_FILE, &offered).map_err(SwitchError::Ask)?;
        match answer.trim() {
            "" | "none" => return Ok(None),
            path if PathBuf::from(path).is_absolute() => return Ok(Some(PathBuf::from(path))),
            path => ask.tell(&format!("{path} is not an absolute path.")),
        }
    }
}
