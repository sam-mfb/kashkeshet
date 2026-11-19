//! The setup record, the note of an unfinished setup, and finding a server
//! kkt installed here with no record.

use crate::{Output, PartError, SetupFiles};
use kkt_bare_service::ServiceError;
use kkt_containerized_service::ContainerError;
use kkt_deployment::connection::{self, ConnectionError, ConnectionOptions, ConnectionSource};
use kkt_machine::{Env, Platform};
use kkt_remote_access::SharingError;
use kkt_setup as setup;
use std::path::PathBuf;

/// Why a server kkt installed here with no record cannot be made out.
#[derive(Debug, thiserror::Error)]
pub enum UnrecordedError {
    /// kkt in Docker in the folder setup uses could not be looked at.
    #[error(
        "Found a kkt server in Docker from {}, not recorded, but {source}.",
        folder.display()
    )]
    DockerUnreadable {
        folder: PathBuf,
        #[source]
        source: Box<ContainerError>,
    },
    /// The copy of its access key that setup saves is not there.
    #[error(
        "Found a kkt server in Docker from {}, not recorded, but the copy of its access key \
         setup saves, {}, is missing.",
        folder.display(),
        key_file.display()
    )]
    KeyCopyMissing { folder: PathBuf, key_file: PathBuf },
    /// Whether a kkt service is installed could not be told.
    #[error(transparent)]
    ServiceState(ServiceError),
    #[error(
        "{}",
        setup::unreadable_service(&format!("its service file could not be read: {}", .0))
    )]
    ServiceFile(#[source] ServiceError),
    #[error(
        "{}",
        setup::unreadable_service("its service file went away while it was read")
    )]
    ServiceFileGone,
    #[error(
        "{}",
        setup::unreadable_service(&format!("its settings could not be read: {}", .0))
    )]
    Settings(#[source] SettingsUnread),
    #[error(
        "{}",
        setup::unreadable_service(&format!(
            "whether it is shared over Tailscale is not known: {}",
            .0
        ))
    )]
    Sharing(#[source] SharingError),
    /// kkt-setup's refusal of a service that does not run with the
    /// default config file's settings.
    #[error(transparent)]
    NotDefaults(#[from] setup::NotDefaults),
    /// kkt-setup's refusal when a server in Docker and a service are both
    /// installed.
    #[error(transparent)]
    BothInstalled(setup::BothInstalled),
}

/// Which of an installed service's settings could not be read.
#[derive(Debug, thiserror::Error)]
pub enum SettingsUnread {
    /// Whether it runs with the default config file's settings.
    #[error(transparent)]
    Defaults(PartError),
    /// How a client reaches it.
    #[error(transparent)]
    Connection(ConnectionError),
    /// The address it listens on.
    #[error(transparent)]
    Bind(PartError),
}

/// Why the setup record or the note of an unfinished setup could not be
/// read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Files(PartError),
    /// Whether the service an unfinished setup noted is installed could not
    /// be told.
    #[error(transparent)]
    ServiceState(ServiceError),
}

/// Why the setup record could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("set HOME to keep the setup record")]
    NoHome,
    #[error(transparent)]
    Files(PartError),
}

/// Why `kkt serve` may not start here, or that it could not tell.
#[derive(Debug, thiserror::Error)]
pub enum ServeCheckError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Guard(#[from] kkt_deployment::GuardError),
}

/// Why there is no setup record to act on.
#[derive(Debug, thiserror::Error)]
pub enum RecordedError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    NoRecord(#[from] setup::NoRecord),
}

/// Where `kkt setup` saves the Docker server's access key, in the config
/// directory.
pub const DOCKER_KEY_FILE_NAME: &str = "docker-access-key";

/// A server kkt installed here, in Docker or as a service, whether or not
/// it is recorded: for callers that found no record and no note of an
/// unfinished setup. An error names how the server it is about runs.
pub fn read_unrecorded<D: crate::ServiceDefaults>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<Option<setup::Unrecorded>, (setup::Purpose, UnrecordedError)> {
    let docker =
        unrecorded_docker(parts, env, platform).map_err(|e| (setup::Purpose::Docker, e))?;
    let service = unrecorded_service(parts).map_err(|e| (setup::Purpose::Service, e))?;
    setup::unrecorded_here(docker, service)
        .map_err(|(purpose, e)| (purpose, UnrecordedError::BothInstalled(e)))
}

