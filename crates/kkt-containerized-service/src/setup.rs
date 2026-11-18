//! Setting the service up in its managed folder: its files, the backups
//! folder's owner, the Tailscale container's login when the install is
//! shared, starting it, and saving the server's keys' copies on this
//! machine.

use crate::backup::backups;
use crate::files::{ENV_FILE, PENDING_TAILNET, embedded};
use crate::in_server::SERVER;
use crate::services::worded;
use crate::{NodeLogin, SharingNode, Tell};
use kkt_clock::{Clock, Span};
use kkt_container::{
    Browser, Inputs, KeyCopies, LoginLink, Output, Project, ProjectError, ProjectFile, Projects,
    Provisioned, User,
};
use kkt_keys::KeyFiles;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The service the Tailscale container runs in, when the install is shared.
const SHARING: &str = "tailscale";
/// The user ID the server's image runs as. A host folder it writes backups
/// to must belong to it.
const IMAGE_USER_ID: u32 = 10001;
/// Where the Compose file mounts the backups folder in the container.
const BACKUPS_MOUNT: &str = "/backups";
/// The exit code of a command given an option it does not know, as a kkt
/// from before admin keys is given `kkt key show --admin`.
const UNKNOWN_OPTION_EXIT: i32 = 2;

/// How long setup waits on the containers, and how often it asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// How long to wait for someone to log the Tailscale container in.
    pub login_wait: Duration,
    /// How long to wait for the server to start and make its keys.
    pub start_wait: Duration,
    /// How many checks to wait for a login link from a Tailscale container
    /// that needs a login before restarting it. A node whose key expired,
    /// or that was logged out, asks for no login until it starts again; a
    /// new one shows its link within a few seconds.
    pub link_wait_polls: u32,
    /// How often to ask while waiting.
    pub poll_interval: Duration,
}

impl Settings {
    /// Ten minutes for the login and two for the server to start, asking
    /// every two seconds; a Tailscale container with no login link after
    /// five checks is restarted.
    pub(crate) const DEFAULT: Settings = Settings {
        login_wait: Duration::from_secs(10 * 60),
        start_wait: Duration::from_secs(2 * 60),
        link_wait_polls: 5,
        poll_interval: Duration::from_secs(2),
    };
}

/// What setup works with besides the project: the person, the clock, the
/// login link and browser, the sharing node's reader and the key files.
pub(crate) struct Parts<'a> {
    pub projects: &'a dyn Projects,
    pub tell: &'a dyn Tell,
    pub clock: &'a dyn Clock,
    pub link: &'a dyn LoginLink,
    pub browser: &'a dyn Browser,
    pub sharing: &'a dyn SharingNode,
    pub key_files: &'a dyn KeyFiles,
    pub settings: Settings,
}

/// The settings file for `inputs`. `node` is the node's name on the
/// tailnet, `(hostname, tailnet)`, once Tailscale has logged in; until
/// then a placeholder stands in for the tailnet so Compose can start the
/// Tailscale container alone.
pub(crate) fn env_text(inputs: &Inputs, node: Option<(&str, &str)>) -> String {
    let mut text = String::from(
        "# Written by `kkt setup`. Setup writes it again when you run it or upgrade.\n",
    );
    text.push_str(&format!("KKT_VERSION={}\n", inputs.version));
    text.push_str(&format!("COMPOSE_PROJECT_NAME={}\n", inputs.project));
    text.push_str(&format!("KKT_PORT={}\n", inputs.port));
    if !inputs.allow_hosts.is_empty() {
        text.push_str(&format!(
            "KKT_ALLOW_HOSTS={}\n",
            inputs.allow_hosts.join(",")
        ));
    }
    if let Some(sharing) = &inputs.sharing {
        let (hostname, tailnet) = node.unwrap_or((&sharing.hostname, PENDING_TAILNET));
        text.push_str(&format!(
            "COMPOSE_FILE={}\n",
            crate::files::TAILSCALE_COMPOSE_FILE
        ));
        text.push_str(&format!("TS_HOSTNAME={hostname}\n"));
        text.push_str(&format!("TS_TAILNET={tailnet}\n"));
        if let Some(key) = &sharing.auth_key {
            text.push_str(&format!("TS_AUTHKEY={key}\n"));
        }
    }
    text
}

