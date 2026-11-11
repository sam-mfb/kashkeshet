//! An install's lifecycle: `kkt status`, `kkt upgrade`, `kkt uninstall`
//! and `kkt backup`, and the backup `kkt serve` makes before it upgrades
//! the database.

use crate::commands::lanes::existing_db;
use crate::commands::service::{self, ServiceAction, service_plan};
use crate::machine::keys::{
    KeyKind, admin_name_line, local_operator_key, local_web_key, new_key_file,
};
use crate::machine::record::{
    docker_folder, read_setup, read_unfinished, read_unrecorded, recorded_setup, write_setup_record,
};
use crate::machine::service::ServiceSystem;
use crate::machine::settings::{ServeArgs, read_config, serve_bind};
use kkt_backup as backup;
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_config as config;
use kkt_provenance::Build;
use kkt_service::{Manager, ServiceError};
use kkt_settings::Env;
use kkt_setup as setup;
use std::path::{Path, PathBuf};

/// Writes a backup of `contents` into `folder`, copying the database with
/// SQLite's online backup.
fn write_backup(
    folder: &Path,
    clock: &dyn Clock,
    contents: &backup::Contents,
) -> Result<PathBuf, String> {
    backup::write(
        folder,
        clock.now(),
        Build::CURRENT.version,
        contents,
        crate::machine::database::copy,
    )
}