/// kkt in Docker in the folder `kkt setup` uses, with the access key's host
/// copy it saves.
fn unrecorded_docker<D>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<Option<setup::Unrecorded>, UnrecordedError> {
    let (Ok(folder), Some(config_dir)) = (docker_folder(env, platform), parts.serve.config_dir())
    else {
        return Ok(None);
    };
    let existing = match parts
        .container
        .existing(&kkt_container::Folder(folder.clone()))
    {
        Ok(Some(existing)) => existing,
        Ok(None) => return Ok(None),
        Err(source) => {
            return Err(UnrecordedError::DockerUnreadable {
                folder,
                source: Box::new(source),
            });
        }
    };
    let key_file = config_dir.join(DOCKER_KEY_FILE_NAME);
    if !parts.key_files.is_file(&key_file) {
        return Err(UnrecordedError::KeyCopyMissing { folder, key_file });
    }
    let (hostname, tailnet) = existing.node.clone().unzip();
    Ok(Some(setup::Unrecorded {
        url: docker_url(existing.port, existing.node.as_ref(), parts.mcp_path),
        mode: setup::Mode::Docker(setup::DockerSetup {
            folder,
            tailscale: existing.sharing,
            hostname,
            tailnet,
        }),
        key_file: Some(key_file),
    }))
}

/// The installed kkt service, reached as the config file says, or over
/// Tailscale if Tailscale Serve proxies to it. Its service file must run
/// `kkt serve` with the settings `kkt setup` installs and `kkt upgrade`
/// installs again (the default config file's), so that clients are told
/// the address it really has and an upgrade keeps its settings.
fn unrecorded_service<D: crate::ServiceDefaults>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
) -> Result<Option<setup::Unrecorded>, UnrecordedError> {
    let service = parts.service;
    match service.ready() {
        Ok(()) => {}
        // No service can be installed here.
        Err(ServiceError::NoHome | ServiceError::Unsupported(_)) => return Ok(None),
        Err(e) => return Err(UnrecordedError::ServiceState(e)),
    }
    if !service
        .state()
        .map_err(UnrecordedError::ServiceState)?
        .installed
    {
        return Ok(None);
    }
    let args = service
        .installed_args()
        .map_err(UnrecordedError::ServiceFile)?
        .ok_or(UnrecordedError::ServiceFileGone)?;
    let unread = |e| UnrecordedError::Settings(e);
    let matches_defaults = parts
        .service_defaults
        .matches_defaults(&args)
        .map_err(|e| unread(SettingsUnread::Defaults(e)))?;
    setup::service_runs_defaults(&args, matches_defaults)?;
    let connection = connection::serve_connection(parts.connection, ConnectionOptions::NONE, false)
        .map_err(|e| unread(SettingsUnread::Connection(e)))?;
    let bind = parts
        .serve
        .bind()
        .map_err(|e| unread(SettingsUnread::Bind(e)))?;
    let shared = parts
        .remote
        .served(bind)
        .map_err(UnrecordedError::Sharing)?;
    Ok(Some(setup::unrecorded_service(
        shared.as_ref().map(|node| node.dns_name.as_str()),
        connection.url,
        connection.key_file,
        parts.mcp_path,
    )))
}

/// What is installed on this machine, read through the install's parts: the
/// setup record, the note of an unfinished setup whose install is still
/// here, and a server installed with no record. Only parts that can ask
/// what an installed service runs with (`D: ServiceDefaults`) can read it.
pub struct InstallReads<'a, 'p, C: ConnectionSource, D> {
    parts: &'a crate::InstallParts<'p, C, D>,
    env: &'a Env,
    platform: Platform,
}

impl<'a, 'p, C: ConnectionSource, D> InstallReads<'a, 'p, C, D> {
    pub fn new(parts: &'a crate::InstallParts<'p, C, D>, env: &'a Env, platform: Platform) -> Self {
        Self {
            parts,
            env,
            platform,
        }
    }
}

/// The three reads of what is installed here. Each is made only when
/// asked, so a caller that stops at the record reads nothing more.
impl<C: ConnectionSource, D: crate::ServiceDefaults> InstallReads<'_, '_, C, D> {
    /// This machine's setup record, if `kkt setup` wrote one.
    pub fn record(&self) -> Result<Option<setup::Record>, ReadError> {
        read_setup(self.parts.serve, self.parts.setup_files)
    }
    /// The note of a setup not finished, if the install it noted is still
    /// here.
    pub fn unfinished(&self) -> Result<Option<setup::Unfinished>, ReadError> {
        read_unfinished(
            self.parts.serve,
            self.parts.setup_files,
            self.parts.container,
            self.parts.service,
        )
    }
    /// A server kkt installed here, in Docker or as a service, with no
    /// record. An error says how the server it is about runs, and why it
    /// cannot be made out.
    pub fn unrecorded(
        &self,
    ) -> Result<Option<setup::Unrecorded>, (setup::Purpose, UnrecordedError)> {
        read_unrecorded(self.parts, self.env, self.platform)
    }
}

