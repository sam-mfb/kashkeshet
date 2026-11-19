//! What the installed service is doing: whether its files are there, which
//! of its containers run, the version its settings select, the name pack
//! the running server uses, and, when it is shared over Tailscale, whether
//! the Tailscale container is logged in.

use crate::ContainerError;
use crate::files::{ENV_FILE, SHARING, TAILSCALE_COMPOSE_FILE, version_in_env};
use crate::in_server::SERVER;
use crate::{ConfigReader, Login, State};
use crate::{NodeLogin, SharingNode};
use kkt_container::{FileState, Output, Project, ProjectError, Projects, User};
use kkt_machine::Env;
use std::path::{Path, PathBuf};

/// `reader` parses the container's config file, for its name pack.
/// `default_name_pack` is the name pack `kkt serve` uses when its settings
/// name none, and `config_file` where its default config file is under its
/// config home; the caller gives both.
pub(crate) fn status(
    projects: &dyn Projects,
    project: &Project,
    sharing: &dyn SharingNode,
    reader: &dyn ConfigReader,
    default_name_pack: &str,
    config_file: &Path,
) -> Result<State, ContainerError> {
    if projects.file(project, Path::new(ENV_FILE)) == FileState::Missing {
        return Ok(State {
            files_present: false,
            running: false,
            version: None,
            name_pack: Err("container is not set up".into()),
            sharing: false,
            login: None,
        });
    }
    let env = projects
        .read(project, Path::new(ENV_FILE))
        .map_err(ContainerError::from)?;
    // Not knowing is not the same as stopped.
    let running = projects
        .running(project)
        .map_err(|e| ContainerError::from_tool(projects, e))?;
    let tailscale = env
        .lines()
        .any(|l| l.trim() == format!("COMPOSE_FILE={TAILSCALE_COMPOSE_FILE}"));
    let login = if tailscale {
        Some(login_state(projects, project, sharing)?)
    } else {
        None
    };
    let running = running.iter().any(|service| service == SERVER);
    let name_pack = if running {
        name_pack(projects, project, reader, default_name_pack, config_file)
    } else {
        Err("container is not running".into())
    };
    Ok(State {
        files_present: true,
        running,
        version: version_in_env(&env),
        name_pack,
        sharing: tailscale,
        login,
    })
}

/// Asks the Tailscale container whether it is logged in.
fn login_state(
    projects: &dyn Projects,
    project: &Project,
    sharing: &dyn SharingNode,
) -> Result<Login, ContainerError> {
    let command = sharing.status_command();
    let command: Vec<&str> = command.iter().map(String::as_str).collect();
    let ran = projects
        .exec(project, SHARING, User::Default, &command, Output::Captured)
        .map_err(|e| ContainerError::from_tool(projects, e))?;
    if !ran.success() {
        return Ok(Login::Unreachable);
    }
    Ok(match sharing.login(&ran.stdout) {
        Some(NodeLogin::LoggedIn) => Login::LoggedIn,
        Some(NodeLogin::NeedsLogin) => Login::NeedsLogin,
        Some(NodeLogin::Other(state)) => Login::Other(state),
        None => Login::Other("an unreadable answer".to_string()),
    })
}

/// The name pack the running `kkt serve` uses: its `--name-pack`, else the
/// one its config file names, else `default`. The config file is
/// `--config`'s, else `config_file` under its config home. Any failure is
/// said as a short reason.
pub(crate) fn name_pack(
    projects: &dyn Projects,
    project: &Project,
    reader: &dyn ConfigReader,
    default: &str,
    config_file: &Path,
) -> Result<String, String> {
    let config = projects.inspect(project, SERVER).map_err(|e| match e {
        ProjectError::NoContainer { .. } => "could not identify the kkt container",
        ProjectError::Unexpected { .. } => "unreadable container settings",
        _ => "container query failed",
    })?;
    let args = config.command.ok_or("unreadable container command")?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let entry = config.entrypoint.ok_or("unreadable container entrypoint")?;
    if entry.len() != 1
        || Path::new(&entry[0]).file_name().and_then(|s| s.to_str()) != Some("kkt")
        || args.first() != Some(&"serve")
    {
        return Err("container does not run kkt serve directly".into());
    }
    if let Some(pack) = flag(&args, "--name-pack")? {
        return Ok(pack.to_string());
    }
    let vars = config.env.ok_or("unreadable container environment")?;
    let variable = |name: &str| {
        vars.iter().find_map(|s| {
            let (key, value) = s.split_once('=')?;
            (key == name && !value.is_empty()).then(|| PathBuf::from(value))
        })
    };
    let env = Env {
        home: variable("HOME"),
        xdg_config_home: variable("XDG_CONFIG_HOME"),
        xdg_data_home: variable("XDG_DATA_HOME"),
        named: Default::default(),
    };
    let explicit = flag(&args, "--config")?;
    let path = match explicit {
        Some(path) => PathBuf::from(path),
        None => env
            .config_home()
            .map(|home| home.join(config_file))
            .ok_or("container has no config path")?,
    };
    // A container whose image names no folder runs in `/`.
    let working = config.working_dir.as_deref().unwrap_or("/");
    let path = if path.is_absolute() {
        path
    } else {
        Path::new(working).join(path)
    };
    let path_text = config_path_text(&path)?;
    if explicit.is_none() {
        let exists = projects
            .exec(
                project,
                SERVER,
                User::Default,
                &["test", "-e", path_text],
                Output::Captured,
            )
            .map_err(|e| match e {
                ProjectError::Missing { cause, .. } => {
                    format!("could not check the container config: {cause}")
                }
                other => format!("could not check the container config: {other}"),
            })?;
        match exists.status {
            Some(0) => {}
            Some(1) => return Ok(default.to_string()),
            _ => return Err("could not check the container config".into()),
        }
    }
    let text = projects
        .exec(
            project,
            SERVER,
            User::Default,
            &["cat", "--", path_text],
            Output::Captured,
        )
        .and_then(|ran| ran.succeeded())
        .map_err(|_| "container query failed")?;
    let said = reader
        .settings(
            &text,
            path.parent().unwrap_or(Path::new("/")),
            env.home.as_deref(),
        )
        .map_err(|e| format!("unreadable container config: {e}"))?;
    Ok(said.name_pack.unwrap_or_else(|| default.to_string()))
}

/// The config file's path as text for a command, refusing one that is not
/// UTF-8.
pub(crate) fn config_path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "unreadable config path".to_string())
}

/// The value serve's `args` give `name`, as `name value` or `name=value`.
fn flag<'a>(args: &[&'a str], name: &str) -> Result<Option<&'a str>, String> {
    for (index, arg) in args.iter().enumerate() {
        if *arg == name {
            return args
                .get(index + 1)
                .copied()
                .map(Some)
                .ok_or_else(|| "container command has a setting without a value".to_string());
        }
        if let Some((key, value)) = arg.split_once('=')
            && key == name
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
