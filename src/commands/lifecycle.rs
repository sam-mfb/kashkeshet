//! An install's lifecycle: `kkt status`, `kkt upgrade`, `kkt uninstall`
//! and `kkt backup`, and the backup `kkt serve` makes before it upgrades
//! the database.

use crate::commands::lanes::existing_db;
use crate::commands::service::{self, ServiceAction, service_plan};
use crate::machine::container::ContainerHost;
use crate::machine::keys::{
    KeyKind, admin_name_line, local_operator_key, local_web_key, new_key_file,
};
use crate::machine::record::{
    docker_folder, read_setup, read_unfinished, read_unrecorded, recorded_setup, write_setup_record,
};
use crate::machine::remote_access::RemoteHost;
use crate::machine::service::ServiceSystem;
use crate::machine::settings::{ContainerConfigs, ServeArgs, read_config, serve_bind};
use kkt_backup as backup;
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_service::{Manager, ServiceError};
use kkt_settings::Limits;
use kkt_setup as setup;
use kkt_terminal_os::Terminal;
use std::path::{Path, PathBuf};

/// Writes a backup of `contents` into `folder`, copying the database with
/// SQLite's online backup.
fn write_backup(
    folder: &Path,
    clock: &dyn Clock,
    contents: &backup::Contents,
) -> Result<PathBuf, String> {
    let layout = backup::layout(clock.now(), Build::CURRENT.version, contents);
    kkt_backup_os::write(folder, &layout, crate::machine::database::copy)
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
    let folder = setup::upgrade_backup_folder(database);
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
    match kkt_backup_os::prune(&folder, keep) {
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
    platform: Platform,
    folder: Option<&Path>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let mode = read_setup(env)?.map(|record| record.mode);
    match setup::backup_of(mode.as_ref(), folder, db.is_some() || config.is_some())? {
        setup::BackupOf::Docker { folder, into } => {
            let written = kkt_container::backup(
                &ContainerHost,
                &kkt_container::Folder(folder),
                into.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            Terminal.output(format_args!("Backed up to {}\n", written.display()));
            Ok(())
        }
        setup::BackupOf::Local { into } => local_backup(env, platform, &into, db, config),
    }
}

pub async fn status(env: &Env, platform: Platform, serve_args: ServeArgs) -> Result<(), String> {
    let unfinished = read_unfinished(env);
    let recorded = read_setup(env)?;
    let unrecorded = if setup::status_looks_for_unrecorded(recorded.as_ref(), &unfinished) {
        read_unrecorded(env, platform, serve_args).map_err(|(_, e)| e)
    } else {
        Ok(None)
    };
    let found_unfinished = unfinished.as_ref().ok().and_then(Option::as_ref);
    let found_unrecorded = unrecorded.as_ref().ok().and_then(Option::as_ref);
    if let Some(path) = local_web_key(env, platform, found_unfinished, found_unrecorded)? {
        Terminal.output(format_args!("{}", setup::web_key_line(&path)));
    }
    if let Some(path) = local_operator_key(
        env,
        platform,
        found_unfinished,
        found_unrecorded,
        KeyKind::Admin,
    )?
    .filter(|path| kkt_admission_os::KeyFiles::exists(path))
    {
        Terminal.output(format_args!("{}", setup::admin_key_line(&path)));
        let key = kkt_admission_os::KeyFiles::read(&path)
            .map_err(|e| format!("could not read key file {}: {}", path.display(), e.error))?;
        if let Some(line) = admin_name_line(&path, &key, "Admin name")? {
            Terminal.output(format_args!("{line}\n"));
        }
    }
    let said = setup::status(
        recorded.as_ref(),
        &unfinished,
        &unrecorded,
        docker_folder(env, platform),
    );
    for line in &said.lines {
        Terminal.output(format_args!("{line}\n"));
    }
    match said.then? {
        setup::StatusThen::Service => {
            print_configured_name_pack(env)?;
            service::run(env, platform, ServiceAction::Status).await
        }
        setup::StatusThen::Docker(folder) => {
            let folder = kkt_container::Folder(folder);
            let state = kkt_container::status(
                &ContainerHost,
                &folder,
                &ContainerConfigs,
                Limits::DEFAULT.name_pack.name(),
            )
            .map_err(|e| e.to_string())?;
            for line in setup::docker_status_lines(&state, folder.path()) {
                Terminal.output(format_args!("{line}"));
            }
            Ok(())
        }
        setup::StatusThen::Done => Ok(()),
    }
}

/// Status reads local configuration, not a running server's command line.
fn print_configured_name_pack(env: &Env) -> Result<(), String> {
    let (_, options) = read_config(env, None)?;
    Terminal.output(format_args!(
        "{}",
        setup::configured_name_pack_line(options.name_pack_name())
    ));
    Ok(())
}

pub async fn upgrade(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    allow_downgrade: bool,
) -> Result<(), String> {
    let record = recorded_setup(env, platform, serve_args)?;
    let running = match setup::running_version(&record)? {
        setup::RunningVersion::DockerSettings(folder) => {
            let path = folder.join(kkt_container::ENV_FILE);
            let text = kkt_container::docker_env(&ContainerHost, &folder)?;
            kkt_container::version_in_env(&text)
                .ok_or_else(|| format!("{} has no KKT_VERSION", path.display()))?
        }
        setup::RunningVersion::Recorded(version) => version,
    };
    let plan = setup::upgrade_plan(&record, &running, Build::CURRENT.version, allow_downgrade)?;
    let Some(how) = plan.replace else {
        Terminal.output(format_args!("{}\n", plan.said));
        return Ok(());
    };
    match how {
        setup::Replace::Docker { folder, key_file } => {
            kkt_container::upgrade(
                &ContainerHost,
                &SystemClock,
                kkt_container::Settings::DEFAULT,
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
            let plan = service_plan(env, platform, None, None, None, false)?;
            let folder = setup::service_backup_folder(kkt_config_file::config_dir(env))?;
            local_backup(env, platform, &folder, None, None)?;
            if let Some(key) = &plan.new_web_key {
                new_key_file(key, false)?;
                Terminal.output(format_args!(
                    "Created a new web key at {}. Read it with `kkt key show --web`.\n",
                    key.display()
                ));
            }
            let manager = Manager::for_this_os(env).map_err(|e| e.to_string())?;
            for line in manager
                .install(
                    &ServiceSystem,
                    &SystemClock,
                    kkt_service::Settings::DEFAULT,
                    &plan.spec,
                )
                .await
                .map_err(unrecorded_upgrade)?
            {
                Terminal.output(format_args!("{line}\n"));
            }
        }
    }
    write_setup_record(env, &plan.record)?;
    Terminal.output(format_args!("{}\n", plan.said));
    Ok(())
}

/// What `kkt upgrade` says when installing the service fails. The record
/// is written only after the install, so when the new service would not
/// load (`kkt service start` finishes that), the upgrade is recorded only
/// when `kkt upgrade` is run again.
fn unrecorded_upgrade(error: ServiceError) -> String {
    setup::service_upgrade_error(
        &error.to_string(),
        matches!(error, ServiceError::ReloadFailed { .. }),
    )
}

pub async fn uninstall(env: &Env, platform: Platform, serve_args: ServeArgs) -> Result<(), String> {
    let record = recorded_setup(env, platform, serve_args)?;
    let stays_client = setup::uninstall_record(&record, Build::CURRENT.version);
    match record.mode {
        setup::Mode::Docker(install) => {
            kkt_container::down(
                &ContainerHost,
                &kkt_container::Folder(install.folder),
                false,
            )
            .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { tailscale: shared } => {
            if shared {
                // The default config file's address, as `serve` would bind.
                kkt_remote_access::stop(&RemoteHost, serve_bind(env, platform)?)?;
            }
            service::run(env, platform, ServiceAction::Uninstall).await?;
        }
        setup::Mode::Client => {}
    }
    if let Some(client) = stays_client {
        write_setup_record(env, &client)?;
        Terminal.output(format_args!("{}", setup::uninstall_line(Some(&client))));
        return Ok(());
    }
    let config_dir = kkt_config_file::config_dir(env).ok_or("set HOME to find the setup record")?;
    kkt_setup_os::remove_record(&config_dir)?;
    Terminal.output(format_args!("{}", setup::uninstall_line(None)));
    Ok(())
}

/// `kkt backup`: backs up the database `kkt serve` would use (or `db`),
/// with the config file and existing keys it would read, into `folder`.
fn local_backup(
    env: &Env,
    platform: Platform,
    folder: &Path,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let (config_file, options) = read_config(env, config)?;
    let database = existing_db(env, platform, db, options.db.clone())?;
    let default_access_key = crate::machine::settings::default_access_key(env);
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
    Terminal.output(format_args!("Backed up to {}\n", written.display()));
    Terminal.output(format_args!("  database {}\n", database.display()));
    if let Some(path) = &config_file {
        Terminal.output(format_args!("  config file {}\n", path.display()));
    }
    match &access_key {
        Some(path) => Terminal.output(format_args!("  access key {}\n", path.display())),
        None => Terminal.output(format_args!("  no access key\n")),
    }
    if let Some(path) = &web_key {
        Terminal.output(format_args!("  web key {}\n", path.display()));
    }
    if let Some(path) = &admin_key {
        Terminal.output(format_args!("  admin key {}\n", path.display()));
    }
    if let Some(path) = &admin_name {
        Terminal.output(format_args!("  admin name {}\n", path.display()));
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
