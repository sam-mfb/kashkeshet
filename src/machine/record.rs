//! The setup record, the note of an unfinished setup, and finding a server
//! kkt installed here with no record.

use crate::machine::connection::{ConnectionOptions, serve_connection};
use crate::machine::container::ContainerHost;
use crate::machine::remote_access::RemoteHost;
use crate::machine::service::ServiceSystem;
use crate::machine::settings::{
    ServeArgs, as_running, serve_bind, serve_settings, serve_settings_from_args,
};
use crate::process::write_whole;
use kkt_machine::{Env, Platform};
use kkt_service::{Manager, ServiceError};
use kkt_settings::ServeOptions;
use kkt_setup as setup;
use std::path::PathBuf;

/// The folder, in the per-user data directory, that `kkt setup` keeps
/// the Docker install's files in.
const DOCKER_FOLDER: &str = "docker";
/// Where `kkt setup` saves the Docker server's access key, in the config
/// directory.
pub(crate) const DOCKER_KEY_FILE_NAME: &str = "docker-access-key";

/// The MCP endpoint of kkt in Docker, published on the host's `port`, or
/// served over Tailscale as `node`, `(hostname, tailnet)`.
pub fn docker_url(port: u16, node: Option<&(String, String)>) -> String {
    match node {
        Some((hostname, tailnet)) => format!("https://{hostname}.{tailnet}/mcp"),
        None => format!("http://127.0.0.1:{port}/mcp"),
    }
}

/// A server kkt installed here, in Docker or as a service, whether or not
/// it is recorded: for callers that found no record and no note of an
/// unfinished setup. An error names how the server it is about runs.
pub fn read_unrecorded(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
) -> Result<Option<setup::Unrecorded>, (setup::Purpose, String)> {
    let docker = unrecorded_docker(env, platform).map_err(|e| (setup::Purpose::Docker, e))?;
    let service =
        unrecorded_service(env, platform, serve_args).map_err(|e| (setup::Purpose::Service, e))?;
    setup::unrecorded_here(docker, service)
}

