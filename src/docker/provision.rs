//! Setting kkt up in Docker: files, folder ownership, the Tailscale login,
//! starting the containers and saving both keys.

use super::compose::{compose, tailscale_status, version};
use super::files::{create_dir, env_text, write_embedded, write_env};
use super::tailscale::{parse_status, split_name};
use super::{DockerError, Folder, Host, Inputs, LoginLink};
use crate::clock::Clock;
use crate::config::{
    DOCKER_BACKUPS_MOUNT, DOCKER_IMAGE_USER_ID, DOCKER_LINK_WAIT_POLLS, DOCKER_LOGIN_WAIT,
    DOCKER_POLL_INTERVAL, DOCKER_START_WAIT,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What setup found out while starting kkt.
#[derive(Debug, PartialEq, Eq)]
pub struct Provisioned {
    /// The node's name on the tailnet, as `(hostname, tailnet)`, if kkt is
    /// served over Tailscale. kkt answers to `<hostname>.<tailnet>`.
    pub node: Option<(String, String)>,
}

/// Writes the files for `inputs` into `folder`, starts kkt (and Tailscale)
/// and saves the access key at `key_file` and the web key beside it, both
/// readable only by their owner.
/// Safe to run again on the same folder: it carries on from where it was.
pub async fn provision(
    host: &dyn Host,
    clock: &dyn Clock,
    login: &dyn LoginLink,
    folder: &Folder,
    inputs: &Inputs,
    key_file: &Path,
) -> Result<Provisioned, DockerError> {
    check_docker(host)?;
    let tailscale = inputs.tailscale.is_some();
    create_dir(host, folder.path())?;
    create_dir(host, &folder.backups())?;
    write_embedded(host, folder, tailscale)?;
    write_env(host, folder, &env_text(inputs, None))?;
    own_backups(host, folder)?;

    let node = if tailscale {
        host.say("Starting Tailscale...");
        run_attached(host, &compose(folder, &["up", "-d", "tailscale"]))?;
        let node = log_in(host, clock, login, folder).await?;
        write_env(host, folder, &env_text(inputs, Some((&node.0, &node.1))))?;
        host.say(&format!("Logged in to Tailscale as {}.{}.", node.0, node.1));
        Some(node)
    } else {
        None
    };

    host.say("Starting kkt (this takes a minute if the image has to be downloaded)...");
    run_attached(host, &compose(folder, &["up", "-d"]))?;
    let saved = save_keys(host, clock, folder, key_file).await?;
    say_saved(host, &saved);
    host.say(&format!(
        "kkt is running. The access key is saved in {}.",
        key_file.display()
    ));
    Ok(Provisioned { node })
}

/// The host copies [`save_keys`] wrote besides the access key's.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SavedKeys {
    pub web: PathBuf,
    /// `None` when the server has no admin key.
    pub admin: Option<PathBuf>,
}

/// Says where the keys in `saved` are, and how to read them.
pub(super) fn say_saved(host: &dyn Host, saved: &SavedKeys) {
    host.say(&format!(
        "The web key is saved in {}. Read it with `kkt key show --web`.",
        saved.web.display()
    ));
    if let Some(admin) = &saved.admin {
        host.say(&format!(
            "The admin key for /mcp-admin is saved in {}. Read it with `kkt key show --admin`.",
            admin.display()
        ));
    }
}

/// Gives the running kkt container in `folder` an admin key, unless it has
/// one, restarting kkt so it serves `/mcp-admin`; then saves the keys' host
/// copies beside the access key's at `key_file`, as setup does, and returns
/// the admin key's. No key is shown or put in a command's arguments.
pub async fn add_admin_key(
    host: &dyn Host,
    clock: &dyn Clock,
    folder: &Folder,
    key_file: &Path,
) -> Result<PathBuf, DockerError> {
    // Wait for kkt to be up before asking it about its admin key.
    wait_for_key(host, clock, folder, ServerKey::Access).await?;
    if !run(host, &key_show(folder, ServerKey::Admin))?.success() {
        host.say("Making the admin key for /mcp-admin and restarting kkt...");
        run_attached(
            host,
            &compose(
                folder,
                &["exec", "-T", "kkt", "kkt", "key", "create", "--admin"],
            ),
        )?;
        run_attached(host, &compose(folder, &["restart", "kkt"]))?;
    }
    let saved = save_keys(host, clock, folder, key_file).await?;
    say_saved(host, &saved);
    saved.admin.ok_or(DockerError::Unexpected {
        what: "`kkt key show --admin`",
        detail: "kkt has no admin key after making one".into(),
    })
}

