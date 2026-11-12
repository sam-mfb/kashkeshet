//! Setup's paths and wording for the program to carry out.
use crate::{Mode, Purpose, Record};
use std::path::{Path, PathBuf};
const CLIENT_KEY_FILE_NAME: &str = "client-access-key";
const UPGRADE_BACKUP_FOLDER: &str = "upgrade-backups";
const SERVICE_BACKUP_FOLDER: &str = "backups";

pub fn staged_client_key(dir: &Path) -> PathBuf {
    dir.join(format!("{CLIENT_KEY_FILE_NAME}.partial"))
}
/// One key per server, so another server's saved key is never replaced.
pub fn client_key(staged: &Path, url: &str) -> PathBuf {
    let dir = staged.parent().unwrap_or(Path::new("."));
    let server: String = kkt_hash::sha256(url.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    dir.join(format!("{CLIENT_KEY_FILE_NAME}-{server}"))
}

pub fn upgrade_backup_folder(database: &Path) -> PathBuf {
    database
        .parent()
        .unwrap_or(Path::new("."))
        .join(UPGRADE_BACKUP_FOLDER)
}
pub fn service_backup_folder(config_dir: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(config_dir
        .ok_or("set HOME to find the backup folder")?
        .join(SERVICE_BACKUP_FOLDER))
}

/// Clients switched to another server keep using it after the local install goes.
pub fn uninstall_record(record: &Record, version: &str) -> Option<Record> {
    record.clients.clone().map(|server| Record {
        mode: Mode::Client,
        written_by: version.to_string(),
        url: server.url,
        key_file: server.key_file,
        clients: None,
    })
}
pub fn uninstall_line(client: Option<&Record>) -> String {
    match client {
        Some(client) => format!("Removed this machine's server. Data, keys, backups and client configurations were \
             kept, and this machine stays set up as a client of {}.\n", client.url),
        None => "Removed this machine's setup. Data, keys, backups and client configurations were kept.\n".into(),
    }
}

pub fn setup_lane_limit(
    purpose: Purpose,
    local: impl FnOnce() -> Result<usize, String>,
    docker: impl FnOnce() -> Result<usize, String>,
) -> Result<usize, String> {
    match purpose {
        Purpose::Service => local(),
        Purpose::Docker => docker(),
        Purpose::Client => Err("lanes are imported on the server's machine".into()),
    }
}

/// What setup says while it waits for the server at `url`, set up as `mode`,
/// to answer.
pub fn waiting_note(url: &str, mode: &crate::Mode) -> String {
    let tailscale = match mode {
        crate::Mode::Docker(install) => install.tailscale,
        crate::Mode::Service { tailscale } => *tailscale,
        crate::Mode::Client => false,
    };
    if tailscale {
        format!(
            "Waiting for the server to answer at {url} (a new Tailscale node gets its \
             HTTPS certificate on first use)..."
        )
    } else {
        format!("Waiting for the server to answer at {url}...")
    }
}

pub fn web_key_line(path: &Path) -> String {
    format!(
        "Web key: {} (read with `kkt key show --web`)\n",
        path.display()
    )
}
pub fn admin_key_line(path: &Path) -> String {
    format!(
        "Admin key: {} (read with `kkt key show --admin`; agents holding it use /mcp-admin)\n",
        path.display()
    )
}
pub fn configured_name_pack_line(name: &str) -> String {
    format!("Name pack (configured; serve flags may override): {name}\n")
}
pub fn docker_status_lines(state: &kkt_container::State, folder: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(match &state.name_pack {
        Ok(pack) => format!("Name pack: {pack}\n"),
        Err(reason) => format!("Name pack: unavailable ({reason})\n"),
    });
    lines.push(format!("Docker folder: {}\n", folder.display()));
    lines.push(format!(
        "Settings present: {}\nRunning: {}\nVersion: {}\nTailscale: {}\n",
        state.files_present,
        state.running,
        state.version.as_deref().unwrap_or("unknown"),
        state.tailscale
    ));
    match &state.login {
        Some(kkt_container::Login::LoggedIn) => lines.push("Tailscale login: logged in\n".into()),
        Some(kkt_container::Login::NeedsLogin) => lines.push("Tailscale login: needed. The machine's key expired, or it was logged out, so it is off the tailnet. Run `kkt setup` to log in again.\n".into()),
        Some(kkt_container::Login::Unreachable) => lines.push("Tailscale login: unknown; the Tailscale container does not answer\n".into()),
        Some(kkt_container::Login::Other(state)) => lines.push(format!("Tailscale login: {state}\n")),
        None => {}
    }
    lines
}

pub fn service_upgrade_error(error: &str, reload_failed: bool) -> String {
    match reload_failed {
        true => format!(
            "{error} This upgrade is not recorded yet: once the service is loaded, run `kkt upgrade` again to record the new version."
        ),
        false => error.to_string(),
    }
}
