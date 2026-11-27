//! The operator's service commands and the ordered plan/check/install sequence.
use crate::keys::new_key_file;
use kkt_bare_service::Spec;
use kkt_deployment::check_bind;
use kkt_deployment::connection::ConnectionSource;
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

/// Why `kkt service` failed: each kind is a source's own error.
#[derive(Debug, thiserror::Error)]
pub enum ServiceCommandError {
    #[error(transparent)]
    Service(#[from] kkt_bare_service::ServiceError),
    #[error(transparent)]
    Plan(#[from] ServicePlanError),
    #[error(transparent)]
    Bind(#[from] kkt_deployment::BindError),
    #[error(transparent)]
    Key(#[from] kkt_keys::KeyFileError),
}

/// Why the service to install could not be planned.
#[derive(Debug, thiserror::Error)]
pub enum ServicePlanError {
    #[error("could not find this program's path: {0}")]
    ProgramPath(#[source] std::io::Error),
    #[error("invalid setting: config could not be read from {}: {source}", path.display())]
    ConfigPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not make {} absolute: {source}", path.display())]
    Absolute {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The program's serving settings.
    #[error(transparent)]
    Settings(crate::PartError),
    #[error(transparent)]
    Plan(kkt_deployment::PlanError),
    /// The program's check of the whole serving surface.
    #[error(transparent)]
    Check(crate::PartError),
    /// What the service runs.
    #[error(transparent)]
    Spec(kkt_deployment::PlanError),
}

/// Which access key `kkt key show --web` will go by once the command is
/// done, which decides whether a new web key's hint must name its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyLookup {
    /// The setup record as it is now (`kkt service install` alone leaves
    /// it as it is).
    Recorded,
    /// The served access key: the configured or default one, which `kkt
    /// setup` records for the service it is installing, whatever the record
    /// said before.
    Served,
}

/// `kkt service ...`: runs the command against this system's service
/// manager and prints what happened. Installing may wait on the system
/// clock while launchd lets the new service load.
pub async fn run<D>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    command: ServiceAction,
) -> Result<(), ServiceCommandError> {
    run_for(parts, command, KeyLookup::Recorded).await
}

/// [`run`], with the key lookup that will be in effect once the caller is
/// done.
pub(crate) async fn run_for<D>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    command: ServiceAction,
    lookup: KeyLookup,
) -> Result<(), ServiceCommandError> {
    let service = parts.service;
    service.ready()?;
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
            let running = service.state()?.running;
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
                        kkt_bare_service::Settings::DEFAULT,
                        &plan.spec,
                    )
                    .await?;
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
                    new_key_file(parts.key_files, parts.entropy, key, false)?;
                    parts.output.output(format_args!(
                        "Created a new web key at {}. Read it with `{}`.\n",
                        key.display(),
                        web_key_show_command(parts, key, lookup),
                    ));
                }
                if let Some(key) = &plan.new_key {
                    new_key_file(parts.key_files, parts.entropy, key, false)?;
                    parts.output.output(format_args!(
                        "Created a new access key at {}\n",
                        key.display()
                    ));
                }
                match service
                    .install(
                        parts.clock.as_ref(),
                        kkt_bare_service::Settings::DEFAULT,
                        &plan.spec,
                    )
                    .await
                {
                    // The service manager took the request; whether the
                    // server stays up is checked before saying it started.
                    Ok(said) => service
                        .check_started(parts.clock.as_ref(), kkt_bare_service::Settings::DEFAULT)
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
    }?;
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
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    install: ServiceInstall,
) -> Result<ServicePlan, ServicePlanError> {
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
        .map_err(ServicePlanError::ProgramPath)?;
    let config = match config {
        Some(path) => Some(
            parts
                .setup_files
                .canonicalize(&path)
                .map_err(|source| ServicePlanError::ConfigPath { path, source })?,
        ),
        None => None,
    };
    let absolute = |path: PathBuf| {
        parts
            .setup_files
            .absolute(&path)
            .map_err(|source| ServicePlanError::Absolute { path, source })
    };
    let flags = kkt_deployment::ServiceSettings {
        db: db.map(absolute).transpose()?,
        access_key_file: access_key_file.map(absolute).transpose()?,
        no_access_key: no_access_key.then_some(true),
    };
    let inputs = parts
        .serve
        .service_settings(config, flags)
        .map_err(ServicePlanError::Settings)?;
    let plan = kkt_deployment::plan(inputs.request, &|path: &Path| {
        parts.key_files.try_exists(path)
    })
    .map_err(ServicePlanError::Plan)?;
    let checked = inputs.check.check(&plan).map_err(ServicePlanError::Check)?;
    Ok(ServicePlan {
        spec: plan.spec(program).map_err(ServicePlanError::Spec)?,
        new_key: plan.new_key,
        new_web_key: plan.new_web_key,
        bind: checked.bind,
        config_file: checked.config_file,
    })
}

/// The `kkt key show` command that reads a newly created service web key.
/// Uses the bare `--web` form when `kkt key show --web` will resolve to
/// `key` once the command is done; otherwise names the file explicitly.
/// That is the setup record's web key when `lookup` is
/// [`KeyLookup::Recorded`] and a record exists, and the configured or
/// default access key's web key otherwise.
pub(crate) fn web_key_show_command<D>(
    parts: &crate::InstallParts<'_, impl ConnectionSource, D>,
    key: &Path,
    lookup: KeyLookup,
) -> String {
    let served = || {
        parts
            .serve
            .configured_access_key()
            .ok()
            .flatten()
            .or_else(|| parts.serve.default_access_key())
            .map(|access| kkt_keys::web_key_path(&access))
    };
    let resolved = match lookup {
        KeyLookup::Served => served(),
        KeyLookup::Recorded => match crate::record::read_setup(parts.serve, parts.setup_files) {
            Ok(Some(record)) => kkt_deployment::recorded_key(
                &crate::deployment::server_keys(&record),
                kkt_deployment::KeyKind::Web,
            )
            .ok()
            .flatten(),
            Ok(None) => served(),
            Err(_) => None,
        },
    };
    if resolved.as_deref() == Some(key) {
        "kkt key show --web".to_string()
    } else {
        format!(
            "kkt key show --web --key-file {}",
            kkt_diagnostics::shell_word(&key.display().to_string())
        )
    }
}