/// Waits for the running kkt container to have its access and web keys,
/// then saves the access key at `key_file` and the web key beside it, both
/// readable only by their owner, replacing any copies there. If the
/// container also has an admin key, saves it beside them the same way.
/// No key is shown or put in a command's arguments.
pub(super) async fn save_keys(
    host: &dyn Host,
    clock: &dyn Clock,
    folder: &Folder,
    key_file: &Path,
) -> Result<SavedKeys, DockerError> {
    let key = wait_for_key(host, clock, folder, ServerKey::Access).await?;
    let web_key = wait_for_key(host, clock, folder, ServerKey::Web).await?;
    // Only a server whose operator asked for one has an admin key, so it is
    // not waited for.
    let shown = run(host, &key_show(folder, ServerKey::Admin))?;
    let admin_key = (shown.success() && !shown.stdout.trim().is_empty())
        .then(|| format!("{}\n", shown.stdout.trim()));
    if let Some(parent) = key_file.parent() {
        create_dir(host, parent)?;
    }
    let write = |path: &Path, key: &str| {
        host.write_file(path, key, true)
            .map_err(|e| DockerError::io(format!("could not write {}", path.display()), e))
    };
    write(key_file, &key)?;
    let web = key_file.with_file_name(crate::config::DOCKER_WEB_KEY_FILE_NAME);
    write(&web, &web_key)?;
    let admin = match admin_key {
        Some(admin_key) => {
            let admin = key_file.with_file_name(crate::config::DOCKER_ADMIN_KEY_FILE_NAME);
            write(&admin, &admin_key)?;
            Some(admin)
        }
        None => None,
    };
    Ok(SavedKeys { web, admin })
}

/// Fails with a clear message if Docker or its Compose plugin is missing.
fn check_docker(host: &dyn Host) -> Result<(), DockerError> {
    let command = version();
    match host.run(&command) {
        Ok(done) if done.success() => Ok(()),
        Ok(done) => Err(DockerError::NotAvailable(format!(
            "`{command}` failed ({})",
            done.detail()
        ))),
        Err(e) => Err(DockerError::NotAvailable(format!(
            "could not run `{command}`: {e}. Install Docker with Compose and run setup again"
        ))),
    }
}

/// Makes the backups folder belong to the user the kkt image runs as, so the
/// container can write to it, without needing sudo on the host.
fn own_backups(host: &dyn Host, folder: &Folder) -> Result<(), DockerError> {
    let owner = DOCKER_IMAGE_USER_ID.to_string();
    run_attached(
        host,
        &compose(
            folder,
            &[
                "run",
                "--rm",
                "--no-deps",
                "--user",
                "root",
                "--entrypoint",
                "chown",
                "kkt",
                &owner,
                DOCKER_BACKUPS_MOUNT,
            ],
        ),
    )
}

