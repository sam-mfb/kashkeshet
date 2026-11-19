//! An install's lifecycle: `kkt status`, `kkt upgrade`, `kkt uninstall`
//! and `kkt backup`. The backup `kkt serve` makes before it upgrades the
//! database is the program's, beside its serving code.

use crate::deployment;
use crate::keys::{KeyKind, admin_name_line, local_operator_key, local_web_key, new_key_file};
use crate::record::{
    read_setup, read_unfinished, read_unrecorded, recorded_setup, write_setup_record,
};
use crate::service::{self, ServiceAction, ServiceInstall, service_plan};
use crate::{InstallParts, ServeFacts, ServiceDefaults};
use kkt_backup as backup;
use kkt_bare_service::ServiceError;
use kkt_deployment::connection::ConnectionSource;
use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_setup as setup;
use std::path::{Path, PathBuf};

/// `kkt backup`: backs up the Docker server through its container, or the
/// database `kkt serve` would use (or `db`), with the config file and
/// existing keys it would read, into `folder` or the default folder.
pub fn backup<D>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    folder: Option<&Path>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let mode = read_setup(parts.serve, parts.setup_files)
        .map_err(|e| e.to_string())?
        .map(|record| record.mode);
    match backup::backup_of(
        &server_here(mode.as_ref()),
        folder,
        db.is_some() || config.is_some(),
    )
    .map_err(|e| e.to_string())?
    {
        backup::BackupOf::Docker { folder, into } => {
            let written = parts
                .container
                .backup(&kkt_container::Folder(folder), into.as_deref())
                .map_err(|e| e.to_string())?;
            parts
                .output
                .output(format_args!("Backed up to {}\n", written.display()));
            Ok(())
        }
        backup::BackupOf::Local { into } => local_backup(parts, &into, db, config),
    }
}

/// Where this machine's server is, for a backup, from the recorded `mode`
/// (`None` with no record): with no record, the local database `kkt serve`
/// would use.
fn server_here(mode: Option<&setup::Mode>) -> backup::ServerHere {
    match mode {
        Some(setup::Mode::Docker(install)) => backup::ServerHere::InDocker {
            folder: install.folder.clone(),
        },
        Some(setup::Mode::Client) => backup::ServerHere::Elsewhere,
        Some(setup::Mode::Service { .. }) | None => backup::ServerHere::ThisMachine,
    }
}

/// Whether the key file at `path` is there, or why that cannot be told.
fn key_file_there(files: &dyn kkt_keys::KeyFiles, path: &Path) -> Result<bool, String> {
    files.try_exists(path).map_err(|e| {
        format!(
            "could not tell whether the key file {} is there: {e}",
            path.display()
        )
    })
}

/// `kkt status`: how this machine is set up, its web and admin keys, and
/// then the service's or the Docker server's state.
pub async fn status<D: ServiceDefaults>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<(), String> {
    // The reasons a read failed are kept as they are, to say in the lines.
    let unfinished = read_unfinished(
        parts.serve,
        parts.setup_files,
        parts.container,
        parts.service,
    )
    .map_err(kkt_deployment::Unknown::new);
    let recorded = read_setup(parts.serve, parts.setup_files).map_err(|e| e.to_string())?;
    let unrecorded = if kkt_deployment::status_looks_for_unrecorded(recorded.as_ref(), &unfinished)
    {
        read_unrecorded(parts, env, platform).map_err(|(_, e)| kkt_deployment::Unknown::new(e))
    } else {
        Ok(None)
    };
    let found_unfinished = unfinished.as_ref().ok().and_then(Option::as_ref);
    let found_unrecorded = unrecorded.as_ref().ok().and_then(Option::as_ref);
    if let Some(path) =
        local_web_key(parts, found_unfinished, found_unrecorded).map_err(|e| e.to_string())?
    {
        parts.output.output(format_args!("{}", web_key_line(&path)));
    }
    let admin_key = local_operator_key(parts, found_unfinished, found_unrecorded, KeyKind::Admin)
        .map_err(|e| e.to_string())?;
    let admin_key = match admin_key {
        Some(path) if !key_file_there(parts.key_files, &path)? => None,
        other => other,
    };
    if let Some(path) = admin_key {
        parts
            .output
            .output(format_args!("{}", admin_key_line(&path)));
        let key = parts
            .key_files
            .read(&path)
            .map_err(|e| format!("could not read key file {}: {e}", path.display()))?;
        if let Some(line) = admin_name_line(parts.key_files, &path, &key, "Admin name")
            .map_err(|e| e.to_string())?
        {
            parts.output.output(format_args!("{line}\n"));
        }
    }
    let said = kkt_deployment::status(
        recorded.as_ref().map(deployment::recorded).as_ref(),
        &deployment::setup_not_finished(&unfinished, env, platform),
        &deployment::found_without_record(&unrecorded),
    );
    for line in &said.lines {
        parts.output.output(format_args!("{line}\n"));
    }
    match said.then.map_err(|e| e.to_string())? {
        kkt_deployment::StatusThen::Service => {
            // Status reads local configuration, not a running server's
            // command line.
            let config = parts.serve.config(None).map_err(|e| e.to_string())?;
            parts.output.output(format_args!(
                "{}",
                configured_name_pack_line(&config.name_pack)
            ));
            service::run(parts, ServiceAction::Status)
                .await
                .map_err(|e| e.to_string())
        }
        kkt_deployment::StatusThen::Docker(folder) => {
            let folder = kkt_container::Folder(folder);
            let state = parts
                .container
                .status(
                    &folder,
                    parts.container_configs,
                    &parts.serve.default_name_pack(),
                    &parts.serve.config_file_in_home(),
                )
                .map_err(|e| e.to_string())?;
            for line in docker_status_lines(&state, folder.path()) {
                parts.output.output(format_args!("{line}"));
            }
            Ok(())
        }
        kkt_deployment::StatusThen::Done => Ok(()),
    }
}

