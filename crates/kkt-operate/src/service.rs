//! The operator's service commands and the ordered plan/check/install sequence.
use crate::keys::new_key_file;
use kkt_service::{Spec, check_bind};
use kkt_setup::connection::ConnectionSource;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// What `kkt service` does.
pub enum ServiceAction {
    Install(ServiceInstall),
    Uninstall,
    Start,
    Stop,
    Status,
    Logs { lines: usize, follow: bool },
}

/// How `kkt service install` installs the service.
pub struct ServiceInstall {
    pub config: Option<PathBuf>,
    pub db: Option<PathBuf>,
    pub access_key_file: Option<PathBuf>,
    pub no_access_key: bool,
    pub dry_run: bool,
}

impl ServiceInstall {
    /// Install from the default config, with no command-line overrides.
    pub const NONE: Self = Self {
        config: None,
        db: None,
        access_key_file: None,
        no_access_key: false,
        dry_run: false,
    };
}

/// `kkt service ...`: runs the command against this system's service
/// manager and prints what happened. Installing may wait on the system
/// clock while launchd lets the new service load.
pub async fn run<D>(
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    command: ServiceAction,
) -> Result<(), String> {
    let service = parts.service;
    service.ready().map_err(|e| e.to_string())?;
    let said = match command {
        ServiceAction::Install(ServiceInstall {
            config,
            db,
            access_key_file,
            no_access_key,
            dry_run,
        }) => {
            let plan = service_plan(
                parts,
                ServiceInstall {
                    config,
                    db,
                    access_key_file,
                    no_access_key,
                    dry_run,
                },
            )?;
            let running = service.state().map_err(|e| e.to_string())?.running;
            check_bind(
                running,
                plan.bind,
                plan.config_file.as_deref(),
                parts.try_bind,
            )?;
            if dry_run {
                let actions = service
                    .dry_run(
                        parts.clock.as_ref(),
                        kkt_service::Settings::DEFAULT,
                        &plan.spec,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                let mut said = vec!["Dry run: nothing was changed.".to_string()];
                if let Some(key) = &plan.new_web_key {
                    said.push(format!("Would create a new web key at {}", key.display()));
                }
                if let Some(key) = &plan.new_key {
                    said.push(format!(
                        "Would create a new access key at {}",
                        key.display()
                    ));
                }
                said.extend(actions);
                Ok(said)
            } else {
                // Said at once, so they are seen even if the start fails.
                if let Some(key) = &plan.new_web_key {
                    new_key_file(parts.key_files, key, false)?;
                    parts.output.output(format_args!(
                        "Created a new web key at {}. Read it with `kkt key show --web`.\n",
                        key.display()
                    ));
                }
                if let Some(key) = &plan.new_key {
                    new_key_file(parts.key_files, key, false)?;
                    parts.output.output(format_args!(
                        "Created a new access key at {}\n",
                        key.display()
                    ));
                }
                match service
                    .install(
                        parts.clock.as_ref(),
                        kkt_service::Settings::DEFAULT,
                        &plan.spec,
                    )
                    .await
                {
                    // The service manager took the request; whether the
                    // server stays up is checked before saying it started.
                    Ok(said) => service
                        .check_started(parts.clock.as_ref(), kkt_service::Settings::DEFAULT)
                        .await
                        .map(|()| said),
                    Err(e) => Err(e),
                }
            }
        }
        ServiceAction::Uninstall => service.uninstall(),
        ServiceAction::Start => service.start(),
        ServiceAction::Stop => service.stop(),
        ServiceAction::Status => service.status(),
        ServiceAction::Logs { lines, follow } => service.logs(lines, follow),
    }
    .map_err(|e| e.to_string())?;
    for line in said {
        parts.output.output(format_args!("{line}\n"));
    }
    Ok(())
}

/// What `kkt service install` does: the service to install, and the access
/// key to create first, if any.
pub struct ServicePlan {
    pub spec: Spec,
    pub new_key: Option<PathBuf>,
    pub new_web_key: Option<PathBuf>,
    /// The address the server will listen on.
    pub bind: SocketAddr,
    /// The config file that sets `bind`: the one the settings were read
    /// from, else the default one, if there is a home to keep it in.
    pub config_file: Option<String>,
}

/// Resolve paths, read serving inputs lazily, plan the service, then ask the
/// program to check its whole serving surface with the plan's database and keys.
pub fn service_plan<D>(
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    install: ServiceInstall,
) -> Result<ServicePlan, String> {
    let ServiceInstall {
        config,
        db,
        access_key_file,
        no_access_key,
        ..
    } = install;
    let program = parts
        .setup_files
        .program_path()
        .map_err(|e| format!("could not find this program's path: {e}"))?;
    let config = match config {
        Some(path) => Some(parts.setup_files.canonicalize(&path).map_err(|e| {
            format!(
                "invalid setting: config could not be read from {}: {e}",
                path.display()
            )
        })?),
        None => None,
    };
    let absolute = |path: PathBuf| {
        parts
            .setup_files
            .absolute(&path)
            .map_err(|e| format!("could not make {} absolute: {e}", path.display()))
    };
    let flags = kkt_service::ServiceSettings {
        db: db.map(absolute).transpose()?,
        access_key_file: access_key_file.map(absolute).transpose()?,
        no_access_key: no_access_key.then_some(true),
    };
    let inputs = parts.serve.service_settings(config, flags)?;
    let plan = kkt_service::plan(inputs.request, &|path: &Path| parts.key_files.exists(path))
        .map_err(|e| e.to_string())?;
    let checked = inputs.check.check(&plan)?;
    Ok(ServicePlan {
        spec: plan.spec(program).map_err(|e| e.to_string())?,
        new_key: plan.new_key,
        new_web_key: plan.new_web_key,
        bind: checked.bind,
        config_file: checked.config_file,
    })
}