/// Writes the files for `inputs` into the project's folder, starts the
/// server (and Tailscale, logging it in) and saves the access and web
/// keys' copies where `keys` says, each readable only by its owner. Safe to
/// run again on the same folder: it carries on from where it was.
pub(crate) async fn provision(
    parts: &Parts<'_>,
    project: &Project<'_>,
    inputs: &Inputs,
    keys: &KeyCopies,
) -> Result<Provisioned, String> {
    let projects = parts.projects;
    let tell = parts.tell;
    check_tool(projects, project)?;
    let sharing = inputs.sharing.is_some();
    let folder = project.folder;
    make_folder(projects, project, folder.path())?;
    make_folder(projects, project, &backups(folder))?;
    write_embedded(projects, project, sharing)?;
    write_env(projects, project, &env_text(inputs, None))?;
    own_backups(projects, project)?;

    let node = if sharing {
        tell.say("Starting Tailscale...");
        projects
            .up(project, &[SHARING], Output::Terminal)
            .map_err(|e| worded(projects, e))?;
        let node = log_in(parts, project).await?;
        write_env(
            projects,
            project,
            &env_text(inputs, Some((&node.0, &node.1))),
        )?;
        tell.say(&format!("Logged in to Tailscale as {}.{}.", node.0, node.1));
        Some(node)
    } else {
        None
    };

    tell.say("Starting kkt (this takes a minute if the image has to be downloaded)...");
    projects
        .up(project, &[], Output::Terminal)
        .map_err(|e| worded(projects, e))?;
    let saved = save_keys(parts, project, keys).await?;
    say_saved(tell, &saved);
    tell.say(&format!(
        "kkt is running. The access key is saved in {}.",
        keys.access.display()
    ));
    Ok(Provisioned { node })
}

/// The copies [`save_keys`] wrote besides the access key's.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SavedKeys {
    pub web: PathBuf,
    /// `None` when the server has no admin key.
    pub admin: Option<PathBuf>,
}

/// Says where the keys in `saved` are, and how to read them.
pub(crate) fn say_saved(tell: &dyn Tell, saved: &SavedKeys) {
    tell.say(&format!(
        "The web key is saved in {}. Read it with `kkt key show --web`.",
        saved.web.display()
    ));
    if let Some(admin) = &saved.admin {
        tell.say(&format!(
            "The admin key for /mcp-admin is saved in {}. Read it with `kkt key show --admin`.",
            admin.display()
        ));
    }
}

/// Waits for the running server to have its access and web keys, then
/// writes their copies where `keys` says through the key files, each
/// readable only by its owner, replacing any copies there. If the server
/// also has an admin key, writes its copy the same way. No key is shown or
/// put in a command's arguments or an error.
pub(crate) async fn save_keys(
    parts: &Parts<'_>,
    project: &Project<'_>,
    keys: &KeyCopies,
) -> Result<SavedKeys, String> {
    let access = wait_for_key(parts, project, ServerKey::Access).await?;
    let web_key = wait_for_key(parts, project, ServerKey::Web).await?;
    // Only a server whose operator asked for one has an admin key, so it is
    // not waited for.
    let admin_key = read_admin_key(parts.projects, project)?;
    // Each folder once, the access key's first.
    let mut made: Vec<&Path> = Vec::new();
    for path in [&keys.access, &keys.web, &keys.admin] {
        if let Some(parent) = path.parent()
            && !made.contains(&parent)
        {
            parts.key_files.create_dir_all(parent)?;
            made.push(parent);
        }
    }
    // A copy is always replaced: it follows the server's key.
    let write = |path: &Path, key: &str| parts.key_files.write_key(path, key, true);
    write(&keys.access, &access)?;
    write(&keys.web, &web_key)?;
    let admin = match admin_key {
        Some(admin_key) => {
            write(&keys.admin, &admin_key)?;
            Some(keys.admin.clone())
        }
        None => None,
    };
    Ok(SavedKeys {
        web: keys.web.clone(),
        admin,
    })
}

/// One of the server's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerKey {
    Access,
    Web,
    Admin,
}

