//! Looking at, backing up, upgrading, stopping and removing kkt in Docker.

use super::compose::{compose, tailscale_status};
use super::files::{ENV_FILE, set_version, version_in_env, write_embedded, write_env};
use super::provision::{key_show, run, run_attached, save_keys};
use super::tailscale::parse_status;
use super::{DockerError, Folder, Host};
use crate::clock::Clock;
use crate::config::DOCKER_BACKUPS_MOUNT;
use std::path::{Path, PathBuf};

/// How kkt in Docker stands.
#[derive(Debug, PartialEq, Eq)]
pub struct State {
    /// Whether the managed folder has its settings file.
    pub files_present: bool,
    /// Whether the kkt container is running.
    pub running: bool,
    /// The kkt version the settings file names.
    pub version: Option<String>,
    /// The selected name pack, or why it could not be read.
    pub name_pack: Result<String, String>,
    /// Whether it is set up to be served over Tailscale.
    pub tailscale: bool,
    /// The Tailscale container's login, for a setup served over Tailscale.
    pub login: Option<Login>,
}

/// Whether the Tailscale container is logged in to the tailnet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    LoggedIn,
    /// Not logged in: the machine's key expired, or it was logged out. It is
    /// off the tailnet until someone logs it in again.
    NeedsLogin,
    /// The Tailscale container does not answer, such as when it is not running.
    Unreachable,
    /// Some other state, as Tailscale names it.
    Other(String),
}

pub fn status(host: &dyn Host, folder: &Folder) -> Result<State, DockerError> {
    let env_path = folder.path().join(ENV_FILE);
    if !host.exists(&env_path) {
        return Ok(State {
            files_present: false,
            running: false,
            version: None,
            name_pack: Err("container is not set up".into()),
            tailscale: false,
            login: None,
        });
    }
    let env = host
        .read_file(&env_path)
        .map_err(|e| DockerError::io(format!("could not read {}", env_path.display()), e))?;
    let ps = compose(folder, &["ps", "--status", "running", "--services"]);
    let done = run(host, &ps)?;
    if !done.success() {
        // Not knowing is not the same as stopped.
        return Err(DockerError::Command {
            command: ps.to_string(),
            detail: done.detail(),
        });
    }
    let tailscale = env
        .lines()
        .any(|l| l.trim() == "COMPOSE_FILE=compose.tailscale.yaml");
    let login = if tailscale {
        Some(login_state(host, folder)?)
    } else {
        None
    };
    let running = done.stdout.lines().any(|l| l.trim() == "kkt");
    let name_pack = if running {
        super::name_pack::selected(host, folder)
    } else {
        Err("container is not running".into())
    };
    Ok(State {
        files_present: true,
        running,
        version: version_in_env(&env),
        name_pack,
        tailscale,
        login,
    })
}

/// Asks the Tailscale container whether it is logged in.
fn login_state(host: &dyn Host, folder: &Folder) -> Result<Login, DockerError> {
    let done = run(host, &tailscale_status(folder))?;
    if !done.success() {
        return Ok(Login::Unreachable);
    }
    Ok(match parse_status(&done.stdout) {
        Ok(node) => match node.backend_state.as_str() {
            "Running" => Login::LoggedIn,
            "NeedsLogin" => Login::NeedsLogin,
            other => Login::Other(other.to_string()),
        },
        Err(_) => Login::Other("an unreadable answer".to_string()),
    })
}

/// How the host's copy of a key compares with the running kkt container's.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyCopy {
    Same,
    /// The container's key changed after the copy was saved, such as when
    /// it was rotated inside the container.
    Differs,
}

/// Compares the web key's host copy at `web_key_file` with the running kkt
/// container's web key. Neither key is shown, nor put in an error.
pub fn compare_web_key(
    host: &dyn Host,
    folder: &Folder,
    web_key_file: &Path,
) -> Result<KeyCopy, DockerError> {
    let copy = host
        .read_file(web_key_file)
        .map_err(|e| DockerError::io(format!("could not read {}", web_key_file.display()), e))?;
    let show = key_show(folder, true);
    let done = run(host, &show)?;
    let server = done.stdout.trim();
    if !done.success() || server.is_empty() {
        // Its output is left out: it may hold the key.
        return Err(DockerError::Command {
            command: show.to_string(),
            detail: match done.code {
                Some(code) => format!("exit {code}, no key"),
                None => "ended by a signal".to_string(),
            },
        });
    }
    Ok(if copy.trim() == server {
        KeyCopy::Same
    } else {
        KeyCopy::Differs
    })
}

/// Starts the containers.
pub fn up(host: &dyn Host, folder: &Folder) -> Result<(), DockerError> {
    run_attached(host, &compose(folder, &["up", "-d"]))
}

