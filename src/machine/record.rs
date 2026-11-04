//! The setup record, the note of an unfinished setup, and finding a server
//! kkt installed here with no record.

use crate::machine::connection::{ConnectionOptions, serve_connection};
use crate::machine::settings::{
    ServeArgs, as_running, serve_bind, serve_settings, serve_settings_from_args,
};
use crate::process::write_whole;
use kkt_config as config;
use kkt_container as docker;
use kkt_remote_access as tailscale;
use kkt_service::{Manager, OsSystem, ServiceError};
use kkt_settings::{Env, Platform, ServeOptions};
use kkt_setup as setup;
use std::path::PathBuf;

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
    serve_args: ServeArgs,
) -> Result<Option<setup::Unrecorded>, (setup::Purpose, String)> {
    let docker = unrecorded_docker(env).map_err(|e| (setup::Purpose::Docker, e))?;
    let service = unrecorded_service(env, serve_args).map_err(|e| (setup::Purpose::Service, e))?;
    match (docker, service) {
        (Some(docker), Some(_)) => Err((
            setup::Purpose::Docker,
            format!(
                "{} A kkt service is installed here too, so which one clients use is not \
                 known; remove one (`kkt service uninstall`, or `docker compose down` in the \
                 Docker folder) and run `kkt setup` again",
                docker.summary()
            ),
        )),
        (found, None) | (None, found) => Ok(found),
    }
}

/// kkt in Docker in the folder `kkt setup` uses, with the access key's host
/// copy it saves.
pub(crate) fn unrecorded_docker(env: &Env) -> Result<Option<setup::Unrecorded>, String> {
    let (Some(data), Some(config_dir)) = (env.data_dir(Platform::CURRENT), env.config_dir()) else {
        return Ok(None);
    };
    let folder = data.join(config::DOCKER_FOLDER);
    let found = |what: String| {
        format!(
            "Found a kkt server in Docker from {}, not recorded, but {what}.",
            folder.display()
        )
    };
    let existing = match docker::existing(&docker::OsHost, &docker::Folder(folder.clone())) {
        Ok(Some(existing)) => existing,
        Ok(None) => return Ok(None),
        Err(e) => return Err(found(e.to_string())),
    };
    let key_file = config_dir.join(config::DOCKER_KEY_FILE_NAME);
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
    serve_args: ServeArgs,
) -> Result<Option<setup::Unrecorded>, String> {
    let manager = match Manager::for_this_os(env) {
        Ok(manager) => manager,
        // No service can be installed here.
        Err(ServiceError::NoHome | ServiceError::Unsupported) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !manager
        .state(&OsSystem)
        .map_err(|e| e.to_string())?
        .installed
    {
        return Ok(None);
    }
    let found = |what: String| format!("Found a kkt service, not recorded, but {what}.");
    let args = manager
        .installed_args(&OsSystem)
        .map_err(|e| found(format!("its service file could not be read: {e}")))?
        .ok_or_else(|| found("its service file went away while it was read".into()))?;
    let unread = |e: String| found(format!("its settings could not be read: {e}"));
    let installed = serve_settings_from_args(env, serve_args, &args).map_err(unread)?;
    let defaults = serve_settings(env, None, ServeOptions::NONE).map_err(unread)?;
    if as_running(installed) != as_running(defaults) {
        return Err(found(format!(
            "it runs `kkt {}`, with settings other than the default config file's, which \
             `kkt setup` and `kkt upgrade` install; a record could not keep them. Keep \
             managing it with `kkt service`, or put its settings in the default config \
             file, install it again with `kkt service install` and no options, and run \
             `kkt setup`",
            args.join(" ")
        )));
    }
    let connection = serve_connection(env, ConnectionOptions::NONE, false)
        .map_err(|e| found(format!("its settings could not be read: {e}")))?;
    let bind =
        serve_bind(env).map_err(|e| found(format!("its settings could not be read: {e}")))?;
    let shared = tailscale::served(&tailscale::OsHost, bind).map_err(|e| {
        found(format!(
            "whether it is shared over Tailscale is not known: {e}"
        ))
    })?;
    Ok(Some(setup::Unrecorded {
        mode: setup::Mode::Service {
            tailscale: shared.is_some(),
        },
        url: match shared {
            Some(node) => format!("https://{}/mcp", node.dns_name),
            None => connection.url,
        },
        key_file: connection.key_file,
    }))
}

/// The server a `kkt setup` installed here but did not record, if it noted
/// one and the install is still there: the managed Docker folder's settings
/// file, or the installed service.
pub fn read_unfinished(env: &Env) -> Result<Option<setup::Unfinished>, String> {
    let Some(path) = setup::unfinished_path(env) else {
        return Ok(None);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let unfinished = setup::Unfinished::parse(&text, &path)?;
    // An install that did not finish may have changed anything or nothing;
    // only a finished one can be looked for.
    let Some(record) = &unfinished.record else {
        return Ok(Some(unfinished));
    };
    let present = match &record.mode {
        setup::Mode::Docker(install) => install.folder.join(docker::ENV_FILE).is_file(),
        setup::Mode::Service { .. } => {
            Manager::for_this_os(env)
                .and_then(|manager| manager.state(&OsSystem))
                .map_err(|e| e.to_string())?
                .installed
        }
        setup::Mode::Client => false,
    };
    Ok(present.then_some(unfinished))
}

/// The mode of the server `unfinished` notes: from its record, or, for an
/// install that did not finish, where that install puts the server.
pub fn unfinished_mode(env: &Env, unfinished: &setup::Unfinished) -> Result<setup::Mode, String> {
    if let Some(record) = &unfinished.record {
        return Ok(record.mode.clone());
    }
    Ok(match unfinished.purpose {
        setup::Purpose::Docker => setup::Mode::Docker(setup::DockerSetup {
            folder: docker_folder(env)?,
            tailscale: unfinished.tailscale,
            hostname: None,
            tailnet: None,
        }),
        _ => setup::Mode::Service {
            tailscale: unfinished.tailscale,
        },
    })
}

/// The folder `kkt setup` keeps kkt's Docker files in.
pub fn docker_folder(env: &Env) -> Result<PathBuf, String> {
    Ok(env
        .data_dir(Platform::CURRENT)
        .ok_or("set HOME to keep kkt's Docker files")?
        .join(config::DOCKER_FOLDER))
}

/// This machine's setup record, if `kkt setup` wrote one.
pub fn read_setup(env: &Env) -> Result<Option<setup::Record>, String> {
    let Some(path) = setup::record_path(env) else {
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
            let state = Manager::for_this_os(env).and_then(|m| m.state(&OsSystem));
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
    let path = setup::record_path(env).ok_or("set HOME to keep the setup record")?;
    write_whole(&path, &record.render()?)
}

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again.
pub fn recorded_setup(env: &Env, serve_args: ServeArgs) -> Result<setup::Record, String> {
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
    Err(match read_unrecorded(env, serve_args) {
        Ok(Some(found)) => format!(
            "{no_record}. {} Run `kkt setup` to record it (`kkt setup --adopt` with no \
             terminal); it installs nothing. Then run this again.",
            found.summary()
        ),
        Ok(None) => format!("{no_record}; run `kkt setup` first"),
        Err((_, e)) => format!("{no_record}; run `kkt setup` first. {e}"),
    })
}