/// `kkt upgrade`: replaces the recorded server with this version, the
/// Docker server's image or the service's program, backing a service's
/// files up first, then records the new version.
pub async fn upgrade<D: ServiceDefaults>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
    allow_downgrade: bool,
) -> Result<(), String> {
    let record = recorded_setup(parts, env, platform).map_err(|e| e.to_string())?;
    let recorded = deployment::recorded(&record);
    let running = match kkt_deployment::running_version(&recorded).map_err(|e| e.to_string())? {
        kkt_deployment::RunningVersion::DockerSettings(folder) => {
            deployment::server_version(&parts.container.version(&kkt_container::Folder(folder))?)
        }
        kkt_deployment::RunningVersion::Recorded(version) => version,
    };
    let plan =
        kkt_deployment::upgrade_plan(&recorded, &running, Build::CURRENT.version, allow_downgrade)
            .map_err(|e| e.to_string())?;
    let Some(how) = plan.replace else {
        parts.output.output(format_args!("{}\n", plan.said));
        return Ok(());
    };
    match how {
        kkt_deployment::Replace::Docker { folder, key_file } => {
            parts
                .container
                .upgrade(
                    parts.clock.as_ref(),
                    &kkt_container::Folder(folder),
                    Build::CURRENT.version,
                    key_file
                        .as_deref()
                        .map(kkt_deployment::docker_key_copies)
                        .as_ref(),
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        kkt_deployment::Replace::Service => {
            // Validate the replacement service before doing anything. Its
            // program is this one, the binary just installed by the user.
            let service = service_plan(parts, ServiceInstall::NONE).map_err(|e| e.to_string())?;
            let folder = backup::service_backup_folder(parts.serve.config_dir())
                .map_err(|e| e.to_string())?;
            local_backup(parts, &folder, None, None)?;
            if let Some(key) = &service.new_web_key {
                new_key_file(parts.key_files, parts.entropy, key, false)
                    .map_err(|e| e.to_string())?;
                parts.output.output(format_args!(
                    "Created a new web key at {}. Read it with `kkt key show --web`.\n",
                    key.display()
                ));
            }
            parts.service.ready().map_err(|e| e.to_string())?;
            for line in parts
                .service
                .install(
                    parts.clock.as_ref(),
                    kkt_bare_service::Settings::DEFAULT,
                    &service.spec,
                )
                .await
                .map_err(unrecorded_upgrade)?
            {
                parts.output.output(format_args!("{line}\n"));
            }
        }
    }
    // The record now names the version the server runs.
    let mut record = record;
    record.written_by = Build::CURRENT.version.to_string();
    write_setup_record(parts.serve, parts.setup_files, &record).map_err(|e| e.to_string())?;
    parts.output.output(format_args!("{}\n", plan.said));
    Ok(())
}

/// What `kkt upgrade` says when installing the service fails. The record
/// is written only after the install, so when the new service would not
/// load (`kkt service start` finishes that), the upgrade is recorded only
/// when `kkt upgrade` is run again.
fn unrecorded_upgrade(error: ServiceError) -> String {
    service_upgrade_error(
        &error.to_string(),
        matches!(error, ServiceError::ReloadFailed { .. }),
    )
}

/// `kkt uninstall`: stops and removes the recorded server, then removes the
/// record, or keeps it as a client's when the machine stays one.
pub async fn uninstall<D: ServiceDefaults>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    env: &Env,
    platform: Platform,
) -> Result<(), String> {
    let record = recorded_setup(parts, env, platform).map_err(|e| e.to_string())?;
    let stays_client = uninstall_record(&record, Build::CURRENT.version);
    match record.mode {
        setup::Mode::Docker(install) => {
            parts
                .container
                .down(&kkt_container::Folder(install.folder), false)
                .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { tailscale: shared } => {
            stop_sharing(shared, parts.serve, parts.remote)?;
            service::run(parts, ServiceAction::Uninstall)
                .await
                .map_err(|e| e.to_string())?;
        }
        setup::Mode::Client => {}
    }
    if let Some(client) = stays_client {
        write_setup_record(parts.serve, parts.setup_files, &client).map_err(|e| e.to_string())?;
        parts
            .output
            .output(format_args!("{}", uninstall_line(Some(&client))));
        return Ok(());
    }
    let config_dir = parts
        .serve
        .config_dir()
        .ok_or("set HOME to find the setup record")?;
    parts
        .setup_files
        .remove_record(&config_dir)
        .map_err(|e| e.to_string())?;
    parts
        .output
        .output(format_args!("{}", uninstall_line(None)));
    Ok(())
}

/// Stops sharing the service over Tailscale, if it is `shared`. Only then
/// does it ask where the server listens: the default config file's address,
/// as `serve` would bind.
fn stop_sharing(
    shared: bool,
    serve: &dyn ServeFacts,
    remote: &dyn kkt_remote_access::RemoteAccess,
) -> Result<(), String> {
    if shared {
        remote
            .stop(serve.bind().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The database file `kkt serve` would use given the config file's `db`,
/// unless `db` names another. It must exist.
fn existing_db<D>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    db: Option<PathBuf>,
    config_db: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let path = parts
        .serve
        .database(db, config_db)
        .map_err(|e| e.to_string())?;
    if !parts.database.exists(&path) {
        return Err(format!(
            "no database at {}; pass --db with the server's database file",
            path.display()
        ));
    }
    Ok(path)
}

/// Backs up the database `kkt serve` would use (or `db`), with the config
/// file and existing keys it would read, into `folder`.
fn local_backup<D>(
    parts: &InstallParts<'_, impl ConnectionSource, D>,
    folder: &Path,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let config = parts.serve.config(config).map_err(|e| e.to_string())?;
    let database = existing_db(parts, db, config.db.clone())?;
    let default_access_key = parts.serve.default_access_key();
    let backup::BackupKeys {
        access_key,
        web_key,
        admin_key,
        admin_name,
    } = backup::backup_keys(
        backup::AccessKeyFrom {
            configured: config.access_key_file.as_deref(),
            no_access_key: config.no_access_key,
            default: default_access_key.as_deref(),
        },
        &mut |path| parts.key_files.try_exists(path),
    )
    .map_err(|e| e.to_string())?;
    let contents = backup::Contents {
        database: &database,
        config: config.file.as_deref(),
        access_key: access_key.as_deref(),
        web_key: web_key.as_deref(),
        admin_key: admin_key.as_deref(),
        admin_name: admin_name.as_deref(),
    };
    let layout = backup::layout(parts.clock.now(), Build::CURRENT.version, &contents);
    let written = parts
        .backups
        .write(folder, &layout)
        .map_err(|e| e.to_string())?;
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

/// Clients switched to another server keep using it after the local install goes.
pub(crate) fn uninstall_record(record: &setup::Record, version: &str) -> Option<setup::Record> {
    record.clients.clone().map(|server| setup::Record {
        mode: setup::Mode::Client,
        written_by: version.to_string(),
        url: server.url,
        key_file: server.key_file,
        clients: None,
    })
}

pub(crate) fn uninstall_line(client: Option<&setup::Record>) -> String {
    match client {
        Some(client) => format!("Removed this machine's server. Data, keys, backups and client configurations were \
             kept, and this machine stays set up as a client of {}.\n", client.url),
        None => "Removed this machine's setup. Data, keys, backups and client configurations were kept.\n".into(),
    }
}

pub(crate) fn web_key_line(path: &Path) -> String {
    format!(
        "Web key: {} (read with `kkt key show --web`)\n",
        path.display()
    )
}

pub(crate) fn admin_key_line(path: &Path) -> String {
    format!(
        "Admin key: {} (read with `kkt key show --admin`; agents holding it use /mcp-admin)\n",
        path.display()
    )
}

pub(crate) fn configured_name_pack_line(name: &str) -> String {
    format!("Name pack (configured; serve flags may override): {name}\n")
}

pub(crate) fn docker_status_lines(
    state: &kkt_containerized_service::State,
    folder: &Path,
) -> Vec<String> {
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
        state.sharing
    ));
    match &state.login {
        Some(kkt_containerized_service::Login::LoggedIn) => lines.push("Tailscale login: logged in\n".into()),
        Some(kkt_containerized_service::Login::NeedsLogin) => lines.push("Tailscale login: needed. The machine's key expired, or it was logged out, so it is off the tailnet. Run `kkt setup` to log in again.\n".into()),
        Some(kkt_containerized_service::Login::Unreachable) => lines.push("Tailscale login: unknown; the Tailscale container does not answer\n".into()),
        Some(kkt_containerized_service::Login::Other(state)) => lines.push(format!("Tailscale login: {state}\n")),
        None => {}
    }
    lines
}

pub(crate) fn service_upgrade_error(error: &str, reload_failed: bool) -> String {
    match reload_failed {
        true => format!(
            "{error} This upgrade is not recorded yet: once the service is loaded, run `kkt upgrade` again to record the new version."
        ),
        false => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_status_uninstall_and_upgrade_lines_are_pinned() {
        // The uninstall record and its lines.
        let server = setup::Record {
            mode: setup::Mode::Service { tailscale: false },
            written_by: "0.9.0".into(),
            url: "http://127.0.0.1:7420/mcp".into(),
            key_file: Some(PathBuf::from("/k/access")),
            clients: Some(setup::Server {
                url: "https://other.example/mcp".into(),
                key_file: Some(PathBuf::from("/k/client")),
            }),
        };
        assert_eq!(
            uninstall_record(&server, "1.0.0"),
            Some(setup::Record {
                mode: setup::Mode::Client,
                written_by: "1.0.0".into(),
                url: "https://other.example/mcp".into(),
                key_file: Some(PathBuf::from("/k/client")),
                clients: None,
            })
        );
        let alone = setup::Record {
            clients: None,
            ..server.clone()
        };
        assert_eq!(uninstall_record(&alone, "1.0.0"), None);
        let client = uninstall_record(&server, "1.0.0").expect("stays a client");
        assert_eq!(
            uninstall_line(Some(&client)),
            "Removed this machine's server. Data, keys, backups and client configurations were \
             kept, and this machine stays set up as a client of https://other.example/mcp.\n"
        );
        assert_eq!(
            uninstall_line(None),
            "Removed this machine's setup. Data, keys, backups and client configurations were kept.\n"
        );
        // The key lines and the name pack line.
        assert_eq!(
            web_key_line(Path::new("/k/web")),
            "Web key: /k/web (read with `kkt key show --web`)\n"
        );
        assert_eq!(
            admin_key_line(Path::new("/k/admin")),
            "Admin key: /k/admin (read with `kkt key show --admin`; agents holding it use /mcp-admin)\n"
        );
        assert_eq!(
            configured_name_pack_line("animals"),
            "Name pack (configured; serve flags may override): animals\n"
        );
        // The upgrade error, with and without a failed reload.
        assert_eq!(service_upgrade_error("It failed.", false), "It failed.");
        assert_eq!(
            service_upgrade_error("It failed.", true),
            "It failed. This upgrade is not recorded yet: once the service is loaded, run `kkt upgrade` again to record the new version."
        );
        // Docker's status lines, every login state.
        let folder = Path::new("/d");
        let mut state = kkt_containerized_service::State {
            files_present: true,
            running: true,
            version: Some("0.9.0".into()),
            name_pack: Ok("animals".into()),
            sharing: true,
            login: Some(kkt_containerized_service::Login::LoggedIn),
        };
        let head = |pack: &str, version: &str| {
            vec![
                format!("Name pack: {pack}\n"),
                "Docker folder: /d\n".to_string(),
                format!(
                    "Settings present: true\nRunning: true\nVersion: {version}\nTailscale: true\n"
                ),
            ]
        };
        let mut want = head("animals", "0.9.0");
        want.push("Tailscale login: logged in\n".into());
        assert_eq!(docker_status_lines(&state, folder), want);
        state.login = Some(kkt_containerized_service::Login::NeedsLogin);
        let mut want = head("animals", "0.9.0");
        want.push("Tailscale login: needed. The machine's key expired, or it was logged out, so it is off the tailnet. Run `kkt setup` to log in again.\n".into());
        assert_eq!(docker_status_lines(&state, folder), want);
        state.login = Some(kkt_containerized_service::Login::Unreachable);
        let mut want = head("animals", "0.9.0");
        want.push("Tailscale login: unknown; the Tailscale container does not answer\n".into());
        assert_eq!(docker_status_lines(&state, folder), want);
        state.login = Some(kkt_containerized_service::Login::Other("Starting".into()));
        let mut want = head("animals", "0.9.0");
        want.push("Tailscale login: Starting\n".into());
        assert_eq!(docker_status_lines(&state, folder), want);
        state.login = None;
        state.name_pack = Err("no config".into());
        state.version = None;
        assert_eq!(
            docker_status_lines(&state, folder),
            vec![
                "Name pack: unavailable (no config)\n".to_string(),
                "Docker folder: /d\n".to_string(),
                "Settings present: true\nRunning: true\nVersion: unknown\nTailscale: true\n"
                    .to_string(),
            ]
        );
    }

    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_backup_acts_on_the_server_the_record_names() {
        let docker = setup::Mode::Docker(setup::DockerSetup {
            folder: PathBuf::from("/data/docker"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        });
        assert_eq!(
            server_here(Some(&docker)),
            backup::ServerHere::InDocker {
                folder: PathBuf::from("/data/docker")
            }
        );
        assert_eq!(
            server_here(Some(&setup::Mode::Client)),
            backup::ServerHere::Elsewhere
        );
        assert_eq!(
            server_here(Some(&setup::Mode::Service { tailscale: true })),
            backup::ServerHere::ThisMachine
        );
        assert_eq!(server_here(None), backup::ServerHere::ThisMachine);
    }

    #[test]
    fn an_upgrade_whose_service_would_not_load_says_to_run_it_again() {
        // As kkt-bare-service-launchd words it.
        let error = ServiceError::ReloadFailed(
            "`launchctl bootstrap gui/501 kkt.plist` failed on each of 141 tries over 35 seconds, \
             while launchd was still removing the old service: Bootstrap failed: 5: \
             Input/output error. kashkeshet is stopped: the old service was unloaded, and the new \
             one is installed but not loaded. Once the old service is gone, run `kkt service \
             start` to load the new one, and `kkt service status` to check."
                .to_string(),
        );
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

    /// Serving facts that log each question and answer only `bind`. The
    /// config places, which read no file, are answered for the home
    /// `/home/k` without a log.
    #[derive(Default)]
    struct Asked(RefCell<Vec<&'static str>>);

    impl ServeFacts for Asked {
        fn config(&self, _: Option<PathBuf>) -> Result<crate::Config, crate::PartError> {
            self.0.borrow_mut().push("config");
            Err("not asked here".into())
        }
        fn configured_access_key(&self) -> Result<Option<PathBuf>, crate::PartError> {
            self.0.borrow_mut().push("configured_access_key");
            Err("not asked here".into())
        }
        fn database(
            &self,
            _: Option<PathBuf>,
            _: Option<PathBuf>,
        ) -> Result<PathBuf, crate::PartError> {
            self.0.borrow_mut().push("database");
            Err("not asked here".into())
        }
        fn bind(&self) -> Result<std::net::SocketAddr, crate::PartError> {
            self.0.borrow_mut().push("bind");
            Ok("127.0.0.1:7420".parse().expect("an address"))
        }
        fn default_access_key(&self) -> Option<PathBuf> {
            self.0.borrow_mut().push("default_access_key");
            None
        }
        fn default_name_pack(&self) -> String {
            self.0.borrow_mut().push("default_name_pack");
            "default".to_string()
        }
        fn config_dir(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/k/.config/kkt"))
        }
        fn config_file(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/k/.config/kkt/config.toml"))
        }
        fn config_file_in_home(&self) -> PathBuf {
            PathBuf::from("kkt/config.toml")
        }
        fn service_settings(
            &self,
            _: Option<PathBuf>,
            _: kkt_deployment::ServiceSettings,
        ) -> Result<crate::ServiceInputs<'_>, crate::PartError> {
            self.0.borrow_mut().push("service_settings");
            Err("not asked here".into())
        }
    }

    /// A sharing provider that logs each operation and fails it.
    #[derive(Default)]
    struct Tailscale(RefCell<Vec<Vec<String>>>);

    impl kkt_remote_access::RemoteAccess for Tailscale {
        fn prepare(&self) -> Result<kkt_remote_access::Node, kkt_remote_access::SharingError> {
            self.0.borrow_mut().push(vec!["prepare".into()]);
            Err(not_here())
        }
        fn serve(&self, _: std::net::SocketAddr) -> Result<(), kkt_remote_access::SharingError> {
            self.0.borrow_mut().push(vec!["serve".into()]);
            Err(not_here())
        }
        fn served(
            &self,
            _: std::net::SocketAddr,
        ) -> Result<Option<kkt_remote_access::Node>, kkt_remote_access::SharingError> {
            self.0.borrow_mut().push(vec!["served".into()]);
            Err(not_here())
        }
        fn stop(&self, _: std::net::SocketAddr) -> Result<(), kkt_remote_access::SharingError> {
            self.0.borrow_mut().push(vec!["stop".into()]);
            Err(not_here())
        }
    }

    fn not_here() -> kkt_remote_access::SharingError {
        kkt_remote_access::SharingError::new(
            kkt_remote_access::Sharing::Unavailable,
            "tailscale is not here",
        )
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

    /// Every effect `uninstall` must not reach: each call is logged and fails.
    #[derive(Default)]
    struct Untouched(RefCell<Vec<&'static str>>);

    impl Untouched {
        fn asked<T>(&self, what: &'static str) -> std::io::Result<T> {
            self.0.borrow_mut().push(what);
            Err(std::io::Error::other("not asked here"))
        }
        fn refused<T>(&self, what: &'static str) -> Result<T, String> {
            self.0.borrow_mut().push(what);
            Err("not asked here".into())
        }
    }

    impl kkt_containerized_service::ConfigReader for Untouched {
        fn settings(
            &self,
            _: &str,
            _: &Path,
            _: Option<&Path>,
        ) -> Result<kkt_containerized_service::ContainerConfig, String> {
            self.refused("container_configs.settings")
        }
    }

    impl kkt_llm_agent_config::Host for Untouched {
        fn read(&self, _: &Path) -> std::io::Result<String> {
            self.asked("clients.read")
        }
        fn exists(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("clients.exists");
            false
        }
        fn create_private(&self, _: &Path, _: &str) -> std::io::Result<()> {
            self.asked("clients.create_private")
        }
        fn restrict(&self, _: &Path) -> std::io::Result<()> {
            self.asked("clients.restrict")
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            self.asked("clients.create_dir_all")
        }
        fn rename(&self, _: &Path, _: &Path) -> std::io::Result<()> {
            self.asked("clients.rename")
        }
        fn remove_file(&self, _: &Path) -> std::io::Result<()> {
            self.asked("clients.remove_file")
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            self.asked("clients.absolute")
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            self.asked("clients.canonicalize")
        }
        fn process_id(&self) -> u32 {
            self.0.borrow_mut().push("clients.process_id");
            0
        }
    }
    impl kkt_llm_agent_config::ClientCommand for Untouched {
        fn remove_server(
            &self,
            _: Option<&Path>,
        ) -> Result<(), kkt_llm_agent_config::CommandError> {
            self.refused("client_command.remove_server")
                .map_err(not_run)
        }
        fn add_server(
            &self,
            _: &str,
            _: Option<&Path>,
        ) -> Result<(), kkt_llm_agent_config::CommandError> {
            self.refused("client_command.add_server").map_err(not_run)
        }
    }

    // A `Prober` must be `Sync`, so it keeps no log: it fails the test.
    struct NoProbes;
    impl kkt_diagnostics::Prober for NoProbes {
        fn key_file(&self, _: &Path) -> Result<kkt_diagnostics::KeyFile, String> {
            panic!("uninstall read a key file to probe")
        }
        fn answer<'a>(
            &'a self,
            _: &'a str,
        ) -> kkt_diagnostics::Probing<'a, kkt_diagnostics::Answered> {
            panic!("uninstall probed a server")
        }
        fn initialize<'a>(
            &'a self,
            _: &'a str,
            _: Option<&'a str>,
        ) -> kkt_diagnostics::Probing<'a, kkt_diagnostics::Initialized> {
            panic!("uninstall probed a server")
        }
    }

    impl crate::ImportLanes for Untouched {
        fn check(&self, _: &Path, _: setup::Purpose) -> Result<usize, crate::PartError> {
            self.refused("import_lanes.check")
                .map_err(crate::PartError::from)
        }
        fn import(&self, _: &setup::Record, _: &Path) -> Result<String, crate::PartError> {
            self.refused("import_lanes.import")
                .map_err(crate::PartError::from)
        }
    }

    impl ConnectionSource for Untouched {
        type KeySource = ();
        fn recorded(
            &self,
        ) -> Result<
            Option<kkt_deployment::connection::RecordedServer>,
            kkt_deployment::connection::SourceError,
        > {
            self.refused("connection.record")
                .map_err(|said| kkt_deployment::connection::SourceError(said.into()))
        }
        fn serve(
            &self,
            _: kkt_deployment::connection::ConnectionOptions,
        ) -> Result<
            kkt_deployment::connection::ServeConnection<()>,
            kkt_deployment::connection::SourceError,
        > {
            self.refused("connection.serve")
                .map_err(|said| kkt_deployment::connection::SourceError(said.into()))
        }
        fn make_default_key(
            &self,
            _: &kkt_keys::AccessKey,
        ) -> Result<(), kkt_deployment::connection::SourceError> {
            self.refused("connection.make_default_key")
                .map_err(|said| kkt_deployment::connection::SourceError(said.into()))
        }
        fn check_key(
            &self,
            _: &Path,
            _: &(),
        ) -> Result<(), kkt_deployment::connection::SourceError> {
            self.refused("connection.check_key")
                .map_err(|said| kkt_deployment::connection::SourceError(said.into()))
        }
        fn absolute(&self, _: &Path) -> Result<PathBuf, kkt_deployment::connection::SourceError> {
            self.refused("connection.absolute")
                .map_err(|said| kkt_deployment::connection::SourceError(said.into()))
        }
    }

    impl ServiceDefaults for Untouched {
        fn matches_defaults(&self, _: &[String]) -> Result<bool, crate::PartError> {
            self.refused("service_defaults.matches_defaults")
                .map_err(crate::PartError::from)
        }
    }

    impl kkt_keys::KeyFiles for Untouched {
        fn read(&self, _: &Path) -> std::io::Result<String> {
            self.asked("key_files.read")
        }
        fn read_optional(&self, _: &Path) -> std::io::Result<Option<String>> {
            self.asked("key_files.read_optional")
        }
        fn try_exists(&self, _: &Path) -> std::io::Result<bool> {
            self.asked("key_files.try_exists")
        }
        fn is_file(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("key_files.is_file");
            false
        }
        fn mode(&self, _: &Path) -> std::io::Result<Option<u32>> {
            self.asked("key_files.mode")
        }
        fn create_dir_all(&self, _: &Path) -> Result<(), kkt_keys::KeyFileError> {
            self.refused("key_files.create_dir_all")
                .map_err(not_asked_key)
        }
        fn keep_staged(&self, _: &Path, _: &Path) -> Result<(), kkt_keys::KeyFileError> {
            self.refused("key_files.keep_staged").map_err(not_asked_key)
        }
        fn remove(&self, _: &Path) -> std::io::Result<()> {
            self.asked("key_files.remove")
        }
        fn new_key(
            &self,
            _: &Path,
            _: bool,
            _: &mut dyn FnMut() -> String,
        ) -> Result<String, kkt_keys::KeyFileError> {
            self.refused("key_files.new_key").map_err(not_asked_key)
        }
        fn make_default_key(
            &self,
            _: &kkt_keys::AccessKey,
            _: &mut dyn FnMut() -> String,
        ) -> Result<Option<PathBuf>, kkt_keys::KeyFileError> {
            self.refused("key_files.make_default_key")
                .map_err(not_asked_key)
        }
        fn write_key(&self, _: &Path, _: &str, _: bool) -> Result<(), kkt_keys::KeyFileError> {
            self.refused("key_files.write_key").map_err(not_asked_key)
        }
        fn write_admin_name(&self, _: &Path, _: Option<&str>) -> std::io::Result<()> {
            self.asked("key_files.write_admin_name")
        }
    }

    impl kkt_containerized_service::LoginLink for Untouched {
        fn show(&self, _: &str) {
            self.0.borrow_mut().push("login_link.show");
        }
        fn open_requested(&self) -> bool {
            self.0.borrow_mut().push("login_link.open_requested");
            false
        }
    }

    impl kkt_setup::Lines for Untouched {
        fn interactive(&self) -> bool {
            self.0.borrow_mut().push("lines.interactive");
            false
        }
        fn say(&self, _: &str) {
            self.0.borrow_mut().push("lines.say");
        }
        fn read_line(&self, _: &str) -> Result<Option<String>, kkt_setup::LineError> {
            self.refused("lines.read_line")
                .map_err(|said| kkt_setup::LineError(said.into()))
        }
    }

    impl crate::ConfigText for Untouched {
        fn read(&self, _: &Path) -> Result<Option<String>, crate::PartError> {
            self.refused("config_file.read")
                .map_err(crate::PartError::from)
        }
        fn write(&self, _: &Path, _: &str) -> Result<(), crate::PartError> {
            self.refused("config_file.write")
                .map_err(crate::PartError::from)
        }
        fn with_allowed_host(&self, _: &str, _: &str) -> Result<String, crate::PartError> {
            self.refused("config_file.with_allowed_host")
                .map_err(crate::PartError::from)
        }
    }

    impl crate::Databases for Untouched {
        fn exists(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("database.exists");
            false
        }
    }

    /// A key file error for a step a test did not expect.
    fn not_asked_key(said: String) -> kkt_keys::KeyFileError {
        kkt_keys::KeyFileError::file(
            kkt_keys::KeyFileAction::Write,
            "nowhere",
            std::io::Error::other(said),
        )
    }

    /// A client command error for a call a test did not expect.
    fn not_run(said: String) -> kkt_llm_agent_config::CommandError {
        kkt_llm_agent_config::CommandError {
            kind: kkt_llm_agent_config::CommandFailure::NotRun,
            cause: said.into(),
        }
    }

    /// A backup error for an operation a test did not expect.
    fn not_asked(said: String) -> backup::BackupError {
        backup::BackupError::file(
            backup::FileAction::Write,
            "nowhere",
            std::io::Error::other(said),
        )
    }

    impl kkt_backup::Backups for Untouched {
        fn write(&self, _: &Path, _: &backup::Layout<'_>) -> Result<PathBuf, backup::BackupError> {
            self.refused("backups.write").map_err(not_asked)
        }
        fn prune(&self, _: &Path, _: usize) -> Result<Vec<PathBuf>, backup::BackupError> {
            self.refused("backups.prune").map_err(not_asked)
        }
    }

    /// A clock `uninstall` must not read.
    struct NoClock;
    impl kkt_clock::Clock for NoClock {
        fn now(&self) -> kkt_clock::Time {
            panic!("uninstall read the clock")
        }
        fn sleep_until(&self, _: kkt_clock::Time) -> futures::future::BoxFuture<'_, ()> {
            panic!("uninstall waited on the clock")
        }
    }

    /// The setup record of a service not shared over Tailscale, which logs
    /// what is done to it.
    struct UnsharedRecord(RefCell<Vec<String>>);

    impl crate::SetupFiles for UnsharedRecord {
        fn read_record(&self, _: &Path) -> Result<Option<setup::Record>, crate::PartError> {
            self.0.borrow_mut().push("read the record".into());
            Ok(Some(setup::Record {
                mode: setup::Mode::Service { tailscale: false },
                written_by: "0.10.0".into(),
                url: "http://127.0.0.1:7420/mcp".into(),
                key_file: Some(PathBuf::from("/home/k/.config/kkt/access-key")),
                clients: None,
            }))
        }
        fn read_unfinished(&self, _: &Path) -> Result<Option<setup::Unfinished>, crate::PartError> {
            self.0.borrow_mut().push("read the unfinished note".into());
            Err("not asked here".into())
        }
        fn write_record(&self, _: &Path, _: &setup::Record) -> Result<(), crate::PartError> {
            self.0.borrow_mut().push("wrote a record".into());
            Err("not asked here".into())
        }
        fn write_unfinished(
            &self,
            _: &Path,
            _: &setup::Unfinished,
        ) -> Result<(), crate::PartError> {
            self.0.borrow_mut().push("wrote an unfinished note".into());
            Err("not asked here".into())
        }
        fn remove_unfinished(&self, _: &Path) -> Result<(), crate::PartError> {
            self.0
                .borrow_mut()
                .push("removed the unfinished note".into());
            Err("not asked here".into())
        }
        fn kkt_path(&self) -> Result<PathBuf, crate::PartError> {
            self.0.borrow_mut().push("asked kkt's path".into());
            Err("not asked here".into())
        }
        fn path_dirs(&self) -> Vec<PathBuf> {
            self.0.borrow_mut().push("asked PATH".into());
            Vec::new()
        }
        fn on_path(&self, _: &[PathBuf], _: &str) -> bool {
            self.0.borrow_mut().push("asked what is on PATH".into());
            false
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("not asked here"))
        }
        fn program_path(&self) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("not asked here"))
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("not asked here"))
        }
        fn remove_record(&self, dir: &Path) -> Result<(), crate::PartError> {
            self.0
                .borrow_mut()
                .push(format!("removed the record in {}", dir.display()));
            Ok(())
        }
    }

    /// What the command says.
    #[derive(Default)]
    struct Said(RefCell<String>);
    impl crate::Output for Said {
        fn output(&self, text: std::fmt::Arguments<'_>) {
            self.0.borrow_mut().push_str(&text.to_string());
        }
        fn errors(&self, text: std::fmt::Arguments<'_>) {
            self.0.borrow_mut().push_str(&text.to_string());
        }
    }

    #[test]
    fn uninstalling_a_recorded_service_not_shared_over_tailscale_asks_no_serving_question() {
        let home = PathBuf::from("/home/k");
        let env = Env {
            home: Some(home.clone()),
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let (serve, remote, untouched) =
            (Asked::default(), Tailscale::default(), Untouched::default());
        let service = kkt_bare_service::testing::Memory::installed(vec!["serve".into()], true);
        let record = UnsharedRecord(RefCell::new(Vec::new()));
        let said = Said::default();
        let try_bind = |_: std::net::SocketAddr| -> std::io::Result<()> {
            panic!("uninstall tried an address")
        };
        let container = kkt_containerized_service::testing::Memory::new(None, String::new());
        let parts = InstallParts {
            container: &container,
            container_configs: &untouched,
            service: &service,
            try_bind: &try_bind,
            remote: &remote,
            clients: &untouched,
            client_command: &untouched,
            prober: &NoProbes,
            import_lanes: &untouched,
            connection: &untouched,
            clock: std::sync::Arc::new(NoClock),
            serve: &serve,
            service_defaults: &untouched,
            setup_files: &record,
            key_files: &untouched,
            entropy: &kkt_entropy::testing::SeqEntropy::new(0),
            database: &untouched,
            backups: &untouched,
            output: &said,
            mcp_path: "/mcp",
            login_link: &untouched,
            lines: &untouched,
            config_file: &untouched,
        };
        futures::executor::block_on(uninstall(&parts, &env, Platform::Xdg))
            .expect("the service and its record are removed");
        assert!(serve.0.borrow().is_empty(), "asked {:?}", serve.0.borrow());
        assert!(remote.0.borrow().is_empty(), "ran {:?}", remote.0.borrow());
        assert!(container.calls().is_empty(), "{:?}", container.calls());
        assert!(
            untouched.0.borrow().is_empty(),
            "{:?}",
            untouched.0.borrow()
        );
        assert_eq!(service.done(), ["uninstall"]);
        let config_dir = home.join(".config").join("kkt");
        assert_eq!(
            *record.0.borrow(),
            [
                "read the record".to_string(),
                format!("removed the record in {}", config_dir.display()),
            ]
        );
        assert!(
            said.0.borrow().ends_with(&uninstall_line(None)),
            "{}",
            said.0.borrow()
        );
    }

    /// A server in Docker that setup installed but did not record, found
    /// through the container's `existing`: shared over Tailscale, with the
    /// access key's copy setup saves. Its address is the node's, and its
    /// mode says so; without the key's copy it is refused, naming it.
    #[test]
    fn an_unrecorded_install_in_docker_is_found_through_the_container() {
        let home = PathBuf::from("/home/k");
        let env = Env {
            home: Some(home.clone()),
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let folder = kkt_deployment::docker_folder(&env, Platform::Xdg).expect("a Docker folder");
        let key_file = home
            .join(".config")
            .join("kkt")
            .join(crate::record::DOCKER_KEY_FILE_NAME);
        let container = kkt_containerized_service::testing::Memory::new(
            Some(("kkt-1".into(), "tail1234.ts.net".into())),
            String::new(),
        );
        let untouched = Untouched::default();
        let shared = kkt_containerized_service::Inputs {
            version: "0.4.0".into(),
            project: "kkt".into(),
            port: 7420,
            allow_hosts: vec![],
            sharing: Some(kkt_containerized_service::Sharing {
                hostname: "kkt-1".into(),
                auth_key: None,
            }),
        };
        let copies = kkt_containerized_service::KeyCopies {
            access: key_file.clone(),
            web: home.join("web"),
            admin: home.join("admin"),
        };
        futures::executor::block_on(kkt_containerized_service::ServiceContainer::provision(
            &container,
            &NoClock,
            &untouched,
            &kkt_container::Folder(folder.clone()),
            &shared,
            &copies,
        ))
        .expect("installed");
        let (serve, remote) = (Asked::default(), Tailscale::default());
        let service = kkt_bare_service::testing::Memory::new();
        let record = UnsharedRecord(RefCell::new(Vec::new()));
        let said = Said::default();
        let try_bind = |_: std::net::SocketAddr| -> std::io::Result<()> {
            panic!("finding an install tried an address")
        };
        let with_copy = kkt_keys::testing::Memory::new().with_file(key_file.clone(), "key\n");
        let read = |container: &dyn kkt_containerized_service::ServiceContainer,
                    key_files: &dyn kkt_keys::KeyFiles| {
            let parts = InstallParts {
                container,
                container_configs: &untouched,
                service: &service,
                try_bind: &try_bind,
                remote: &remote,
                clients: &untouched,
                client_command: &untouched,
                prober: &NoProbes,
                import_lanes: &untouched,
                connection: &untouched,
                clock: std::sync::Arc::new(NoClock),
                serve: &serve,
                service_defaults: &untouched,
                setup_files: &record,
                key_files,
                entropy: &kkt_entropy::testing::SeqEntropy::new(0),
                database: &untouched,
                backups: &untouched,
                output: &said,
                mcp_path: "/mcp",
                login_link: &untouched,
                lines: &untouched,
                config_file: &untouched,
            };
            read_unrecorded(&parts, &env, Platform::Xdg)
        };
        let found = read(&container, &with_copy)
            .expect("found")
            .expect("an install in Docker");
        assert_eq!(found.url, "https://kkt-1.tail1234.ts.net/mcp");
        assert_eq!(found.key_file, Some(key_file.clone()));
        assert_eq!(
            found.mode,
            setup::Mode::Docker(setup::DockerSetup {
                folder: folder.clone(),
                tailscale: true,
                hostname: Some("kkt-1".into()),
                tailnet: Some("tail1234.ts.net".into()),
            })
        );
        let (purpose, refused) =
            read(&container, &untouched).expect_err("refused without the key's copy");
        assert_eq!(purpose, setup::Purpose::Docker);
        assert_eq!(
            refused.to_string(),
            format!(
                "Found a kkt server in Docker from {}, not recorded, but the copy of its access key setup saves, {}, is missing.",
                folder.display(),
                key_file.display()
            )
        );

        // An install that is not shared: no node, and the address is the
        // port it was given on this machine.
        let local = kkt_containerized_service::testing::Memory::new(
            Some(("kkt-1".into(), "tail1234.ts.net".into())),
            String::new(),
        );
        let unshared = kkt_containerized_service::Inputs {
            port: 7431,
            sharing: None,
            ..shared.clone()
        };
        futures::executor::block_on(kkt_containerized_service::ServiceContainer::provision(
            &local,
            &NoClock,
            &untouched,
            &kkt_container::Folder(folder.clone()),
            &unshared,
            &copies,
        ))
        .expect("installed");
        let found = read(&local, &with_copy)
            .expect("found")
            .expect("an install in Docker");
        assert_eq!(found.url, "http://127.0.0.1:7431/mcp");
        assert_eq!(found.key_file, Some(key_file.clone()));
        assert_eq!(
            found.mode,
            setup::Mode::Docker(setup::DockerSetup {
                folder: folder.clone(),
                tailscale: false,
                hostname: None,
                tailnet: None,
            })
        );
    }

    /// A container whose installs cannot be looked at.
    struct Unseen;

    impl kkt_containerized_service::ServiceContainer for Unseen {
        fn provision<'a>(
            &'a self,
            _: &'a dyn kkt_clock::Clock,
            _: &'a dyn kkt_containerized_service::LoginLink,
            _: &'a kkt_container::Folder,
            _: &'a kkt_containerized_service::Inputs,
            _: &'a kkt_containerized_service::KeyCopies,
        ) -> futures::future::LocalBoxFuture<
            'a,
            Result<
                kkt_containerized_service::Provisioned,
                kkt_containerized_service::ContainerError,
            >,
        > {
            unreachable!("nothing is installed")
        }
        fn add_admin_key<'a>(
            &'a self,
            _: &'a dyn kkt_clock::Clock,
            _: &'a kkt_container::Folder,
            _: &'a kkt_containerized_service::KeyCopies,
            _: &'a str,
        ) -> futures::future::LocalBoxFuture<
            'a,
            Result<PathBuf, kkt_containerized_service::ContainerError>,
        > {
            unreachable!("nothing is installed")
        }
        fn status(
            &self,
            _: &kkt_container::Folder,
            _: &dyn kkt_containerized_service::ConfigReader,
            _: &str,
            _: &Path,
        ) -> Result<kkt_containerized_service::State, kkt_containerized_service::ContainerError>
        {
            unreachable!("no status is asked")
        }
        fn existing(
            &self,
            _: &kkt_container::Folder,
        ) -> Result<
            Option<kkt_containerized_service::Existing>,
            kkt_containerized_service::ContainerError,
        > {
            Err(kkt_containerized_service::ContainerError::Refused {
                what: "the project".into(),
            })
        }
        fn compare_web_key(
            &self,
            _: &kkt_container::Folder,
            _: &Path,
        ) -> Result<kkt_containerized_service::KeyCopy, kkt_containerized_service::ContainerError>
        {
            unreachable!("no key is compared")
        }
        fn up(
            &self,
            _: &kkt_container::Folder,
        ) -> Result<(), kkt_containerized_service::ContainerError> {
            unreachable!("nothing is started")
        }
        fn down(
            &self,
            _: &kkt_container::Folder,
            _: bool,
        ) -> Result<(), kkt_containerized_service::ContainerError> {
            unreachable!("nothing is stopped")
        }
        fn backup(
            &self,
            _: &kkt_container::Folder,
            _: Option<&Path>,
        ) -> Result<PathBuf, kkt_containerized_service::ContainerError> {
            unreachable!("nothing is backed up")
        }
        fn upgrade<'a>(
            &'a self,
            _: &'a dyn kkt_clock::Clock,
            _: &'a kkt_container::Folder,
            _: &'a str,
            _: Option<&'a kkt_containerized_service::KeyCopies>,
        ) -> futures::future::LocalBoxFuture<
            'a,
            Result<(), kkt_containerized_service::ContainerError>,
        > {
            unreachable!("nothing is upgraded")
        }
        fn run_in_server(
            &self,
            _: &kkt_container::Folder,
            _: &kkt_containerized_service::InServer<'_>,
        ) -> Result<String, kkt_containerized_service::ContainerError> {
            unreachable!("nothing is run")
        }
        fn config_home_file(
            &self,
            _: &kkt_container::Folder,
            _: &Path,
        ) -> Result<String, kkt_containerized_service::ContainerError> {
            unreachable!("nothing is read")
        }
        fn has_settings(&self, _: &kkt_container::Folder) -> bool {
            unreachable!("nothing is asked")
        }
        fn settings_file(&self, _: &kkt_container::Folder) -> bool {
            unreachable!("nothing is asked")
        }
        fn version(
            &self,
            _: &kkt_container::Folder,
        ) -> Result<String, kkt_containerized_service::ContainerError> {
            unreachable!("nothing is asked")
        }
        fn sharing_key(
            &self,
            _: &Path,
        ) -> Result<String, kkt_containerized_service::ContainerError> {
            unreachable!("nothing is asked")
        }
    }

    /// An installed service that fails at the step given.
    struct Failing(&'static str);

    impl Failing {
        fn at(&self, step: &str) -> Result<(), kkt_bare_service::ServiceError> {
            if self.0 == step {
                return Err(kkt_bare_service::ServiceError::Failed {
                    command: format!("service {step}"),
                    detail: "refused".into(),
                });
            }
            Ok(())
        }
    }

    impl kkt_bare_service::Service for Failing {
        fn ready(&self) -> Result<(), kkt_bare_service::ServiceError> {
            self.at("ready")
        }
        fn install<'a>(
            &'a self,
            _: &'a dyn kkt_clock::Clock,
            _: kkt_bare_service::Settings,
            _: &'a kkt_bare_service::Spec,
        ) -> kkt_bare_service::Pending<'a, Vec<String>> {
            unreachable!("nothing is installed")
        }
        fn dry_run<'a>(
            &'a self,
            _: &'a dyn kkt_clock::Clock,
            _: kkt_bare_service::Settings,
            _: &'a kkt_bare_service::Spec,
        ) -> kkt_bare_service::Pending<'a, Vec<String>> {
            unreachable!("nothing is installed")
        }
        fn check_started<'a>(
            &'a self,
            _: &'a dyn kkt_clock::Clock,
            _: kkt_bare_service::Settings,
        ) -> kkt_bare_service::Pending<'a, ()> {
            unreachable!("nothing is started")
        }
        fn installed_args(&self) -> Result<Option<Vec<String>>, kkt_bare_service::ServiceError> {
            self.at("args")?;
            if self.0 == "gone" {
                return Ok(None);
            }
            Ok(Some(vec!["serve".into()]))
        }
        fn uninstall(&self) -> Result<Vec<String>, kkt_bare_service::ServiceError> {
            unreachable!("nothing is uninstalled")
        }
        fn start(&self) -> Result<Vec<String>, kkt_bare_service::ServiceError> {
            unreachable!("nothing is started")
        }
        fn stop(&self) -> Result<Vec<String>, kkt_bare_service::ServiceError> {
            unreachable!("nothing is stopped")
        }
        fn add_key(
            &self,
            _: &Path,
            _: &dyn Fn(&Path) -> Result<(), kkt_bare_service::KeyError>,
        ) -> Result<bool, kkt_bare_service::ServiceError> {
            unreachable!("no key is added")
        }
        fn state(&self) -> Result<kkt_bare_service::ServiceState, kkt_bare_service::ServiceError> {
            self.at("state")?;
            Ok(kkt_bare_service::ServiceState {
                installed: true,
                running: true,
            })
        }
        fn status(&self) -> Result<Vec<String>, kkt_bare_service::ServiceError> {
            unreachable!("no status is asked")
        }
        fn logs(&self, _: usize, _: bool) -> Result<Vec<String>, kkt_bare_service::ServiceError> {
            unreachable!("no logs are asked")
        }
    }

    /// Settings that run the defaults or not, or that cannot say.
    struct Defaults(Option<bool>);
    impl ServiceDefaults for Defaults {
        fn matches_defaults(&self, _: &[String]) -> Result<bool, crate::PartError> {
            self.0.ok_or_else(|| "the config file is unreadable".into())
        }
    }

    /// A server reached at a fixed address with no key, or none at all.
    struct Reached(bool);
    impl ConnectionSource for Reached {
        type KeySource = ();
        fn recorded(
            &self,
        ) -> Result<
            Option<kkt_deployment::connection::RecordedServer>,
            kkt_deployment::connection::SourceError,
        > {
            unreachable!("no record is read")
        }
        fn serve(
            &self,
            _: kkt_deployment::connection::ConnectionOptions,
        ) -> Result<
            kkt_deployment::connection::ServeConnection<()>,
            kkt_deployment::connection::SourceError,
        > {
            match self.0 {
                true => Ok(kkt_deployment::connection::ServeConnection {
                    url: "http://127.0.0.1:7420/mcp".into(),
                    access_key: kkt_keys::AccessKey::None,
                    key_source: (),
                }),
                false => Err(kkt_deployment::connection::SourceError(
                    "the serve settings are unreadable".into(),
                )),
            }
        }
        fn make_default_key(
            &self,
            _: &kkt_keys::AccessKey,
        ) -> Result<(), kkt_deployment::connection::SourceError> {
            unreachable!("no key is made")
        }
        fn check_key(
            &self,
            _: &Path,
            _: &(),
        ) -> Result<(), kkt_deployment::connection::SourceError> {
            unreachable!("no key is checked")
        }
        fn absolute(&self, _: &Path) -> Result<PathBuf, kkt_deployment::connection::SourceError> {
            unreachable!("no path is made absolute")
        }
    }

    /// Serving facts under the home `/home/k` whose address cannot be read.
    struct NoBind;
    impl ServeFacts for NoBind {
        fn config(&self, _: Option<PathBuf>) -> Result<crate::Config, crate::PartError> {
            unreachable!("no config is read")
        }
        fn configured_access_key(&self) -> Result<Option<PathBuf>, crate::PartError> {
            unreachable!("no config is read")
        }
        fn database(
            &self,
            _: Option<PathBuf>,
            _: Option<PathBuf>,
        ) -> Result<PathBuf, crate::PartError> {
            unreachable!("no database is asked")
        }
        fn bind(&self) -> Result<std::net::SocketAddr, crate::PartError> {
            Err("bind is not an address".into())
        }
        fn default_access_key(&self) -> Option<PathBuf> {
            unreachable!("no key is made")
        }
        fn default_name_pack(&self) -> String {
            unreachable!("no name pack is asked")
        }
        fn config_dir(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/k/.config/kkt"))
        }
        fn config_file(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/k/.config/kkt/config.toml"))
        }
        fn config_file_in_home(&self) -> PathBuf {
            PathBuf::from("kkt/config.toml")
        }
        fn service_settings(
            &self,
            _: Option<PathBuf>,
            _: kkt_deployment::ServiceSettings,
        ) -> Result<crate::ServiceInputs<'_>, crate::PartError> {
            unreachable!("no service is planned")
        }
    }

    /// The reasons a server installed with no record cannot be made out,
    /// word for word, with how it runs.
    #[test]
    fn why_an_unrecorded_server_cannot_be_made_out_is_pinned() {
        let env = Env {
            home: Some(PathBuf::from("/home/k")),
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let untouched = Untouched::default();
        let empty = kkt_containerized_service::testing::Memory::new(None, String::new());
        let record = UnsharedRecord(RefCell::new(Vec::new()));
        let said = Said::default();
        let remote = Tailscale::default();
        let try_bind = |_: std::net::SocketAddr| -> std::io::Result<()> {
            panic!("finding an install tried an address")
        };
        let asked = Asked::default();
        let read = |container: &dyn kkt_containerized_service::ServiceContainer,
                    service: &dyn kkt_bare_service::Service,
                    defaults: &Defaults,
                    connection: &Reached,
                    serve: &dyn ServeFacts| {
            let parts = InstallParts {
                container,
                container_configs: &untouched,
                service,
                try_bind: &try_bind,
                remote: &remote,
                clients: &untouched,
                client_command: &untouched,
                prober: &NoProbes,
                import_lanes: &untouched,
                connection,
                clock: std::sync::Arc::new(NoClock),
                serve,
                service_defaults: defaults,
                setup_files: &record,
                key_files: &untouched,
                entropy: &kkt_entropy::testing::SeqEntropy::new(0),
                database: &untouched,
                backups: &untouched,
                output: &said,
                mcp_path: "/mcp",
                login_link: &untouched,
                lines: &untouched,
                config_file: &untouched,
            };
            read_unrecorded(&parts, &env, Platform::Xdg)
                .map(|_| ())
                .map_err(|(purpose, e)| (purpose, e.to_string()))
        };
        let folder = kkt_deployment::docker_folder(&env, Platform::Xdg).expect("a Docker folder");
        let none = kkt_bare_service::testing::Memory::new();
        let service = |why: &str| {
            (
                setup::Purpose::Service,
                format!("Found a kkt service, not recorded, but {why}."),
            )
        };
        let failed = |step: &str| format!("`service {step}` failed: refused");
        assert_eq!(
            read(
                &Unseen,
                &none,
                &Defaults(Some(true)),
                &Reached(true),
                &asked
            ),
            Err((
                setup::Purpose::Docker,
                format!(
                    "Found a kkt server in Docker from {}, not recorded, but the project.",
                    folder.display()
                )
            ))
        );
        assert_eq!(
            read(
                &empty,
                &Failing("ready"),
                &Defaults(Some(true)),
                &Reached(true),
                &asked
            ),
            Err((setup::Purpose::Service, failed("ready")))
        );
        assert_eq!(
            read(
                &empty,
                &Failing("state"),
                &Defaults(Some(true)),
                &Reached(true),
                &asked
            ),
            Err((setup::Purpose::Service, failed("state")))
        );
        assert_eq!(
            read(
                &empty,
                &Failing("args"),
                &Defaults(Some(true)),
                &Reached(true),
                &asked
            ),
            Err(service(&format!(
                "its service file could not be read: {}",
                failed("args")
            )))
        );
        assert_eq!(
            read(
                &empty,
                &Failing("gone"),
                &Defaults(Some(true)),
                &Reached(true),
                &asked
            ),
            Err(service("its service file went away while it was read"))
        );
        assert_eq!(
            read(
                &empty,
                &Failing(""),
                &Defaults(None),
                &Reached(true),
                &asked
            ),
            Err(service(
                "its settings could not be read: the config file is unreadable"
            ))
        );
        assert_eq!(
            read(
                &empty,
                &Failing(""),
                &Defaults(Some(true)),
                &Reached(false),
                &asked
            ),
            Err(service(
                "its settings could not be read: the serve settings are unreadable"
            ))
        );
        assert_eq!(
            read(
                &empty,
                &Failing(""),
                &Defaults(Some(true)),
                &Reached(true),
                &NoBind
            ),
            Err(service(
                "its settings could not be read: bind is not an address"
            ))
        );
        assert_eq!(
            read(
                &empty,
                &Failing(""),
                &Defaults(Some(true)),
                &Reached(true),
                &asked
            ),
            Err(service(
                "whether it is shared over Tailscale is not known: tailscale is not here"
            ))
        );
    }

    /// A host where nothing is shared over Tailscale.
    struct NotShared;
    impl kkt_remote_access::RemoteAccess for NotShared {
        fn prepare(&self) -> Result<kkt_remote_access::Node, kkt_remote_access::SharingError> {
            unreachable!("nothing is shared")
        }
        fn serve(&self, _: std::net::SocketAddr) -> Result<(), kkt_remote_access::SharingError> {
            unreachable!("nothing is shared")
        }
        fn served(
            &self,
            _: std::net::SocketAddr,
        ) -> Result<Option<kkt_remote_access::Node>, kkt_remote_access::SharingError> {
            Ok(None)
        }
        fn stop(&self, _: std::net::SocketAddr) -> Result<(), kkt_remote_access::SharingError> {
            unreachable!("nothing is shared")
        }
    }

    /// A service that does not run the default settings, and a server in
    /// Docker installed beside a service: kkt-setup's refusals, passed on
    /// word for word, with how the server runs.
    #[test]
    fn kkt_setups_refusals_of_an_unrecorded_server_are_passed_on() {
        let home = PathBuf::from("/home/k");
        let env = Env {
            home: Some(home.clone()),
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let folder = kkt_deployment::docker_folder(&env, Platform::Xdg).expect("a Docker folder");
        let key_file = home
            .join(".config/kkt")
            .join(crate::record::DOCKER_KEY_FILE_NAME);
        let untouched = Untouched::default();
        let installed = kkt_containerized_service::testing::Memory::new(None, String::new());
        let inputs = kkt_containerized_service::Inputs {
            version: "0.4.0".into(),
            project: "kkt".into(),
            port: 7420,
            allow_hosts: vec![],
            sharing: None,
        };
        let copies = kkt_containerized_service::KeyCopies {
            access: key_file.clone(),
            web: home.join("web"),
            admin: home.join("admin"),
        };
        futures::executor::block_on(kkt_containerized_service::ServiceContainer::provision(
            &installed,
            &NoClock,
            &untouched,
            &kkt_container::Folder(folder.clone()),
            &inputs,
            &copies,
        ))
        .expect("installed");
        let empty = kkt_containerized_service::testing::Memory::new(None, String::new());
        let key_files = kkt_keys::testing::Memory::new().with_file(key_file, "key\n");
        let record = UnsharedRecord(RefCell::new(Vec::new()));
        let said = Said::default();
        let try_bind = |_: std::net::SocketAddr| -> std::io::Result<()> {
            panic!("finding an install tried an address")
        };
        let serve = Asked::default();
        let read = |container: &dyn kkt_containerized_service::ServiceContainer,
                    defaults: &Defaults| {
            let parts = InstallParts {
                container,
                container_configs: &untouched,
                service: &Failing(""),
                try_bind: &try_bind,
                remote: &NotShared,
                clients: &untouched,
                client_command: &untouched,
                prober: &NoProbes,
                import_lanes: &untouched,
                connection: &Reached(true),
                clock: std::sync::Arc::new(NoClock),
                serve: &serve,
                service_defaults: defaults,
                setup_files: &record,
                key_files: &key_files,
                entropy: &kkt_entropy::testing::SeqEntropy::new(0),
                database: &untouched,
                backups: &untouched,
                output: &said,
                mcp_path: "/mcp",
                login_link: &untouched,
                lines: &untouched,
                config_file: &untouched,
            };
            read_unrecorded(&parts, &env, Platform::Xdg)
                .map(|_| ())
                .map_err(|(purpose, e)| (purpose, e.to_string()))
        };
        // kkt-setup's own sentence, which its tests pin.
        let not_defaults = setup::service_runs_defaults(&["serve".to_string()], false)
            .expect_err("refused")
            .to_string();
        assert_eq!(
            read(&empty, &Defaults(Some(false))),
            Err((setup::Purpose::Service, not_defaults))
        );
        assert_eq!(
            read(&installed, &Defaults(Some(true))),
            Err((
                setup::Purpose::Docker,
                format!(
                    "Found a kkt server in Docker from {}, for clients at \
                     http://127.0.0.1:7420/mcp, not recorded. A kkt service is installed here \
                     too, so which one clients use is not known; remove one (`kkt service \
                     uninstall`, or `docker compose down` in the Docker folder) and run `kkt \
                     setup` again",
                    folder.display()
                )
            ))
        );
    }

    /// Setup's files, where only the path questions are asked: the step
    /// named fails, the others answer the path as given.
    struct PathsFail(&'static str);

    impl PathsFail {
        fn at(&self, step: &str, path: &Path) -> std::io::Result<PathBuf> {
            match self.0 == step {
                true => Err(std::io::Error::other("refused")),
                false => Ok(path.to_path_buf()),
            }
        }
    }

    impl crate::SetupFiles for PathsFail {
        fn read_record(&self, _: &Path) -> Result<Option<setup::Record>, crate::PartError> {
            unreachable!("no record is read")
        }
        fn read_unfinished(&self, _: &Path) -> Result<Option<setup::Unfinished>, crate::PartError> {
            unreachable!("no note is read")
        }
        fn write_record(&self, _: &Path, _: &setup::Record) -> Result<(), crate::PartError> {
            unreachable!("no record is written")
        }
        fn write_unfinished(
            &self,
            _: &Path,
            _: &setup::Unfinished,
        ) -> Result<(), crate::PartError> {
            unreachable!("no note is written")
        }
        fn remove_unfinished(&self, _: &Path) -> Result<(), crate::PartError> {
            unreachable!("no note is removed")
        }
        fn kkt_path(&self) -> Result<PathBuf, crate::PartError> {
            unreachable!("kkt is not looked for")
        }
        fn path_dirs(&self) -> Vec<PathBuf> {
            unreachable!("PATH is not read")
        }
        fn on_path(&self, _: &[PathBuf], _: &str) -> bool {
            unreachable!("PATH is not read")
        }
        fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
            self.at("absolute", path)
        }
        fn program_path(&self) -> std::io::Result<PathBuf> {
            self.at("program", Path::new("/bin/kkt"))
        }
        fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
            self.at("canonicalize", path)
        }
        fn remove_record(&self, _: &Path) -> Result<(), crate::PartError> {
            unreachable!("no record is removed")
        }
    }

    /// The refusals planning a service words itself, word for word.
    #[test]
    fn the_service_plans_refusals_are_pinned() {
        let untouched = Untouched::default();
        let empty = kkt_containerized_service::testing::Memory::new(None, String::new());
        let service = kkt_bare_service::testing::Memory::new();
        let said = Said::default();
        let remote = Tailscale::default();
        let serve = Asked::default();
        let try_bind = |_: std::net::SocketAddr| -> std::io::Result<()> {
            panic!("planning a service tried an address")
        };
        let plan = |files: &PathsFail, install: crate::service::ServiceInstall| {
            let parts = InstallParts {
                container: &empty,
                container_configs: &untouched,
                service: &service,
                try_bind: &try_bind,
                remote: &remote,
                clients: &untouched,
                client_command: &untouched,
                prober: &NoProbes,
                import_lanes: &untouched,
                connection: &untouched,
                clock: std::sync::Arc::new(NoClock),
                serve: &serve,
                service_defaults: &untouched,
                setup_files: files,
                key_files: &untouched,
                entropy: &kkt_entropy::testing::SeqEntropy::new(0),
                database: &untouched,
                backups: &untouched,
                output: &said,
                mcp_path: "/mcp",
                login_link: &untouched,
                lines: &untouched,
                config_file: &untouched,
            };
            crate::service::service_plan(&parts, install)
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        let install = |config: Option<&str>, db: Option<&str>| crate::service::ServiceInstall {
            config: config.map(PathBuf::from),
            db: db.map(PathBuf::from),
            ..crate::service::ServiceInstall::NONE
        };
        assert_eq!(
            plan(&PathsFail("program"), install(None, None)),
            Err("could not find this program's path: refused".to_string())
        );
        assert_eq!(
            plan(&PathsFail("canonicalize"), install(Some("kkt.toml"), None)),
            Err("invalid setting: config could not be read from kkt.toml: refused".to_string())
        );
        assert_eq!(
            plan(&PathsFail("absolute"), install(None, Some("kkt.db"))),
            Err("could not make kkt.db absolute: refused".to_string())
        );
        // The serving facts' own refusal, passed on as it is.
        assert_eq!(
            plan(&PathsFail(""), install(None, None)),
            Err("not asked here".to_string())
        );
    }
}
