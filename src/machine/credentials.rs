//! kkt-agent-commands' `Credentials`, from the native client's credential
//! and state files, with setup's credential-file rule and the warnings on
//! standard error.

use kkt_agent_commands::{CredentialSlot, Credentials, CredentialsError};
use kkt_diagnostics as diagnostics;
use kkt_log::{Level, Log};
use kkt_log_tracing::TracingLog;
use kkt_terminal::Terminal;
use kkt_watch as watch;
use std::path::{Path, PathBuf};

/// Agent credential files and watch state files on this machine.
pub(crate) struct CredentialFiles;

impl Credentials for CredentialFiles {
    fn reserve(&self, path: PathBuf) -> Result<Box<dyn CredentialSlot>, CredentialsError> {
        kkt_agent_files::CredentialFile::create(path, |path, e| {
            TracingLog.record(
                Level::Warn,
                module_path!(),
                format_args!(
                    "could not remove incomplete credential file {}: {e}",
                    path.display()
                ),
                &[],
            );
        })
        .map(|file| Box::new(CredentialFile(file)) as Box<dyn CredentialSlot>)
        .map_err(|e| CredentialsError(Box::new(e)))
    }

    fn token(&self, file: Option<&Path>) -> Result<String, CredentialsError> {
        kkt_agent_files::watch_token(file, credential_check, private_warning)
            .map_err(|e| CredentialsError(Box::new(e)))
    }

    fn claim_state(&self, path: PathBuf) -> Result<Box<dyn watch::Place>, CredentialsError> {
        kkt_agent_files::StateFile::claim(path)
            .map(|file| Box::new(StateFile(file)) as Box<dyn watch::Place>)
            .map_err(|e| CredentialsError(Box::new(e)))
    }
}

/// A reserved destination for an agent token or recovery code.
struct CredentialFile(kkt_agent_files::CredentialFile);

impl CredentialSlot for CredentialFile {
    fn save(&mut self, secret: &str) -> Result<(), CredentialsError> {
        self.0
            .save(secret)
            .map_err(|e| CredentialsError(Box::new(e)))
    }
}

fn credential_check(
    path: &Path,
    facts: kkt_agent_files::CredentialFileFacts,
) -> Result<Option<kkt_agent_files::PrivateMode>, Box<dyn std::error::Error + Send + Sync>> {
    let quoted = diagnostics::shell_word(&path.to_string_lossy());
    kkt_agent_commands::credential_file_check(
        path,
        &quoted,
        facts.owner,
        facts.mode,
        facts.caller_uid,
    )
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    .map(|fix| {
        fix.map(|fix| kkt_agent_files::PrivateMode {
            mode: fix.mode,
            advice: fix.advice,
            warning: fix.warning,
        })
    })
}

fn private_warning(text: &str) {
    Terminal.errors(format_args!("{text}\n"));
}

/// The native state file supplied to watch's Place.
struct StateFile(kkt_agent_files::StateFile);

impl watch::Place for StateFile {
    fn load(&self) -> Result<Option<i64>, watch::PlaceError> {
        self.0.load().map_err(|e| watch::PlaceError(Box::new(e)))
    }
    fn save(&self, id: i64) -> Result<(), watch::PlaceError> {
        self.0.save(id).map_err(|e| watch::PlaceError(Box::new(e)))
    }
}

#[cfg(test)]
mod agent_secret_tests;
