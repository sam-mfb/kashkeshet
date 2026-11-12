//! The setup record, the note of an unfinished setup, and finding a server
//! kkt installed here with no record.

use crate::{Output, SetupFiles};
use kkt_machine::{Env, Platform};
use kkt_service::ServiceError;
use kkt_setup as setup;
use kkt_setup::connection::{self, ConnectionOptions, ConnectionSource};

/// Where `kkt setup` saves the Docker server's access key, in the config
/// directory.
pub const DOCKER_KEY_FILE_NAME: &str = "docker-access-key";

/// A server kkt installed here, in Docker or as a service, whether or not
/// it is recorded: for callers that found no record and no note of an
/// unfinished setup. An error names how the server it is about runs.
pub fn read_unrecorded<D: crate::ServiceDefaults>(
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<Option<setup::Unrecorded>, (setup::Purpose, String)> {
    let docker =
        unrecorded_docker(parts, env, platform).map_err(|e| (setup::Purpose::Docker, e))?;
    let service = unrecorded_service(parts).map_err(|e| (setup::Purpose::Service, e))?;
    setup::unrecorded_here(docker, service)
}

/// kkt in Docker in the folder `kkt setup` uses, with the access key's host
/// copy it saves.
fn unrecorded_docker<D>(
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<Option<setup::Unrecorded>, String> {
    let (Ok(folder), Some(config_dir)) = (
        setup::docker_folder(env, platform),
        kkt_config_file::config_dir(env.config_home().as_deref()),
    ) else {
        return Ok(None);
    };
    let found = |what: String| {
        format!(
            "Found a kkt server in Docker from {}, not recorded, but {what}.",
            folder.display()
        )
    };
    let existing =
        match kkt_container::existing(parts.container, &kkt_container::Folder(folder.clone())) {
            Ok(Some(existing)) => existing,
            Ok(None) => return Ok(None),
            Err(e) => return Err(found(e.to_string())),
        };
    let key_file = config_dir.join(DOCKER_KEY_FILE_NAME);
    if !parts.key_files.is_file(&key_file) {
        return Err(found(format!(
            "the copy of its access key setup saves, {}, is missing",
            key_file.display()
        )));
    }
    let (hostname, tailnet) = existing.node.clone().unzip();
    Ok(Some(setup::Unrecorded {
        url: docker_url(existing.port, existing.node.as_ref()),
        mode: setup::Mode::Docker(setup::DockerSetup {
            folder,
            tailscale: existing.tailscale,
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
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
) -> Result<Option<setup::Unrecorded>, String> {
    let service = parts.service;
    match service.ready() {
        Ok(()) => {}
        // No service can be installed here.
        Err(ServiceError::NoHome | ServiceError::Unsupported) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    if !service.state().map_err(|e| e.to_string())?.installed {
        return Ok(None);
    }
    let found = |what: String| setup::unreadable_service(&what);
    let args = service
        .installed_args()
        .map_err(|e| found(format!("its service file could not be read: {e}")))?
        .ok_or_else(|| found("its service file went away while it was read".into()))?;
    let unread = |e: String| found(format!("its settings could not be read: {e}"));
    let matches_defaults = parts
        .service_defaults
        .matches_defaults(&args)
        .map_err(unread)?;
    setup::service_runs_defaults(&args, matches_defaults)?;
    let connection = connection::serve_connection(parts.connection, ConnectionOptions::NONE, false)
        .map_err(|e| found(format!("its settings could not be read: {e}")))?;
    let bind = parts
        .serve
        .bind()
        .map_err(|e| found(format!("its settings could not be read: {e}")))?;
    let shared = parts.remote.served(bind).map_err(|e| {
        found(format!(
            "whether it is shared over Tailscale is not known: {e}"
        ))
    })?;
    Ok(Some(setup::unrecorded_service(
        shared.as_ref().map(|node| node.dns_name.as_str()),
        connection.url,
        connection.key_file,
    )))
}

/// The server a `kkt setup` installed here but did not record, if it noted
/// one and the install is still there: the managed Docker folder's settings
/// file, or the installed service.
pub fn read_unfinished(
    env: &Env,
    files: &dyn SetupFiles,
    container: &dyn kkt_container::Host,
    service: &dyn kkt_service::Service,
) -> Result<Option<setup::Unfinished>, String> {
    let Some(config_dir) = kkt_config_file::config_dir(env.config_home().as_deref()) else {
        return Ok(None);
    };
    let Some(unfinished) = files.read_unfinished(&config_dir)? else {
        return Ok(None);
    };
    let present = match unfinished.install_check() {
        setup::InstallCheck::Keep => true,
        setup::InstallCheck::DockerFolder(folder) => {
            container.is_file(&folder.join(kkt_container::ENV_FILE))
        }
        setup::InstallCheck::Service => {
            service
                .ready()
                .and_then(|()| service.state())
                .map_err(|e| e.to_string())?
                .installed
        }
        setup::InstallCheck::Gone => false,
    };
    Ok(present.then_some(unfinished))
}

pub use setup::{docker_folder, docker_url, unfinished_mode};

/// This machine's setup record, if `kkt setup` wrote one.
pub fn read_setup(env: &Env, files: &dyn SetupFiles) -> Result<Option<setup::Record>, String> {
    let Some(config_dir) = kkt_config_file::config_dir(env.config_home().as_deref()) else {
        return Ok(None);
    };
    files.read_record(&config_dir)
}

/// Refuses to start a server on a machine `kkt setup` set up otherwise.
pub fn check_setup(
    env: &Env,
    files: &dyn SetupFiles,
    service: &dyn kkt_service::Service,
    output: &dyn Output,
) -> Result<(), String> {
    let record = read_setup(env, files)?;
    let service_running = match record.as_ref().map(|r| &r.mode) {
        Some(setup::Mode::Service { .. }) => {
            let state = service.ready().and_then(|()| service.state());
            match state {
                Ok(state) => Some(state.running),
                Err(e) => {
                    // Not knowing is no reason to refuse: a server already
                    // on the port stops this one with its own message.
                    output.errors(format_args!(
                        "warning: could not tell whether the kkt service is running: {e}\n"
                    ));
                    None
                }
            }
        }
        _ => None,
    };
    setup::guard(record.as_ref(), service_running, false)
}

/// Resolve lifecycle commands through the setup record. An unrecorded
/// machine still supports the original local-database backup command.
pub fn write_setup_record(
    env: &Env,
    files: &dyn SetupFiles,
    record: &setup::Record,
) -> Result<(), String> {
    let config_dir = kkt_config_file::config_dir(env.config_home().as_deref())
        .ok_or("set HOME to keep the setup record")?;
    files.write_record(&config_dir, record)
}

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again.
pub fn recorded_setup<D: crate::ServiceDefaults>(
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<setup::Record, String> {
    setup::recorded_setup(
        read_setup(env, parts.setup_files)?,
        || read_unfinished(env, parts.setup_files, parts.container, parts.service),
        || read_unrecorded(parts, env, platform),
    )
}
