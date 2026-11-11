//! Adapting credential-file policy and warning destinations to the native
//! client, and its watch state file to the watch logic's Place.

use kkt_diagnostics as diagnostics;
use kkt_terminal_os::Terminal;
use kkt_watch as watch;
use std::path::{Path, PathBuf};

/// A reserved destination for an agent token or recovery code.
pub(crate) struct CredentialFile(kkt_agent_client::CredentialFile);

impl CredentialFile {
    pub fn create(path: PathBuf) -> Result<Self, String> {
        kkt_agent_client::CredentialFile::create(path, |path, e| {
            tracing::warn!(
                "could not remove incomplete credential file {}: {e}",
                path.display()
            );
        })
        .map(Self)
    }
    pub fn save(&mut self, secret: &str) -> Result<(), String> {
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

/// The agent's token, using setup's file rule and the original warning output.
pub(crate) fn watch_token(file: Option<&Path>) -> Result<String, String> {
    kkt_agent_client::watch_token(file, credential_check, private_warning)
}

/// The native state file supplied to watch's Place.
pub(crate) struct StateFile(kkt_agent_client::StateFile);
impl StateFile {
    pub(crate) fn claim(path: PathBuf) -> Result<Self, String> {
        kkt_agent_client::StateFile::claim(path).map(Self)
    }
}
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
