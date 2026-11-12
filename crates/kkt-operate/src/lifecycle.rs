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
use kkt_service::ServiceError;
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
            service::run(parts, ServiceAction::Status).await
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
            parts.service.ready().map_err(|e| e.to_string())?;
            for line in parts
                .service
                .install(
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
            service::run(parts, ServiceAction::Uninstall).await?;
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
    remote: &dyn kkt_remote_access::RemoteAccess,
) -> Result<(), String> {
    if shared {
        remote.stop(serve.bind()?)?;
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

    /// A sharing provider that logs each operation and fails it.
    #[derive(Default)]
    struct Tailscale(RefCell<Vec<Vec<String>>>);

    impl kkt_remote_access::RemoteAccess for Tailscale {
        fn prepare(&self) -> Result<kkt_remote_access::Node, String> {
            self.0.borrow_mut().push(vec!["prepare".into()]);
            Err("tailscale is not here".into())
        }
        fn serve(&self, _: std::net::SocketAddr) -> Result<(), String> {
            self.0.borrow_mut().push(vec!["serve".into()]);
            Err("tailscale is not here".into())
        }
        fn served(
            &self,
            _: std::net::SocketAddr,
        ) -> Result<Option<kkt_remote_access::Node>, String> {
            self.0.borrow_mut().push(vec!["served".into()]);
            Err("tailscale is not here".into())
        }
        fn stop(&self, _: std::net::SocketAddr) -> Result<(), String> {
            self.0.borrow_mut().push(vec!["stop".into()]);
            Err("tailscale is not here".into())
        }
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

    impl kkt_container::Host for Untouched {
        fn run(&self, _: &kkt_container::Command) -> std::io::Result<kkt_container::Finished> {
            self.asked("container.run")
        }
        fn run_attached(
            &self,
            _: &kkt_container::Command,
        ) -> std::io::Result<kkt_container::Finished> {
            self.asked("container.run_attached")
        }
        fn exists(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("container.exists");
            false
        }
        fn is_file(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("container.is_file");
            false
        }
        fn read_file(&self, _: &Path) -> std::io::Result<String> {
            self.asked("container.read_file")
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            self.asked("container.create_dir_all")
        }
        fn write_file(&self, _: &Path, _: &str, _: bool) -> std::io::Result<()> {
            self.asked("container.write_file")
        }
        fn say(&self, _: &str) {
            self.0.borrow_mut().push("container.say");
        }
        fn working_dir(&self) -> std::io::Result<PathBuf> {
            self.asked("container.working_dir")
        }
    }

    impl kkt_container::ConfigReader for Untouched {
        fn settings(
            &self,
            _: &str,
            _: &Path,
            _: Option<&Path>,
        ) -> Result<kkt_container::ContainerConfig, String> {
            self.refused("container_configs.settings")
        }
    }

    impl kkt_client_config::Host for Untouched {
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
        fn claude(&self, _: &[&str], _: Option<&Path>) -> Result<(), String> {
            self.refused("clients.claude")
        }
    }

    // A `Prober` must be `Sync`, so it keeps no log: it fails the test.
    struct NoProbes;
    impl kkt_diagnostics::Prober for NoProbes {
        fn key_file(&self, _: &Path) -> Result<kkt_diagnostics::KeyFile, String> {
            panic!("uninstall read a key file to probe")
        }
        fn probe<'a>(&'a self, _: &'a str, _: Option<&'a str>) -> kkt_diagnostics::Probing<'a> {
            panic!("uninstall probed a server")
        }
    }

    impl kkt_lane_files::Disk for Untouched {
        fn file_names(&self, _: &Path) -> std::io::Result<Vec<String>> {
            self.asked("lanes.file_names")
        }
        fn read(&self, _: &Path) -> std::io::Result<String> {
            self.asked("lanes.read")
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            self.asked("lanes.create_dir_all")
        }
        fn write(&self, _: &Path, _: &str) -> std::io::Result<()> {
            self.asked("lanes.write")
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            self.asked("lanes.absolute")
        }
        fn git_commit(&self, _: &Path) -> std::io::Result<Option<String>> {
            self.asked("lanes.git_commit")
        }
    }

    impl ConnectionSource for Untouched {
        type KeySource = ();
        fn record(&self) -> Result<Option<setup::Record>, String> {
            self.refused("connection.record")
        }
        fn serve(
            &self,
            _: kkt_setup::connection::ConnectionOptions,
        ) -> Result<kkt_setup::connection::ServeConnection<()>, String> {
            self.refused("connection.serve")
        }
        fn make_default_key(&self, _: &kkt_admission::AccessKey) -> Result<(), String> {
            self.refused("connection.make_default_key")
        }
        fn check_key(&self, _: &Path, _: &()) -> Result<(), String> {
            self.refused("connection.check_key")
        }
        fn absolute(&self, _: &Path) -> Result<PathBuf, String> {
            self.refused("connection.absolute")
        }
    }

    impl ServiceDefaults for Untouched {
        fn matches_defaults(&self, _: &[String]) -> Result<bool, String> {
            self.refused("service_defaults.matches_defaults")
        }
    }

    impl crate::KeyFiles for Untouched {
        fn read(&self, _: &Path) -> std::io::Result<String> {
            self.asked("key_files.read")
        }
        fn read_optional(&self, _: &Path) -> std::io::Result<Option<String>> {
            self.asked("key_files.read_optional")
        }
        fn exists(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("key_files.exists");
            false
        }
        fn try_exists(&self, _: &Path) -> std::io::Result<bool> {
            self.asked("key_files.try_exists")
        }
        fn is_file(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("key_files.is_file");
            false
        }
        fn create_dir_all(&self, _: &Path) -> Result<(), String> {
            self.refused("key_files.create_dir_all")
        }
        fn keep_staged(&self, _: &Path, _: &Path) -> Result<(), String> {
            self.refused("key_files.keep_staged")
        }
        fn remove(&self, _: &Path) -> std::io::Result<()> {
            self.asked("key_files.remove")
        }
        fn new_key(&self, _: &Path, _: bool) -> Result<String, String> {
            self.refused("key_files.new_key")
        }
        fn write_key(&self, _: &Path, _: &str, _: bool) -> Result<(), String> {
            self.refused("key_files.write_key")
        }
        fn write_admin_name(&self, _: &Path, _: Option<&str>) -> Result<(), String> {
            self.refused("key_files.write_admin_name")
        }
    }

    impl kkt_container::LoginLink for Untouched {
        fn show(&self, _: &str) {
            self.0.borrow_mut().push("login_link.show");
        }
        fn open_requested(&self) -> bool {
            self.0.borrow_mut().push("login_link.open_requested");
            false
        }
        fn open(&self, _: &str) -> std::io::Result<()> {
            self.asked("login_link.open")
        }
        fn open_failure(&self) -> Option<String> {
            self.0.borrow_mut().push("login_link.open_failure");
            None
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
        fn read_line(&self, _: &str) -> Result<Option<String>, String> {
            self.refused("lines.read_line")
        }
    }

    impl crate::ConfigText for Untouched {
        fn read(&self, _: &Path) -> Result<Option<String>, String> {
            self.refused("config_file.read")
        }
        fn write(&self, _: &Path, _: &str) -> Result<(), String> {
            self.refused("config_file.write")
        }
    }

    impl crate::Databases for Untouched {
        fn exists(&self, _: &Path) -> bool {
            self.0.borrow_mut().push("database.exists");
            false
        }
        fn open(&self, _: &Path) -> Result<kkt_rules::Db, String> {
            self.refused("database.open")
        }
    }

    impl crate::BackupWriter for Untouched {
        fn write(&self, _: &Path, _: &backup::Layout<'_>) -> Result<PathBuf, String> {
            self.refused("backups.write")
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
        fn read_record(&self, _: &Path) -> Result<Option<setup::Record>, String> {
            self.0.borrow_mut().push("read the record".into());
            Ok(Some(setup::Record {
                mode: setup::Mode::Service { tailscale: false },
                written_by: "0.10.0".into(),
                url: "http://127.0.0.1:7420/mcp".into(),
                key_file: Some(PathBuf::from("/home/k/.config/kkt/access-key")),
                clients: None,
            }))
        }
        fn read_unfinished(&self, _: &Path) -> Result<Option<setup::Unfinished>, String> {
            self.0.borrow_mut().push("read the unfinished note".into());
            Err("not asked here".into())
        }
        fn write_record(&self, _: &Path, _: &setup::Record) -> Result<(), String> {
            self.0.borrow_mut().push("wrote a record".into());
            Err("not asked here".into())
        }
        fn write_unfinished(&self, _: &Path, _: &setup::Unfinished) -> Result<(), String> {
            self.0.borrow_mut().push("wrote an unfinished note".into());
            Err("not asked here".into())
        }
        fn remove_unfinished(&self, _: &Path) -> Result<(), String> {
            self.0
                .borrow_mut()
                .push("removed the unfinished note".into());
            Err("not asked here".into())
        }
        fn kkt_path(&self) -> Result<PathBuf, String> {
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
        fn remove_record(&self, dir: &Path) -> Result<(), String> {
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
            claude_config_dir: None,
            codex_home: None,
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let (serve, remote, untouched) =
            (Asked::default(), Tailscale::default(), Untouched::default());
        let service = kkt_service::testing::Memory::installed(vec!["serve".into()], true);
        let record = UnsharedRecord(RefCell::new(Vec::new()));
        let said = Said::default();
        let try_bind = |_: std::net::SocketAddr| -> std::io::Result<()> {
            panic!("uninstall tried an address")
        };
        let parts = OperateParts {
            container: &untouched,
            container_configs: &untouched,
            service: &service,
            try_bind: &try_bind,
            remote: &remote,
            clients: &untouched,
            prober: &NoProbes,
            lanes: &untouched,
            connection: &untouched,
            clock: std::sync::Arc::new(NoClock),
            serve: &serve,
            service_defaults: &untouched,
            setup_files: &record,
            key_files: &untouched,
            database: &untouched,
            backups: &untouched,
            output: &said,
            login_link: &untouched,
            lines: &untouched,
            config_file: &untouched,
        };
        futures::executor::block_on(uninstall(&parts, &env, Platform::Xdg))
            .expect("the service and its record are removed");
        assert!(serve.0.borrow().is_empty(), "asked {:?}", serve.0.borrow());
        assert!(remote.0.borrow().is_empty(), "ran {:?}", remote.0.borrow());
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
            said.0.borrow().ends_with(&setup::uninstall_line(None)),
            "{}",
            said.0.borrow()
        );
    }
}