/// The arguments that print the server's key `which`.
fn key_show(which: ServerKey) -> &'static [&'static str] {
    match which {
        ServerKey::Access => &["kkt", "key", "show"],
        ServerKey::Web => &["kkt", "key", "show", "--web"],
        ServerKey::Admin => &["kkt", "key", "show", "--admin"],
    }
}

/// Waits for the server to be up and returns the key `which`, without the
/// line's end.
pub(crate) async fn wait_for_key(
    parts: &Parts<'_>,
    project: &Project<'_>,
    which: ServerKey,
) -> Result<String, String> {
    let projects = parts.projects;
    poll(
        parts.clock,
        parts.settings.start_wait,
        parts.settings.poll_interval,
        || {
            let ran = projects
                .exec(
                    project,
                    SERVER,
                    User::Default,
                    key_show(which),
                    Output::Captured,
                )
                .map_err(|e| worded(projects, e))?;
            let key = ran.stdout.trim();
            Ok((ran.success() && !key.is_empty()).then(|| key.to_string()))
        },
    )
    .await?
    .ok_or_else(|| {
        format!(
            "kkt did not start or did not make its keys in time; `{}` shows why",
            projects.typed(&["logs", SERVER])
        )
    })
}

/// The running server's admin key, without the line's end, or `None` if it
/// has none: its `kkt key show --admin` says so with kkt-keys'
/// `NO_ADMIN_KEY_EXIT`, or, from a kkt older than admin keys, refuses the
/// option. Any other ending, or no key printed, is an error, which leaves
/// the command's output out since it may hold the key.
pub(crate) fn read_admin_key(
    projects: &dyn Projects,
    project: &Project,
) -> Result<Option<String>, String> {
    let ran = projects
        .exec(
            project,
            SERVER,
            User::Default,
            key_show(ServerKey::Admin),
            Output::Captured,
        )
        .map_err(|e| worded(projects, e))?;
    let key = ran.stdout.trim();
    match ran.status {
        Some(0) if !key.is_empty() => Ok(Some(key.to_string())),
        Some(code)
            if code == i32::from(kkt_keys::NO_ADMIN_KEY_EXIT) || code == UNKNOWN_OPTION_EXIT =>
        {
            Ok(None)
        }
        status => Err(format!(
            "`{}` failed ({})",
            ran.command,
            match status {
                Some(code) => format!("exit {code}, could not read the admin key"),
                None => "ended by a signal".to_string(),
            }
        )),
    }
}

/// Fails with a clear message if the tool, or its Compose, is missing.
fn check_tool(projects: &dyn Projects, project: &Project) -> Result<(), String> {
    let tool = projects.tool();
    projects.available(project).map_err(|e| match e {
        ProjectError::Missing { command, cause } => format!(
            "{tool} is not available: could not run `{command}`: {cause}. Install {tool} with Compose and run setup again"
        ),
        other @ ProjectError::Failed { .. } => format!("{tool} is not available: {other}"),
        other => worded(projects, other),
    })
}

fn make_folder(projects: &dyn Projects, project: &Project, path: &Path) -> Result<(), String> {
    projects
        .make_folders(project, &[path])
        .map_err(|e| e.to_string())
}