/// Waits for the Tailscale container to be logged in, showing the login link
/// once, and returns the node's name as `(hostname, tailnet)`.
async fn log_in(
    host: &dyn Host,
    clock: &dyn Clock,
    login: &dyn LoginLink,
    folder: &Folder,
) -> Result<(String, String), DockerError> {
    let status = tailscale_status(folder);
    let mut said_link = false;
    // Polls spent logged out with no link, and whether the container has been
    // restarted for one.
    let mut without_link = 0;
    let mut restarted = false;
    let found = poll(clock, DOCKER_LOGIN_WAIT, DOCKER_POLL_INTERVAL, || {
        let done = run(host, &status)?;
        if !done.success() {
            // The container is still starting.
            return Ok(None);
        }
        let node = parse_status(&done.stdout)?;
        match node.backend_state.as_str() {
            "Running" => Ok(Some(node)),
            "NeedsLogin" => {
                if node.auth_url.is_none() && !said_link && !restarted {
                    without_link += 1;
                    if without_link >= DOCKER_LINK_WAIT_POLLS {
                        // A node that was logged in before, and whose key then
                        // expired or was removed, is not asking for a login
                        // until it starts again.
                        host.say("Tailscale needs you to log in again. Restarting it to get a login link...");
                        run_attached(host, &compose(folder, &["restart", "tailscale"]))?;
                        restarted = true;
                    }
                }
                if let Some(url) = &node.auth_url {
                    if !said_link {
                        login.show(url);
                        host.say("Waiting for you to log in...");
                        said_link = true;
                    }
                    if login.open_requested() {
                        match login.open(url) {
                            Ok(()) => host.say("Opening the link in your browser..."),
                            Err(e) => host.say(&format!(
                                "Could not open a browser ({e}); open the link above yourself."
                            )),
                        }
                    }
                    if let Some(why) = login.open_failure() {
                        host.say(&format!(
                            "Could not open a browser ({why}); open the link above yourself."
                        ));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    })
    .await?
    .ok_or(DockerError::LoginTimeout)?;
    if found.cert_domains.is_empty() {
        return Err(DockerError::HttpsDisabled);
    }
    let full = found.dns_name.ok_or_else(|| DockerError::Unexpected {
        what: "`tailscale status --json`",
        detail: "logged in, but the node has no name".into(),
    })?;
    split_name(&full).ok_or(DockerError::Unexpected {
        what: "`tailscale status --json`",
        detail: format!("the node name {full:?} has no tailnet part"),
    })
}

/// Waits for kkt to be up and returns the selected key.
async fn wait_for_key(
    host: &dyn Host,
    clock: &dyn Clock,
    folder: &Folder,
    which: ServerKey,
) -> Result<String, DockerError> {
    let show = key_show(folder, which);
    poll(clock, DOCKER_START_WAIT, DOCKER_POLL_INTERVAL, || {
        let done = run(host, &show)?;
        let key = done.stdout.trim();
        Ok((done.success() && !key.is_empty()).then(|| format!("{key}\n")))
    })
    .await?
    .ok_or(DockerError::StartTimeout)
}

/// One of the kkt container's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ServerKey {
    Access,
    Web,
    Admin,
}

/// The command that prints the kkt container's key `which`.
pub(super) fn key_show(folder: &Folder, which: ServerKey) -> super::Command {
    let mut args = vec!["exec", "-T", "kkt", "kkt", "key", "show"];
    match which {
        ServerKey::Access => {}
        ServerKey::Web => args.push("--web"),
        ServerKey::Admin => args.push("--admin"),
    }
    compose(folder, &args)
}

/// Asks `attempt` until it has an answer or `wait` has passed on `clock`,
/// pausing `every` between asks. `Ok(None)` means it never did.
async fn poll<T>(
    clock: &dyn Clock,
    wait: Duration,
    every: Duration,
    mut attempt: impl FnMut() -> Result<Option<T>, DockerError>,
) -> Result<Option<T>, DockerError> {
    let end = clock.now() + chrono::Duration::from_std(wait).unwrap_or(chrono::Duration::MAX);
    let pause = chrono::Duration::from_std(every).unwrap_or(chrono::Duration::MAX);
    loop {
        // Asks at a steady rate: the pause counts from when this ask began.
        let began = clock.now();
        if let Some(found) = attempt()? {
            return Ok(Some(found));
        }
        if clock.now() >= end {
            return Ok(None);
        }
        clock.sleep_until(began + pause).await;
    }
}

/// Runs a command and returns how it ended; failing to run it at all is an
/// error.
pub(super) fn run(
    host: &dyn Host,
    command: &super::Command,
) -> Result<super::Finished, DockerError> {
    host.run(command)
        .map_err(|e| DockerError::NotAvailable(format!("could not run `{command}`: {e}")))
}

/// Runs a command with its output on the terminal; a failure is an error.
pub(super) fn run_attached(host: &dyn Host, command: &super::Command) -> Result<(), DockerError> {
    let done = host
        .run_attached(command)
        .map_err(|e| DockerError::NotAvailable(format!("could not run `{command}`: {e}")))?;
    if done.success() {
        Ok(())
    } else {
        Err(DockerError::Command {
            command: command.to_string(),
            detail: done.detail(),
        })
    }
}