/// Before the server upgrades the tables in `database`, backs it up beside
/// it and prunes those backups to the newest `keep`. Only the database: an
/// upgrade changes nothing else, and a copy of the key in the data folder
/// would be one more place it could leak from. Any failure to back up stops
/// the server before the database is changed.
pub(crate) fn backup_before_upgrade(
    database: &Path,
    keep: usize,
    clock: &dyn Clock,
) -> Result<(), String> {
    let Some(upgrade) = crate::machine::database::pending_upgrade(database)? else {
        return Ok(());
    };
    let folder = database
        .parent()
        .unwrap_or(Path::new("."))
        .join(config::UPGRADE_BACKUP_FOLDER);
    let contents = backup::Contents {
        database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let written = write_backup(&folder, clock, &contents).map_err(|e| {
        format!(
            "the database needs upgrading (schema {} to {}), and kkt backs it up first, \
             but the backup failed, so the server did not start and the database is \
             unchanged: {e}",
            upgrade.from, upgrade.to
        )
    })?;
    tracing::info!(
        "backed up the database to {} before upgrading it from schema {} to {}",
        written.display(),
        upgrade.from,
        upgrade.to
    );
    // The backup is made, so the upgrade is safe; an old backup left behind
    // only takes space.
    match backup::prune(&folder, keep) {
        Ok(removed) => {
            for path in removed {
                tracing::info!("removed the old backup {}", path.display());
            }
        }
        Err(e) => tracing::warn!("could not remove old backups: {e}"),
    }
    Ok(())
}

pub fn backup(
    env: &Env,
    folder: Option<&Path>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let mode = read_setup(env)?.map(|record| record.mode);
    match setup::backup_of(mode.as_ref(), folder, db.is_some() || config.is_some())? {
        setup::BackupOf::Docker { folder, into } => {
            let written = kkt_container::backup(
                &kkt_container::OsHost,
                &kkt_container::Folder(folder),
                into.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            println!("Backed up to {}", written.display());
            Ok(())
        }
        setup::BackupOf::Local { into } => local_backup(env, &into, db, config),
    }
}

pub async fn status(env: &Env, serve_args: ServeArgs) -> Result<(), String> {
    let unfinished = read_unfinished(env);
    let recorded = read_setup(env)?;
    let unrecorded = if setup::status_looks_for_unrecorded(recorded.as_ref(), &unfinished) {
        read_unrecorded(env, serve_args).map_err(|(_, e)| e)
    } else {
        Ok(None)
    };
    let found_unfinished = unfinished.as_ref().ok().and_then(Option::as_ref);
    let found_unrecorded = unrecorded.as_ref().ok().and_then(Option::as_ref);
    if let Some(path) = local_web_key(env, found_unfinished, found_unrecorded)? {
        println!(
            "Web key: {} (read with `kkt key show --web`)",
            path.display()
        );
    }
    if let Some(path) = local_operator_key(env, found_unfinished, found_unrecorded, KeyKind::Admin)?
        .filter(|path| path.exists())
    {
        println!(
            "Admin key: {} (read with `kkt key show --admin`; agents holding it use /mcp-admin)",
            path.display()
        );
        let key = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read key file {}: {e}", path.display()))?;
        if let Some(line) = admin_name_line(&path, &key, "Admin name")? {
            println!("{line}");
        }
    }
    let said = setup::status(
        recorded.as_ref(),
        &unfinished,
        &unrecorded,
        docker_folder(env),
    );
    for line in &said.lines {
        println!("{line}");
    }
    match said.then? {
        setup::StatusThen::Service => {
            print_configured_name_pack(env)?;
            service::run(env, ServiceAction::Status).await
        }
        setup::StatusThen::Docker(folder) => {
            let folder = kkt_container::Folder(folder);
            let state = kkt_container::status(&kkt_container::OsHost, &folder)
                .map_err(|e| e.to_string())?;
            match &state.name_pack {
                Ok(pack) => println!("Name pack: {pack}"),
                Err(reason) => println!("Name pack: unavailable ({reason})"),
            }
            println!("Docker folder: {}", folder.path().display());
            println!(
                "Settings present: {}\nRunning: {}\nVersion: {}\nTailscale: {}",
                state.files_present,
                state.running,
                state.version.as_deref().unwrap_or("unknown"),
                state.tailscale
            );
            match &state.login {
                Some(kkt_container::Login::LoggedIn) => println!("Tailscale login: logged in"),
                Some(kkt_container::Login::NeedsLogin) => println!(
                    "Tailscale login: needed. The machine's key expired, or it was logged out, so it is off the tailnet. Run `kkt setup` to log in again."
                ),
                Some(kkt_container::Login::Unreachable) => {
                    println!("Tailscale login: unknown; the Tailscale container does not answer")
                }
                Some(kkt_container::Login::Other(state)) => println!("Tailscale login: {state}"),
                None => {}
            }
            Ok(())
        }
        setup::StatusThen::Done => Ok(()),
    }
}

/// Status reads local configuration, not a running server's command line.
fn print_configured_name_pack(env: &Env) -> Result<(), String> {
    let (_, options) = read_config(env, None)?;
    println!(
        "Name pack (configured; serve flags may override): {}",
        options.name_pack_name()
    );
    Ok(())
}

pub async fn upgrade(
    env: &Env,
    serve_args: ServeArgs,
    allow_downgrade: bool,
) -> Result<(), String> {
    let record = recorded_setup(env, serve_args)?;
    let running = match setup::running_version(&record)? {
        setup::RunningVersion::DockerSettings(folder) => {
            let path = folder.join(kkt_container::ENV_FILE);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            kkt_container::version_in_env(&text)
                .ok_or_else(|| format!("{} has no KKT_VERSION", path.display()))?
        }
        setup::RunningVersion::Recorded(version) => version,
    };
    let plan = setup::upgrade_plan(&record, &running, Build::CURRENT.version, allow_downgrade)?;
    let Some(how) = plan.replace else {
        println!("{}", plan.said);
        return Ok(());
    };
    match how {
        setup::Replace::Docker { folder, key_file } => {
            kkt_container::upgrade(
                &kkt_container::OsHost,
                &SystemClock,
                &kkt_container::Folder(folder),
                Build::CURRENT.version,
                key_file.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        setup::Replace::Service => {
            // Validate the replacement service before doing anything. Its
            // program is current_exe, i.e. the binary just installed by the user.
            let plan = service_plan(env, None, None, None, false)?;
            let folder = env
                .config_dir()
                .ok_or("set HOME to find the backup folder")?
                .join(config::SERVICE_BACKUP_FOLDER);
            local_backup(env, &folder, None, None)?;
            if let Some(key) = &plan.new_web_key {
                new_key_file(key, false)?;
                println!(
                    "Created a new web key at {}. Read it with `kkt key show --web`.",
                    key.display()
                );
            }
            let manager = Manager::for_this_os(env).map_err(|e| e.to_string())?;
            for line in manager
                .install(&ServiceSystem, &SystemClock, &plan.spec)
                .await
                .map_err(unrecorded_upgrade)?
            {
                println!("{line}");
            }
        }
    }
    write_setup_record(env, &plan.record)?;
    println!("{}", plan.said);
    Ok(())
}

/// What `kkt upgrade` says when installing the service fails. The record
/// is written only after the install, so when the new service would not
/// load (`kkt service start` finishes that), the upgrade is recorded only
/// when `kkt upgrade` is run again.
fn unrecorded_upgrade(error: ServiceError) -> String {
    match error {
        ServiceError::ReloadFailed { .. } => format!(
            "{error} This upgrade is not recorded yet: once the service is loaded, run `kkt upgrade` again to record the new version."
        ),
        other => other.to_string(),
    }
}

pub async fn uninstall(env: &Env, serve_args: ServeArgs) -> Result<(), String> {
    let record = recorded_setup(env, serve_args)?;
    // Clients switched to another server keep using it: the machine stays
    // recorded as its client.
    let stays_client = record.clients.clone().map(|server| setup::Record {
        mode: setup::Mode::Client,
        written_by: Build::CURRENT.version.to_string(),
        url: server.url,
        key_file: server.key_file,
        clients: None,
    });
    match record.mode {
        setup::Mode::Docker(install) => {
            kkt_container::down(
                &kkt_container::OsHost,
                &kkt_container::Folder(install.folder),
                false,
            )
            .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { tailscale: shared } => {
            if shared {
                // The default config file's address, as `serve` would bind.
                kkt_remote_access::stop(&kkt_remote_access::OsHost, serve_bind(env)?)?;
            }
            service::run(env, ServiceAction::Uninstall).await?;
        }
        setup::Mode::Client => {}
    }
    if let Some(client) = stays_client {
        write_setup_record(env, &client)?;
        println!(
            "Removed this machine's server. Data, keys, backups and client configurations were \
             kept, and this machine stays set up as a client of {}.",
            client.url
        );
        return Ok(());
    }
    let path = setup::record_path(env).ok_or("set HOME to find the setup record")?;
    std::fs::remove_file(&path).map_err(|e| format!("could not remove {}: {e}", path.display()))?;
    println!(
        "Removed this machine's setup. Data, keys, backups and client configurations were kept."
    );
    Ok(())
}

/// `kkt backup`: backs up the database `kkt serve` would use (or `db`),
/// with the config file and existing keys it would read, into `folder`.
fn local_backup(
    env: &Env,
    folder: &Path,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let (config_file, options) = read_config(env, config)?;
    let database = existing_db(env, db, options.db.clone())?;
    let default_access_key = env.access_key_file();
    let setup::BackupKeys {
        access_key,
        web_key,
        admin_key,
        admin_name,
    } = setup::backup_keys(
        setup::AccessKeyFrom {
            configured: options.access_key_file.as_deref(),
            no_access_key: options.no_access_key,
            default: default_access_key.as_deref(),
        },
        &mut |path| path.try_exists().map_err(|e| e.to_string()),
    )?;
    let contents = backup::Contents {
        database: &database,
        config: config_file.as_deref(),
        access_key: access_key.as_deref(),
        web_key: web_key.as_deref(),
        admin_key: admin_key.as_deref(),
        admin_name: admin_name.as_deref(),
    };
    let written = write_backup(folder, &SystemClock, &contents)?;
    println!("Backed up to {}", written.display());
    println!("  database {}", database.display());
    if let Some(path) = &config_file {
        println!("  config file {}", path.display());
    }
    match &access_key {
        Some(path) => println!("  access key {}", path.display()),
        None => println!("  no access key"),
    }
    if let Some(path) = &web_key {
        println!("  web key {}", path.display());
    }
    if let Some(path) = &admin_key {
        println!("  admin key {}", path.display());
    }
    if let Some(path) = &admin_name {
        println!("  admin name {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn an_upgrade_whose_service_would_not_load_says_to_run_it_again() {
        let error = ServiceError::ReloadFailed {
            command: "launchctl bootstrap gui/501 kkt.plist".to_string(),
            detail: "Bootstrap failed: 5: Input/output error".to_string(),
            tries: 141,
            still_removing_for: Some(Duration::from_secs(35)),
        };
        let said = unrecorded_upgrade(error);
        assert!(
            said.starts_with(
                "`launchctl bootstrap gui/501 kkt.plist` failed on each of 141 tries over 35 seconds, \
                 while launchd was still removing the old service: Bootstrap failed: 5: \
                 Input/output error. kashkeshet is stopped: "
            ),
            "{said}"
        );
        assert!(
            said.contains("run `kkt service start` to load the new one"),
            "{said}"
        );
        assert!(
            said.ends_with("run `kkt upgrade` again to record the new version."),
            "{said}"
        );
        // Other install errors are said as they are.
        assert_eq!(
            unrecorded_upgrade(ServiceError::NotInstalled),
            ServiceError::NotInstalled.to_string()
        );
    }
}