/// Writes the Compose files that come with this program into the folder.
fn write_embedded(projects: &dyn Projects, project: &Project, sharing: bool) -> Result<(), String> {
    for (name, text) in embedded(sharing) {
        let path = project.folder.path().join(name);
        if let Some(parent) = path.parent() {
            make_folder(projects, project, parent)?;
        }
        projects
            .write(
                project,
                &[ProjectFile {
                    path: &path,
                    text,
                    private: false,
                }],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Writes the settings file, readable only by its owner: it can hold a
/// Tailscale auth key.
pub(crate) fn write_env(
    projects: &dyn Projects,
    project: &Project,
    text: &str,
) -> Result<(), String> {
    let path = project.folder.path().join(ENV_FILE);
    projects
        .write(
            project,
            &[ProjectFile {
                path: &path,
                text,
                private: true,
            }],
        )
        .map_err(|e| e.to_string())
}

/// Makes the backups folder belong to the user the server's image runs
/// as, so the container can write to it, without needing sudo on the host.
fn own_backups(projects: &dyn Projects, project: &Project) -> Result<(), String> {
    let owner = IMAGE_USER_ID.to_string();
    projects
        .run_once(
            project,
            SERVER,
            User::Root,
            "chown",
            &[&owner, BACKUPS_MOUNT],
            Output::Terminal,
        )
        .and_then(|ran| ran.succeeded())
        .map(|_| ())
        .map_err(|e| worded(projects, e))
}

/// Waits for the Tailscale container to be logged in, showing the login
/// link once, and returns the node's name as `(hostname, tailnet)`.
async fn log_in(parts: &Parts<'_>, project: &Project<'_>) -> Result<(String, String), String> {
    let Parts {
        projects,
        tell,
        link,
        browser,
        sharing,
        settings,
        ..
    } = *parts;
    let unexpected =
        |detail: &str| format!("unexpected output from `tailscale status --json`: {detail}");
    let mut said_link = false;
    // Polls spent logged out with no link, and whether the container has
    // been restarted for one.
    let mut without_link = 0;
    let mut restarted = false;
    let found = poll(parts.clock, settings.login_wait, settings.poll_interval, || {
        let ran = projects
            .exec(
                project,
                SHARING,
                User::Default,
                &["tailscale", "status", "--json"],
                Output::Captured,
            )
            .map_err(|e| worded(projects, e))?;
        if !ran.success() {
            // The container is still starting.
            return Ok(None);
        }
        let node = sharing.status(&ran.stdout).map_err(|e| unexpected(&e))?;
        match node.login {
            NodeLogin::LoggedIn => Ok(Some(node)),
            NodeLogin::NeedsLogin => {
                if node.link.is_none() && !said_link && !restarted {
                    without_link += 1;
                    if without_link >= settings.link_wait_polls {
                        // A node that was logged in before, and whose key
                        // then expired or was removed, is not asking for a
                        // login until it starts again.
                        tell.say(
                            "Tailscale needs you to log in again. Restarting it to get a login link...",
                        );
                        projects
                            .restart(project, SHARING, Output::Terminal)
                            .map_err(|e| worded(projects, e))?;
                        restarted = true;
                    }
                }
                if let Some(url) = &node.link {
                    if !said_link {
                        link.show(url);
                        tell.say("Waiting for you to log in...");
                        said_link = true;
                    }
                    if link.open_requested() {
                        match browser.open(url) {
                            Ok(()) => tell.say("Opening the link in your browser..."),
                            Err(e) => tell.say(&format!(
                                "Could not open a browser ({e}); open the link above yourself."
                            )),
                        }
                    }
                    if let Some(why) = browser.open_failure() {
                        tell.say(&format!(
                            "Could not open a browser ({why}); open the link above yourself."
                        ));
                    }
                }
                Ok(None)
            }
            NodeLogin::Other(_) => Ok(None),
        }
    })
    .await?
    .ok_or(
        "Tailscale is not logged in yet. Run setup again to continue; the containers are left as they are",
    )?;
    if !found.https {
        return Err("HTTPS certificates are not enabled for your tailnet. Turn them on at https://login.tailscale.com/admin/dns (\"Enable HTTPS\"), then run setup again".into());
    }
    let full = found
        .name
        .ok_or_else(|| unexpected("logged in, but the node has no name"))?;
    split_name(&full)
        .ok_or_else(|| unexpected(&format!("the node name {full:?} has no tailnet part")))
}

/// A node's full name split into its own name and the tailnet's, such as
/// `kkt-1.tail1234.ts.net` into `kkt-1` and `tail1234.ts.net`.
fn split_name(full: &str) -> Option<(String, String)> {
    let (hostname, tailnet) = full.split_once('.')?;
    if hostname.is_empty() || tailnet.is_empty() {
        return None;
    }
    Some((hostname.to_string(), tailnet.to_string()))
}

/// Asks `attempt` until it has an answer or `wait` has passed on `clock`,
/// pausing `every` between asks. `Ok(None)` means it never did.
async fn poll<T>(
    clock: &dyn Clock,
    wait: Duration,
    every: Duration,
    mut attempt: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    let end = clock.now() + Span::from_std(wait).unwrap_or(Span::MAX);
    let pause = Span::from_std(every).unwrap_or(Span::MAX);
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