/// Stops and removes the containers. The data stays, in its volumes, unless
/// `remove_data`.
pub fn down(host: &dyn Host, folder: &Folder, remove_data: bool) -> Result<(), DockerError> {
    let args: &[&str] = if remove_data {
        &["down", "--volumes"]
    } else {
        &["down"]
    };
    run_attached(host, &compose(folder, args))
}

/// Backs kkt up with `kkt backup` inside the container, then copies the new
/// backup folder into `dest`, if given, owned by the person running this.
/// Without a destination, returns the path in the managed backups folder.
/// The managed copy stays in either case.
pub fn backup(
    host: &dyn Host,
    folder: &Folder,
    dest: Option<&Path>,
) -> Result<PathBuf, DockerError> {
    let name = backup_in_container(host, folder)?;
    let Some(dest) = dest else {
        return Ok(folder.backups().join(name));
    };
    // A folder typed relative to where the person is, not to the managed
    // folder the Docker commands run in.
    let dest = host
        .working_dir()
        .map_err(|e| DockerError::io("could not find the current folder", e))?
        .join(dest);
    host.create_dir_all(&dest)
        .map_err(|e| DockerError::io(format!("could not create {}", dest.display()), e))?;
    let from = format!("kkt:{DOCKER_BACKUPS_MOUNT}/{name}");
    let to = dest.display().to_string();
    let command = compose(folder, &["cp", &from, &to]);
    let done = run(host, &command)?;
    if !done.success() {
        return Err(DockerError::Command {
            command: command.to_string(),
            detail: done.detail(),
        });
    }
    Ok(dest.join(name))
}

/// Moves kkt to `new_version`: a backup first (so a failed backup stops the
/// upgrade), then this program's Compose files and the new version in the
/// settings file, then the new image. Then, given the access key's host copy
/// `key_file`, saves both keys again as setup does, so the host has the web
/// key a new version made. kkt is running by then, so failing to save them
/// is said, with how to get the keys, and does not fail the upgrade.
pub async fn upgrade(
    host: &dyn Host,
    clock: &dyn Clock,
    folder: &Folder,
    new_version: &str,
    key_file: Option<&Path>,
) -> Result<(), DockerError> {
    let env_path = folder.path().join(ENV_FILE);
    let env = host
        .read_file(&env_path)
        .map_err(|e| DockerError::io(format!("could not read {}", env_path.display()), e))?;
    backup_in_container(host, folder)?;
    let tailscale = env
        .lines()
        .any(|l| l.trim() == "COMPOSE_FILE=compose.tailscale.yaml");
    write_embedded(host, folder, tailscale)?;
    write_env(host, folder, &set_version(&env, new_version))?;
    let result = run_attached(host, &compose(folder, &["pull"]))
        .and_then(|()| run_attached(host, &compose(folder, &["up", "-d"])));
    if let Err(error) = result {
        // Do not leave the selected version claiming that a failed upgrade
        // succeeded: the caller's version check must allow a later retry.
        write_env(host, folder, &env).map_err(|restore| DockerError::Unexpected {
            what: "upgrade recovery",
            detail: format!("{error}; could not restore the previous settings: {restore}"),
        })?;
        return Err(error);
    }
    let Some(key_file) = key_file else {
        return Ok(());
    };
    match save_keys(host, clock, folder, key_file).await {
        Ok(web_key_file) => host.say(&format!(
            "The web key is saved in {}. Read it with `kkt key show --web`.",
            web_key_file.display()
        )),
        Err(e) => host.say(&format!(
            "kkt is running, but its keys could not be saved on this machine: {e}. \
             To save them, run `kkt setup` again with the same answers. \
             To read the web key now, run `docker compose exec kkt kkt key show --web` in {}.",
            folder.path().display()
        )),
    }
    Ok(())
}

/// Runs `kkt backup` in the container and returns the new folder's name.
fn backup_in_container(host: &dyn Host, folder: &Folder) -> Result<String, DockerError> {
    let command = compose(
        folder,
        &["exec", "-T", "kkt", "kkt", "backup", DOCKER_BACKUPS_MOUNT],
    );
    let done = run(host, &command)?;
    if !done.success() {
        return Err(DockerError::Command {
            command: command.to_string(),
            detail: done.detail(),
        });
    }
    let line = done
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("Backed up to "))
        .ok_or_else(|| DockerError::Unexpected {
            what: "`kkt backup`",
            detail: done.stdout.trim().to_string(),
        })?;
    Path::new(line.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| DockerError::Unexpected {
            what: "`kkt backup`",
            detail: line.to_string(),
        })
}