/// kkt in Docker in the folder `kkt setup` uses, with the access key's host
/// copy it saves.
pub(crate) fn unrecorded_docker(
    env: &Env,
    platform: Platform,
) -> Result<Option<setup::Unrecorded>, String> {
    let (Some(data), Some(config_dir)) = (
        setup::data_dir(env, platform),
        kkt_config_file::config_dir(env),
    ) else {
        return Ok(None);
    };
    let folder = data.join(DOCKER_FOLDER);
    let found = |what: String| {
        format!(
            "Found a kkt server in Docker from {}, not recorded, but {what}.",
            folder.display()
        )
    };
    let existing =
        match kkt_container::existing(&ContainerHost, &kkt_container::Folder(folder.clone())) {
            Ok(Some(existing)) => existing,
            Ok(None) => return Ok(None),
            Err(e) => return Err(found(e.to_string())),
        };
    let key_file = config_dir.join(DOCKER_KEY_FILE_NAME);
    if !key_file.is_file() {
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
pub(crate) fn unrecorded_service(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
) -> Result<Option<setup::Unrecorded>, String> {
    let manager = match Manager::for_this_os(env) {
        Ok(manager) => manager,
        // No service can be installed here.
        Err(ServiceError::NoHome | ServiceError::Unsupported) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !manager
        .state(&ServiceSystem)
        .map_err(|e| e.to_string())?
        .installed
    {
        return Ok(None);
    }
    let found = |what: String| setup::unreadable_service(&what);
    let args = manager
        .installed_args(&ServiceSystem)
        .map_err(|e| found(format!("its service file could not be read: {e}")))?
        .ok_or_else(|| found("its service file went away while it was read".into()))?;
    let unread = |e: String| found(format!("its settings could not be read: {e}"));
    let installed = serve_settings_from_args(env, platform, serve_args, &args).map_err(unread)?;
    let defaults = serve_settings(env, platform, None, ServeOptions::NONE).map_err(unread)?;
    setup::service_runs_defaults(&args, as_running(installed) == as_running(defaults))?;
    let connection = serve_connection(env, platform, ConnectionOptions::NONE, false)
        .map_err(|e| found(format!("its settings could not be read: {e}")))?;
    let bind = serve_bind(env, platform)
        .map_err(|e| found(format!("its settings could not be read: {e}")))?;
    let shared = kkt_remote_access::served(&RemoteHost, bind).map_err(|e| {
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
pub fn read_unfinished(env: &Env) -> Result<Option<setup::Unfinished>, String> {
    let Some(path) = unfinished_path(env) else {
        return Ok(None);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let unfinished = setup::Unfinished::parse(&text, &path)?;
    let present = match unfinished.install_check() {
        setup::InstallCheck::Keep => true,
        setup::InstallCheck::DockerFolder(folder) => folder.join(kkt_container::ENV_FILE).is_file(),
        setup::InstallCheck::Service => {
            Manager::for_this_os(env)
                .and_then(|manager| manager.state(&ServiceSystem))
                .map_err(|e| e.to_string())?
                .installed
        }
        setup::InstallCheck::Gone => false,
    };
    Ok(present.then_some(unfinished))
}

/// The mode of the server `unfinished` notes: from its record, or, for an
/// install that did not finish, where that install puts the server.
pub fn unfinished_mode(
    env: &Env,
    platform: Platform,
    unfinished: &setup::Unfinished,
) -> Result<setup::Mode, String> {
    unfinished.mode(docker_folder(env, platform))
}

/// The folder `kkt setup` keeps kkt's Docker files in.
pub fn docker_folder(env: &Env, platform: Platform) -> Result<PathBuf, String> {
    Ok(setup::data_dir(env, platform)
        .ok_or("set HOME to keep kkt's Docker files")?
        .join(DOCKER_FOLDER))
}

/// Where the setup record is, in kkt's config directory. `None` without a
/// home.
pub(crate) fn record_path(env: &Env) -> Option<PathBuf> {
    kkt_config_file::config_dir(env).map(|dir| setup::record_path(&dir))
}

/// Where an unfinished setup is noted, beside the record. `None` without a
/// home.
pub(crate) fn unfinished_path(env: &Env) -> Option<PathBuf> {
    kkt_config_file::config_dir(env).map(|dir| setup::unfinished_path(&dir))
}

/// This machine's setup record, if `kkt setup` wrote one.
pub fn read_setup(env: &Env) -> Result<Option<setup::Record>, String> {
    let Some(path) = record_path(env) else {
        return Ok(None);
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => setup::Record::parse(&text, &path).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// Refuses to start a server on a machine `kkt setup` set up otherwise.
pub fn check_setup(env: &Env) -> Result<(), String> {
    let record = read_setup(env)?;
    let service_running = match record.as_ref().map(|r| &r.mode) {
        Some(setup::Mode::Service { .. }) => {
            let state = Manager::for_this_os(env).and_then(|m| m.state(&ServiceSystem));
            match state {
                Ok(state) => Some(state.running),
                Err(e) => {
                    // Not knowing is no reason to refuse: a server already
                    // on the port stops this one with its own message.
                    eprintln!("warning: could not tell whether the kkt service is running: {e}");
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
pub fn write_setup_record(env: &Env, record: &setup::Record) -> Result<(), String> {
    let path = record_path(env).ok_or("set HOME to keep the setup record")?;
    write_whole(&path, &record.render()?)
}

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again.
pub fn recorded_setup(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
) -> Result<setup::Record, String> {
    if let Some(record) = read_setup(env)? {
        return Ok(record);
    }
    let no_record = "this machine has no setup record";
    // An unfinished setup is described by `kkt status`.
    match read_unfinished(env) {
        Ok(Some(_)) => return Err(format!("{no_record}; run `kkt setup` to finish it")),
        Ok(None) => {}
        Err(e) => return Err(format!("{no_record}; run `kkt setup` first ({e})")),
    }
    Err(match read_unrecorded(env, platform, serve_args) {
        Ok(Some(found)) => format!(
            "{no_record}. {} Run `kkt setup` to record it (`kkt setup --adopt` with no \
             terminal); it installs nothing. Then run this again.",
            found.summary()
        ),
        Ok(None) => format!("{no_record}; run `kkt setup` first"),
        Err((_, e)) => format!("{no_record}; run `kkt setup` first. {e}"),
    })
}
