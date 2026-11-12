//! kkt-agent-commands' `Credentials`, from the native client's credential
//! and state files, with setup's credential-file rule and the warnings on
//! standard error.

use kkt_agent_commands::{CredentialSlot, Credentials};
use kkt_diagnostics as diagnostics;
use kkt_terminal_os::Terminal;
use kkt_watch as watch;
use std::path::{Path, PathBuf};

/// Agent credential files and watch state files on this machine.
pub(crate) struct CredentialFiles;

impl Credentials for CredentialFiles {
    fn reserve(&self, path: PathBuf) -> Result<Box<dyn CredentialSlot>, String> {
        kkt_agent_client::CredentialFile::create(path, |path, e| {
            tracing::warn!(
                "could not remove incomplete credential file {}: {e}",
                path.display()
            );
        })
        .map(|file| Box::new(CredentialFile(file)) as Box<dyn CredentialSlot>)
    }

    fn token(&self, file: Option<&Path>) -> Result<String, String> {
        kkt_agent_client::watch_token(file, credential_check, private_warning)
    }

    fn claim_state(&self, path: PathBuf) -> Result<Box<dyn watch::Place>, String> {
        kkt_agent_client::StateFile::claim(path)
            .map(|file| Box::new(StateFile(file)) as Box<dyn watch::Place>)
    }
}

/// A reserved destination for an agent token or recovery code.
struct CredentialFile(kkt_agent_client::CredentialFile);

impl CredentialSlot for CredentialFile {
    fn save(&mut self, secret: &str) -> Result<(), String> {
        self.0.save(secret)
    }
}

fn credential_check(
    path: &Path,
    facts: kkt_agent_client::CredentialFileFacts,
) -> Result<Option<kkt_agent_client::PrivateMode>, String> {
    let quoted = diagnostics::shell_word(&path.to_string_lossy());
    kkt_setup::credential_file_check(path, &quoted, facts.owner, facts.mode, facts.caller_uid).map(
        |fix| {
            fix.map(|fix| kkt_agent_client::PrivateMode {
                mode: fix.mode,
                advice: fix.advice,
                warning: fix.warning,
            })
        },
    )
}

fn private_warning(text: &str) {
    Terminal.errors(format_args!("{text}\n"));
}

/// The native state file supplied to watch's Place.
struct StateFile(kkt_agent_client::StateFile);

impl watch::Place for StateFile {
    fn load(&self) -> Result<Option<i64>, String> {
        self.0.load()
    }
    fn save(&self, id: i64) -> Result<(), String> {
        self.0.save(id)
    }
}

#[cfg(test)]
mod agent_secret_tests;