/// The server a `kkt setup` installed here but did not record, if it noted
/// one and the install is still there: the managed Docker folder's settings
/// file, or the installed service.
pub fn read_unfinished(
    serve: &dyn crate::ServeFacts,
    files: &dyn SetupFiles,
    container: &dyn kkt_containerized_service::ServiceContainer,
    service: &dyn kkt_bare_service::Service,
) -> Result<Option<setup::Unfinished>, ReadError> {
    let Some(config_dir) = serve.config_dir() else {
        return Ok(None);
    };
    let Some(unfinished) = files
        .read_unfinished(&config_dir)
        .map_err(ReadError::Files)?
    else {
        return Ok(None);
    };
    let present = match unfinished.install_check() {
        setup::InstallCheck::Keep => true,
        setup::InstallCheck::DockerFolder(folder) => {
            container.settings_file(&kkt_container::Folder(folder))
        }
        setup::InstallCheck::Service => {
            service
                .ready()
                .and_then(|()| service.state())
                .map_err(ReadError::ServiceState)?
                .installed
        }
        setup::InstallCheck::Gone => false,
    };
    Ok(present.then_some(unfinished))
}

pub use kkt_deployment::{docker_folder, docker_url};

/// The mode of the server `unfinished` notes: from its record, or, for an
/// install that did not finish, where that install puts the server, in the
/// folder `kkt setup` keeps kkt's Docker files in.
pub fn unfinished_mode(
    env: &Env,
    platform: Platform,
    unfinished: &setup::Unfinished,
) -> Result<setup::Mode, kkt_deployment::NoDockerHome> {
    unfinished.mode(docker_folder(env, platform))
}

/// This machine's setup record, if `kkt setup` wrote one.
pub fn read_setup(
    serve: &dyn crate::ServeFacts,
    files: &dyn SetupFiles,
) -> Result<Option<setup::Record>, ReadError> {
    let Some(config_dir) = serve.config_dir() else {
        return Ok(None);
    };
    files.read_record(&config_dir).map_err(ReadError::Files)
}

/// Refuses to start a server on a machine `kkt setup` set up otherwise.
pub fn check_setup(
    serve: &dyn crate::ServeFacts,
    files: &dyn SetupFiles,
    service: &dyn kkt_bare_service::Service,
    output: &dyn Output,
) -> Result<(), ServeCheckError> {
    let record = read_setup(serve, files)?;
    let service_running = match record.as_ref().map(|r| &r.mode) {
        Some(setup::Mode::Service { .. }) => {
            let state = service.ready().and_then(|()| service.state());
            match state {
                Ok(state) => Some(state.running),
                Err(e) => {
                    // Not knowing is no reason to refuse: a server already
                    // on the port stops this one with its own message.
                    output.errors(format_args!("{}\n", state_unknown_warning(&e)));
                    None
                }
            }
        }
        _ => None,
    };
    kkt_deployment::guard(
        record.as_ref().map(crate::deployment::recorded).as_ref(),
        service_running,
        false,
    )?;
    Ok(())
}

/// The warning `kkt serve` gives when it cannot tell whether the kkt
/// service is running.
fn state_unknown_warning(error: &ServiceError) -> String {
    format!(
        "warning: could not tell whether the kkt service is running: {}",
        why_state_unknown(error)
    )
}

/// Why the service's state could not be read, for a place where the person
/// asked for no service command (`kkt serve`'s check, `kkt doctor`): a
/// missing desktop login session is said as a fact, without the advice
/// that a service command's refusal gives.
pub(crate) fn why_state_unknown(error: &ServiceError) -> String {
    match error {
        ServiceError::NoLoginSession { fact, .. } => fact.clone(),
        other => other.to_string(),
    }
}

/// Resolve lifecycle commands through the setup record. An unrecorded
/// machine still supports the original local-database backup command.
pub fn write_setup_record(
    serve: &dyn crate::ServeFacts,
    files: &dyn SetupFiles,
    record: &setup::Record,
) -> Result<(), WriteError> {
    let config_dir = serve.config_dir().ok_or(WriteError::NoHome)?;
    files
        .write_record(&config_dir, record)
        .map_err(WriteError::Files)
}

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again.
pub fn recorded_setup<D: crate::ServiceDefaults>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<setup::Record, RecordedError> {
    Ok(setup::recorded_setup(
        read_setup(parts.serve, parts.setup_files)?,
        || {
            read_unfinished(
                parts.serve,
                parts.setup_files,
                parts.container,
                parts.service,
            )
        },
        || read_unrecorded(parts, env, platform),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_login_session_serve_warns_with_the_fact_only() {
        let error = ServiceError::NoLoginSession {
            refusal: "log in at the screen, or run `kkt serve` yourself".to_string(),
            fact: "no one is logged in at the screen".to_string(),
        };
        // The advice belongs to the service commands' own refusal.
        assert_eq!(
            state_unknown_warning(&error),
            "warning: could not tell whether the kkt service is running: no one is logged in \
             at the screen"
        );
    }

    #[test]
    fn other_failures_are_warned_as_the_service_says_them() {
        let error = ServiceError::Failed {
            command: "launchctl print gui/501/x".to_string(),
            detail: "Input/output error".to_string(),
        };
        assert_eq!(
            state_unknown_warning(&error),
            format!("warning: could not tell whether the kkt service is running: {error}")
        );
    }
}
