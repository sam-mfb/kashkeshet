//! An install's lifecycle: `kkt status`, `kkt upgrade`, `kkt uninstall`
//! and `kkt backup`. The backup `kkt serve` makes before it upgrades the
//! database is the program's, beside its serving code.

use crate::keys::{KeyKind, admin_name_line, local_operator_key, local_web_key, new_key_file};
use crate::lanes::existing_db;
use crate::record::{
    docker_folder, read_setup, read_unfinished, read_unrecorded, recorded_setup, write_setup_record,
};
use crate::service::{self, ServiceAction, ServiceInstall, service_plan};
use crate::{OperateParts, ServeFacts, ServiceDefaults};
use kkt_backup as backup;
use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_service::{Manager, ServiceError};
use kkt_setup as setup;
use kkt_setup::connection::ConnectionSource;
use std::path::{Path, PathBuf};

/// `kkt backup`: backs up the Docker server through its container, or the
/// database `kkt serve` would use (or `db`), with the config file and
/// existing keys it would read, into `folder` or the default folder.
pub fn backup<D>(
    parts: &OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    folder: Option<&Path>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let mode = read_setup(env, parts.setup_files)?.map(|record| record.mode);
    match setup::backup_of(mode.as_ref(), folder, db.is_some() || config.is_some())? {
        setup::BackupOf::Docker { folder, into } => {
            let written = kkt_container::backup(
                parts.container,
                &kkt_container::Folder(folder),
                into.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            parts
                .output
                .output(format_args!("Backed up to {}\n", written.display()));
            Ok(())
        }
        setup::BackupOf::Local { into } => local_backup(parts, &into, db, config),
    }
}

/// `kkt status`: how this machine is set up, its web and admin keys, and
/// then the service's or the Docker server's state.
pub async fn status<D: ServiceDefaults>(
    parts: &OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<(), String> {
    let unfinished = read_unfinished(env, parts.setup_files, parts.container, parts.service);
    let recorded = read_setup(env, parts.setup_files)?;
    let unrecorded = if setup::status_looks_for_unrecorded(recorded.as_ref(), &unfinished) {
        read_unrecorded(parts, env, platform).map_err(|(_, e)| e)
    } else {
        Ok(None)
    };
    let found_unfinished = unfinished.as_ref().ok().and_then(Option::as_ref);
    let found_unrecorded = unrecorded.as_ref().ok().and_then(Option::as_ref);
    if let Some(path) = local_web_key(parts, env, found_unfinished, found_unrecorded)? {
        parts
            .output
            .output(format_args!("{}", setup::web_key_line(&path)));
    }
    if let Some(path) = local_operator_key(
        parts,
        env,
        found_unfinished,
        found_unrecorded,
        KeyKind::Admin,
    )?
    .filter(|path| parts.key_files.exists(path))
    {
        parts
            .output
            .output(format_args!("{}", setup::admin_key_line(&path)));
        let key = parts
            .key_files
            .read(&path)
            .map_err(|e| format!("could not read key file {}: {e}", path.display()))?;
        if let Some(line) = admin_name_line(parts.key_files, &path, &key, "Admin name")? {
            parts.output.output(format_args!("{line}\n"));
        }
    }
    let said = setup::status(
        recorded.as_ref(),
        &unfinished,
        &unrecorded,
        docker_folder(env, platform),
    );
    for line in &said.lines {
        parts.output.output(format_args!("{line}\n"));
    }
    match said.then? {
        setup::StatusThen::Service => {
            // Status reads local configuration, not a running server's
            // command line.
            let config = parts.serve.config(None)?;
            parts.output.output(format_args!(
                "{}",
                setup::configured_name_pack_line(&config.name_pack)
            ));
            service::run(parts, env, ServiceAction::Status).await
        }
        setup::StatusThen::Docker(folder) => {
            let folder = kkt_container::Folder(folder);
            let state = kkt_container::status(
                parts.container,
                &folder,
                parts.container_configs,
                kkt_rules::Settings::DEFAULT.name_pack.name(),
            )
            .map_err(|e| e.to_string())?;
            for line in setup::docker_status_lines(&state, folder.path()) {
                parts.output.output(format_args!("{line}"));
            }
            Ok(())
        }
        setup::StatusThen::Done => Ok(()),
    }
}

/// `kkt upgrade`: replaces the recorded server with this version, the
/// Docker server's image or the service's program, backing a service's
/// files up first, then records the new version.
pub async fn upgrade<D: ServiceDefaults>(
    parts: &OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
    allow_downgrade: bool,
) -> Result<(), String> {
    let record = recorded_setup(parts, env, platform)?;
    let running = match setup::running_version(&record)? {
        setup::RunningVersion::DockerSettings(folder) => {
            let path = folder.join(kkt_container::ENV_FILE);
            let text = kkt_container::docker_env(parts.container, &folder)?;
            kkt_container::version_in_env(&text)
                .ok_or_else(|| format!("{} has no KKT_VERSION", path.display()))?
        }
        setup::RunningVersion::Recorded(version) => version,
    };
    let plan = setup::upgrade_plan(&record, &running, Build::CURRENT.version, allow_downgrade)?;
    let Some(how) = plan.replace else {
        parts.output.output(format_args!("{}\n", plan.said));
        return Ok(());
    };
    match how {
        setup::Replace::Docker { folder, key_file } => {
            kkt_container::upgrade(
                parts.container,
                parts.clock.as_ref(),
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
            // program is this one, the binary just installed by the user.
            let service = service_plan(parts, ServiceInstall::NONE)?;
            let folder = setup::service_backup_folder(kkt_config_file::config_dir(
                env.config_home().as_deref(),
            ))?;
            local_backup(parts, &folder, None, None)?;
            if let Some(key) = &service.new_web_key {
                new_key_file(parts.key_files, key, false)?;
                parts.output.output(format_args!(
                    "Created a new web key at {}. Read it with `kkt key show --web`.\n",
                    key.display()
                ));
            }
            let manager = Manager::for_this_os(env.home.as_deref(), env.config_home().as_deref())
                .map_err(|e| e.to_string())?;
            for line in manager
                .install(
                    parts.service,
                    parts.clock.as_ref(),
                    kkt_service::Settings::DEFAULT,
                    &service.spec,
                )
                .await
                .map_err(unrecorded_upgrade)?
            {
                parts.output.output(format_args!("{line}\n"));
            }
        }
    }
    write_setup_record(env, parts.setup_files, &plan.record)?;
    parts.output.output(format_args!("{}\n", plan.said));
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

/// `kkt uninstall`: stops and removes the recorded server, then removes the
/// record, or keeps it as a client's when the machine stays one.
pub async fn uninstall<D: ServiceDefaults>(
    parts: &OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<(), String> {
    let record = recorded_setup(parts, env, platform)?;
    let stays_client = setup::uninstall_record(&record, Build::CURRENT.version);
    match record.mode {
        setup::Mode::Docker(install) => {
            kkt_container::down(
                parts.container,
                &kkt_container::Folder(install.folder),
                false,
            )
            .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { tailscale: shared } => {
            stop_sharing(shared, parts.serve, parts.remote)?;
            service::run(parts, env, ServiceAction::Uninstall).await?;
        }
        setup::Mode::Client => {}
    }
    if let Some(client) = stays_client {
        write_setup_record(env, parts.setup_files, &client)?;
        parts
            .output
            .output(format_args!("{}", setup::uninstall_line(Some(&client))));
        return Ok(());
    }
    let config_dir = kkt_config_file::config_dir(env.config_home().as_deref())
        .ok_or("set HOME to find the setup record")?;
    parts.setup_files.remove_record(&config_dir)?;
    parts
        .output
        .output(format_args!("{}", setup::uninstall_line(None)));
    Ok(())
}

/// Stops sharing the service over Tailscale, if it is `shared`. Only then
/// does it ask where the server listens: the default config file's address,
/// as `serve` would bind.
fn stop_sharing(
    shared: bool,
    serve: &dyn ServeFacts,
    remote: &dyn kkt_remote_access::Host,
) -> Result<(), String> {
    if shared {
        kkt_remote_access::stop(remote, serve.bind()?)?;
    }
    Ok(())
}

/// Backs up the database `kkt serve` would use (or `db`), with the config
/// file and existing keys it would read, into `folder`.
fn local_backup<D>(
    parts: &OperateParts<'_, impl ConnectionSource, D>,
    folder: &Path,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let config = parts.serve.config(config)?;
    let database = existing_db(parts, db, config.db.clone())?;
    let default_access_key = parts.serve.default_access_key();
    let setup::BackupKeys {
        access_key,
        web_key,
        admin_key,
        admin_name,
    } = setup::backup_keys(
        setup::AccessKeyFrom {
            configured: config.access_key_file.as_deref(),
            no_access_key: config.no_access_key,
            default: default_access_key.as_deref(),
        },
        &mut |path| parts.key_files.try_exists(path).map_err(|e| e.to_string()),
    )?;
    let contents = backup::Contents {
        database: &database,
        config: config.file.as_deref(),
        access_key: access_key.as_deref(),
        web_key: web_key.as_deref(),
        admin_key: admin_key.as_deref(),
        admin_name: admin_name.as_deref(),
    };
    let layout = backup::layout(parts.clock.now(), Build::CURRENT.version, &contents);
    let written = parts.backups.write(folder, &layout)?;
    let out = parts.output;
    out.output(format_args!("Backed up to {}\n", written.display()));
    out.output(format_args!("  database {}\n", database.display()));
    if let Some(path) = &config.file {
        out.output(format_args!("  config file {}\n", path.display()));
    }
    match &access_key {
        Some(path) => out.output(format_args!("  access key {}\n", path.display())),
        None => out.output(format_args!("  no access key\n")),
    }
    if let Some(path) = &web_key {
        out.output(format_args!("  web key {}\n", path.display()));
    }
    if let Some(path) = &admin_key {
        out.output(format_args!("  admin key {}\n", path.display()));
    }
    if let Some(path) = &admin_name {
        out.output(format_args!("  admin name {}\n", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
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

    /// Serving facts that log each question and answer only `bind`.
    #[derive(Default)]
    struct Asked(RefCell<Vec<&'static str>>);

    impl ServeFacts for Asked {
        fn config(&self, _: Option<PathBuf>) -> Result<crate::Config, String> {
            self.0.borrow_mut().push("config");
            Err("not asked here".into())
        }
        fn configured_access_key(&self) -> Result<Option<PathBuf>, String> {
            self.0.borrow_mut().push("configured_access_key");
            Err("not asked here".into())
        }
        fn database(&self, _: Option<PathBuf>, _: Option<PathBuf>) -> Result<PathBuf, String> {
            self.0.borrow_mut().push("database");
            Err("not asked here".into())
        }
        fn bind(&self) -> Result<std::net::SocketAddr, String> {
            self.0.borrow_mut().push("bind");
            Ok("127.0.0.1:7420".parse().expect("an address"))
        }
        fn default_access_key(&self) -> Option<PathBuf> {
            self.0.borrow_mut().push("default_access_key");
            None
        }
        fn service_settings(
            &self,
            _: Option<PathBuf>,
            _: kkt_service::ServiceSettings,
        ) -> Result<crate::ServiceInputs<'_>, String> {
            self.0.borrow_mut().push("service_settings");
            Err("not asked here".into())
        }
    }

    /// A `tailscale` that logs each command and fails it.
    #[derive(Default)]
    struct Tailscale(RefCell<Vec<Vec<String>>>);

    impl kkt_remote_access::Host for Tailscale {
        fn run(&self, args: &[String]) -> Result<kkt_remote_access::Output, String> {
            self.0.borrow_mut().push(args.to_vec());
            Err("tailscale is not here".into())
        }
    }

    #[test]
    fn uninstalling_a_service_not_shared_over_tailscale_never_asks() {
        let (serve, remote) = (Asked::default(), Tailscale::default());
        assert_eq!(stop_sharing(false, &serve, &remote), Ok(()));
        assert!(serve.0.borrow().is_empty(), "{:?}", serve.0.borrow());
        assert!(remote.0.borrow().is_empty(), "{:?}", remote.0.borrow());
    }

    #[test]
    fn uninstalling_a_shared_service_asks_where_it_listens_then_tailscale() {
        let (serve, remote) = (Asked::default(), Tailscale::default());
        assert_eq!(
            stop_sharing(true, &serve, &remote),
            Err("tailscale is not here".into())
        );
        assert_eq!(*serve.0.borrow(), ["bind"]);
        assert_eq!(remote.0.borrow().len(), 1);
    }
}
