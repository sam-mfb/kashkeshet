//! An install's lifecycle: `kkt status`, `kkt upgrade`, `kkt uninstall`
//! and `kkt backup`, and the backup `kkt serve` makes before it upgrades
//! the database.

use crate::commands::lanes::existing_db;
use crate::commands::service::{self, ServiceAction, service_plan};
use crate::machine::keys::{
    KeyKind, admin_name_line, local_operator_key, local_web_key, new_key_file,
};
use crate::machine::record::{
    read_setup, read_unfinished, read_unrecorded, recorded_setup, unfinished_mode,
    write_setup_record,
};
use crate::machine::settings::{ServeArgs, configured_name_pack, read_config, serve_bind};
use kkt_backup as backup;
use kkt_clock::{Clock, SystemClock};
use kkt_config as config;
use kkt_container as docker;
use kkt_provenance::Build;
use kkt_remote_access as tailscale;
use kkt_service::{Manager, OsSystem};
use kkt_settings::{self as settings, Env};
use kkt_setup as setup;
use kkt_store as store;
use std::path::{Path, PathBuf};

/// Writes a backup of `contents` into `folder`, copying the database with
/// SQLite's online backup.
fn write_backup(
    folder: &Path,
    clock: &dyn Clock,
    contents: &backup::Contents,
) -> Result<PathBuf, String> {
    let at = clock.now();
    backup::write(
        folder,
        at,
        &kkt_clock::to_text(at),
        Build::CURRENT.version,
        contents,
        |from, to| store::copy_database(from, to).map_err(|e| e.to_string()),
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
    let Some(upgrade) = store::pending_upgrade(database)
        .map_err(|e| format!("could not read {}: {e}", database.display()))?
    else {
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
    match read_setup(env)?.map(|record| record.mode) {
        Some(setup::Mode::Docker(install)) => {
            if db.is_some() || config.is_some() {
                return Err("Docker backup uses the configured container; --db and --config apply only to a local service".into());
            }
            let written = docker::backup(&docker::OsHost, &docker::Folder(install.folder), folder)
                .map_err(|e| e.to_string())?;
            println!("Backed up to {}", written.display());
            Ok(())
        }
        Some(setup::Mode::Client) => {
            Err("this machine is a client; back up the server on the machine where it runs".into())
        }
        Some(setup::Mode::Service { .. }) | None => {
            let folder = folder.ok_or("a local backup needs FOLDER; run `kkt backup <folder>`")?;
            local_backup(env, folder, db, config)
        }
    }
}

pub fn status(env: &Env, serve_args: ServeArgs) -> Result<(), String> {
    let (unfinished, unfinished_error) = match read_unfinished(env) {
        Ok(unfinished) => (unfinished, None),
        Err(e) => (None, Some(e)),
    };
    let recorded = read_setup(env)?;
    let (unrecorded, unrecorded_error) = match (&recorded, &unfinished, &unfinished_error) {
        (None, None, None) => match read_unrecorded(env, serve_args) {
            Ok(found) => (found, None),
            Err((_, e)) => (None, Some(e)),
        },
        _ => (None, None),
    };
    if let Some(path) = local_web_key(env, unfinished.as_ref(), unrecorded.as_ref())? {
        println!(
            "Web key: {} (read with `kkt key show --web`)",
            path.display()
        );
    }
    if let Some(path) = local_operator_key(
        env,
        unfinished.as_ref(),
        unrecorded.as_ref(),
        KeyKind::Admin,
    )?
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
    if let Some(e) = unfinished_error {
        println!("Unfinished setup: unknown ({e}). Run `kkt setup` again.");
    }
    if let Some(unfinished) = &unfinished {
        println!(
            "Unfinished setup: {} Run `kkt setup` again to finish it.",
            unfinished.summary()
        );
    }
    if let Some(e) = &unrecorded_error {
        println!("Unrecorded install: {e}");
    }
    if let Some(found) = &unrecorded {
        println!(
            "Unrecorded install: {} Run `kkt setup` to record it, so `kkt upgrade` manages it.",
            found.summary()
        );
    }
    let (mode, url) = match (recorded, unfinished, unrecorded) {
        (Some(record), _, _) => {
            if let Some(clients) = &record.clients {
                println!(
                    "Clients use: {} (switched from this machine's server; \
                     `kkt switch --local` points them back)",
                    clients.url
                );
            }
            (record.mode, record.url)
        }
        (None, Some(unfinished), _) => match unfinished.record.clone() {
            Some(record) => (record.mode, record.url),
            None => (
                unfinished_mode(env, &unfinished)?,
                "not known until the install finishes".to_string(),
            ),
        },
        (None, None, Some(found)) => (found.mode, found.url),
        (None, None, None) => {
            println!(
                "No setup record; checking this machine's kkt service. Run `kkt setup` to configure this machine."
            );
            print_configured_name_pack(env)?;
            return service::run(env, ServiceAction::Status);
        }
    };
    println!("Mode: {}\nServer: {url}", mode.name());
    match mode {
        setup::Mode::Service { .. } => {
            print_configured_name_pack(env)?;
            service::run(env, ServiceAction::Status)
        }
        setup::Mode::Docker(install) => {
            let folder = docker::Folder(install.folder);
            let state = docker::status(&docker::OsHost, &folder).map_err(|e| e.to_string())?;
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
                Some(docker::Login::LoggedIn) => println!("Tailscale login: logged in"),
                Some(docker::Login::NeedsLogin) => println!(
                    "Tailscale login: needed. The machine's key expired, or it was logged out, so it is off the tailnet. Run `kkt setup` to log in again."
                ),
                Some(docker::Login::Unreachable) => {
                    println!("Tailscale login: unknown; the Tailscale container does not answer")
                }
                Some(docker::Login::Other(state)) => println!("Tailscale login: {state}"),
                None => {}
            }
            Ok(())
        }
        setup::Mode::Client => {
            println!("Client only; the server runs on another machine.");
            Ok(())
        }
    }
}

/// Status reads local configuration, not a running server's command line.
fn print_configured_name_pack(env: &Env) -> Result<(), String> {
    let (_, options) = read_config(env, None)?;
    println!(
        "Name pack (configured; serve flags may override): {}",
        configured_name_pack(&options)
    );
    Ok(())
}

pub async fn upgrade(
    env: &Env,
    serve_args: ServeArgs,
    allow_downgrade: bool,
) -> Result<(), String> {
    let mut record = recorded_setup(env, serve_args)?;
    let current = match &record.mode {
        setup::Mode::Client => {
            return Err(
                "this machine is a client; upgrade the server on the machine where it runs".into(),
            );
        }
        setup::Mode::Docker(install) => {
            let path = install.folder.join(docker::ENV_FILE);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            docker::version_in_env(&text)
                .ok_or_else(|| format!("{} has no KKT_VERSION", path.display()))?
        }
        setup::Mode::Service { .. } => record.written_by.clone(),
    };
    if setup::upgrade(&current, Build::CURRENT.version, allow_downgrade)? == setup::Upgrade::Already
    {
        println!("Server is already at kkt {}", Build::CURRENT.version);
        return Ok(());
    }
    match &record.mode {
        setup::Mode::Docker(install) => {
            docker::upgrade(
                &docker::OsHost,
                &SystemClock,
                &docker::Folder(install.folder.clone()),
                Build::CURRENT.version,
                record.key_file.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { .. } => {
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
                .install(&OsSystem, &plan.spec)
                .map_err(|e| e.to_string())?
            {
                println!("{line}");
            }
        }
        setup::Mode::Client => {
            return Err(
                "this machine is a client; upgrade the server on the machine where it runs".into(),
            );
        }
    }
    record.written_by = Build::CURRENT.version.to_string();
    write_setup_record(env, &record)?;
    println!("Upgraded the server to kkt {}", Build::CURRENT.version);
    Ok(())
}

pub fn uninstall(env: &Env, serve_args: ServeArgs) -> Result<(), String> {
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
            docker::down(&docker::OsHost, &docker::Folder(install.folder), false)
                .map_err(|e| e.to_string())?;
        }
        setup::Mode::Service { tailscale: shared } => {
            if shared {
                // The default config file's address, as `serve` would bind.
                tailscale::stop(&tailscale::OsHost, serve_bind(env)?)?;
            }
            service::run(env, ServiceAction::Uninstall)?;
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
    let access_key = match (&options.access_key_file, options.no_access_key) {
        (Some(path), _) => Some(path.clone()),
        (None, Some(true)) => None,
        (None, _) => env.access_key_file().filter(|path| path.exists()),
    };
    let web_key = match access_key.as_deref().map(settings::web_key_path) {
        Some(path) => match path.try_exists() {
            Ok(true) => Some(path),
            // Installations from before separate web keys have none until
            // their first start or service upgrade.
            Ok(false) => None,
            Err(e) => return Err(format!("could not inspect {}: {e}", path.display())),
        },
        None => None,
    };
    // Only servers whose operator asked for one have an admin key.
    let admin_key = match access_key.as_deref().and_then(settings::admin_key_path) {
        Some(path) => match path.try_exists() {
            Ok(true) => Some(path),
            Ok(false) => None,
            Err(e) => return Err(format!("could not inspect {}: {e}", path.display())),
        },
        None => None,
    };
    // And only an admin key made with a name has one.
    let admin_name = match admin_key.as_deref().map(settings::admin_name_path) {
        Some(path) => match path.try_exists() {
            Ok(true) => Some(path),
            Ok(false) => None,
            Err(e) => return Err(format!("could not inspect {}: {e}", path.display())),
        },
        None => None,
    };
    let contents = backup::Contents {
        database: &database,
        config: config_file.as_deref(),
        access_key: access_key.as_deref(),
        web_key: web_key.as_deref(),
        admin_key: admin_key.as_deref(),
        admin_name: admin_name.as_deref(),
    };
    let written = write_backup(folder, &SystemClock, &contents)?;
    if let Some(path) = &web_key {
        println!("  web key {}", path.display());
    }
    if let Some(path) = &admin_key {
        println!("  admin key {}", path.display());
    }
    if let Some(path) = &admin_name {
        println!("  admin name {}", path.display());
    }
    println!("Backed up to {}", written.display());
    println!("  database {}", database.display());
    if let Some(path) = &config_file {
        println!("  config file {}", path.display());
    }
    match &access_key {
        Some(path) => println!("  access key {}", path.display()),
        None => println!("  no access key"),
    }
    Ok(())
}
