//! A server kkt installed on this machine with no setup record, as an older
//! kkt can leave one: before 0.5.0, setup wrote the record only at the end,
//! so a setup that stopped after installing left the server running and
//! unrecorded. Setup records such a server as it is ("adopts" it), without
//! installing it again, once the person agrees.

use super::ask::{Ask, Question};
use super::flow::{Answers, Purpose, Steps};
use super::record::{Mode, Record};
use crate::config::SETUP_UNKNOWN_VERSION;
use std::path::PathBuf;

/// A server kkt installed here that neither a record nor a note of an
/// unfinished setup covers. Everything in it was read from the install
/// itself, never guessed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unrecorded {
    /// How the server runs: [`Mode::Docker`] or [`Mode::Service`].
    pub mode: Mode,
    /// The MCP endpoint its clients use.
    pub url: String,
    /// The access key its clients send, if it has one.
    pub key_file: Option<PathBuf>,
}

impl Unrecorded {
    /// One sentence saying what was found, for setup, `kkt status` and
    /// `kkt doctor`.
    pub fn summary(&self) -> String {
        let how = match &self.mode {
            Mode::Docker(docker) => format!("in Docker from {}", docker.folder.display()),
            _ => "as a service".to_string(),
        };
        format!(
            "Found a kkt server {how}, for clients at {}, not recorded.",
            self.url
        )
    }

    /// The record that adopts it, written by kkt `version`. A Docker
    /// install's version is in its settings file, but nothing says which
    /// version a service runs, so its record says it is not known, and
    /// `kkt upgrade` installs the service again.
    pub fn record(&self, version: &str) -> Record {
        let written_by = match self.mode {
            Mode::Service { .. } => SETUP_UNKNOWN_VERSION,
            _ => version,
        };
        Record {
            mode: self.mode.clone(),
            written_by: written_by.to_string(),
            url: self.url.clone(),
            key_file: self.key_file.clone(),
        }
    }

    fn purpose(&self) -> Purpose {
        match self.mode {
            Mode::Docker(_) => Purpose::Docker,
            _ => Purpose::Service,
        }
    }

    fn tailscale(&self) -> bool {
        match &self.mode {
            Mode::Docker(docker) => docker.tailscale,
            Mode::Service { tailscale } => *tailscale,
            Mode::Client => false,
        }
    }
}

/// The server to adopt, if there is one and adopting is wanted: with
/// `--adopt`, or at a terminal when the person agrees. Without a terminal and
/// without `--adopt`, it is only named. With `--adopt`, finding nothing to
/// adopt, or answers that disagree with what was found, is an error.
pub(super) fn adoptable(
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn Steps,
) -> Result<Option<Unrecorded>, String> {
    let found = match steps.unrecorded() {
        Ok(Some(found)) => found,
        Ok(None) if answers.adopt => {
            return Err("--adopt: no kkt server without a setup record was found here".into());
        }
        Ok(None) => return Ok(None),
        Err(e) if answers.adopt => return Err(format!("--adopt: {e}")),
        Err(e) => {
            ask.tell(&format!("{e}\nSo it cannot be recorded as it is."));
            return Ok(None);
        }
    };
    let differs = answers.purpose.is_some_and(|p| p != found.purpose())
        || answers.tailscale.is_some_and(|t| t != found.tailscale());
    if differs {
        if answers.adopt {
            return Err(format!(
                "--adopt: {} The answers given now are different.",
                found.summary()
            ));
        }
        ask.tell(&format!(
            "{} The answers given now are different, so setup does not record it.",
            found.summary()
        ));
        return Ok(None);
    }
    if answers.adopt {
        ask.tell(&found.summary());
        return Ok(Some(found));
    }
    if !ask.interactive() {
        ask.tell(&format!(
            "{} To record it instead of setting this machine up anew, run `kkt setup --adopt`.",
            found.summary()
        ));
        return Ok(None);
    }
    let question = Question {
        text: &format!("{} Record it and continue?", found.summary()),
        flag: "--adopt",
        default: Some(true),
    };
    Ok(ask.confirm(question)?.then_some(found))
}
