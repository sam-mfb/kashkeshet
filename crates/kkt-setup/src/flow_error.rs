//! Why `kkt setup` stopped, worded for the person who ran it.

use crate::{AdoptError, AskError, Purpose, StepError};
use kkt_names::NameError;
use std::path::PathBuf;

/// What setup says it stopped short of, when the connection check or a
/// client failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// Nothing was recorded.
    NothingRecorded,
    /// An adopted server is recorded already; setup stopped there.
    SetupStopped,
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Stopped::NothingRecorded => "so nothing was recorded",
            Stopped::SetupStopped => "so setup stopped there",
        })
    }
}

/// Where a setup that did not finish continues from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeFrom {
    TheInstall,
    TheConnectionCheck,
}

/// What a person is told of a server installed, or begun, but not recorded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{summary} Until then, `kkt upgrade` and `kkt uninstall` do not manage it. Run `kkt setup` again to finish: it continues from {}.",
    match from {
        ResumeFrom::TheInstall => "the install",
        ResumeFrom::TheConnectionCheck => "the connection check",
    }
)]
pub struct UnfinishedAdvice {
    /// What was installed, or began to be.
    pub summary: String,
    pub from: ResumeFrom,
}

/// Why `kkt setup` stopped.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// A step failed, in its own words.
    #[error(transparent)]
    Step(StepError),
    /// A step failed, and the question whether to try it again got no
    /// answer.
    #[error("{problem}\n{ask}")]
    RetryUnanswered { problem: StepError, ask: AskError },
    /// The install could not be noted before it started.
    #[error("{0}\nsetup notes the install before starting it, so nothing was changed")]
    NotNoted(Box<SetupError>),
    /// The install, or the admin key, failed with a server noted.
    #[error("{cause}\n{advice}")]
    Unfinished {
        cause: Box<SetupError>,
        advice: UnfinishedAdvice,
    },
    /// What came after the install failed with a server noted.
    #[error("{cause}.\n{advice}")]
    NotRecorded {
        cause: Box<SetupError>,
        advice: UnfinishedAdvice,
    },
    /// What came after failed, and nothing was installed or noted.
    #[error("{0}; fix that and run `kkt setup` again")]
    FixAndRerun(Box<SetupError>),
    /// What came after adopting a server failed; it is recorded.
    #[error(
        "{0}.\nThe server is recorded, so `kkt status` and `kkt upgrade` manage it. For a server set up by an older kkt, `kkt upgrade` may fix the problem; then connect clients with `kkt connect <client> --write`."
    )]
    Adopted(Box<SetupError>),
    /// The setup is recorded, but the lanes were not imported.
    #[error(
        "{cause}\nThe setup is recorded, but the lanes were not imported; import them with `kkt lanes import {}`",
        folder.display()
    )]
    LanesNotImported {
        cause: Box<SetupError>,
        folder: PathBuf,
    },
    /// `--lanes` was given for a client.
    #[error("--lanes is for a server: run `kkt lanes import` on the machine where it runs")]
    LanesForClient,
    /// `--adopt` was given to continue a setup that had not installed.
    #[error(
        "--adopt installs nothing, but this setup had not finished installing its server; run `kkt setup` without --adopt to continue it"
    )]
    AdoptUnfinishedInstall,
    /// The connection check found problems.
    #[error("the connection check found problems (see the fixes above), {stopped}")]
    CheckFailed { stopped: Stopped },
    /// Some clients could not be connected.
    #[error("could not connect {}, {stopped}", clients.join(", "))]
    NotConnected {
        clients: Vec<String>,
        stopped: Stopped,
    },
    /// Setting up this kind of server would leave the recorded one running
    /// with nothing to manage it.
    #[error(
        "{server} (see `kkt status`). Setting kkt up {} instead would leave that server running with nothing to manage it, so nothing was changed. Run `kkt uninstall` first to remove it, or `kkt switch <url>` to point only this machine's clients at another server",
        how(*purpose)
    )]
    KeepsLocalServer { server: String, purpose: Purpose },
    /// Setting up this kind of server would leave the noted one running with
    /// nothing to manage it.
    #[error(
        "{summary} Setting kkt up {} instead would leave that server running with nothing to manage it, so nothing was changed. Run `kkt setup --mode {}` to finish that setup first; then `kkt uninstall` removes it, or `kkt switch <url>` points only this machine's clients at another server",
        how(*purpose),
        noted.name()
    )]
    KeepsNotedServer {
        summary: String,
        purpose: Purpose,
        noted: Purpose,
    },
    /// An answer given as a flag did not pass its check.
    #[error(
        "the answer given with {flag} did not pass its check, so nothing was changed:\n{problem}"
    )]
    FlagFailed {
        flag: &'static str,
        problem: StepError,
    },
    /// A typed key is not a valid access key.
    #[error("that key is not valid: {0}")]
    InvalidKey(StepError),
    /// The admin agent's name breaks its rules.
    #[error(transparent)]
    AdminName(NameError),
    /// A question got no answer.
    #[error(transparent)]
    Ask(AskError),
    /// The server found could not be adopted.
    #[error(transparent)]
    Adopt(AdoptError),
}

/// How a server of `purpose` is set up, as a sentence says it.
fn how(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Docker => "in Docker",
        _ => "as a service",
    }
}
