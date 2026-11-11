//! `kkt service`: installs, removes, starts, stops and reports on the kkt
//! service through this system's service manager, and plans what an
//! installed service runs.

use crate::machine::keys::new_key_file;
use crate::machine::service::ServiceSystem;
use crate::machine::settings::{
    DefaultDb, NO_HOME_TO_KEEP_DB, Report, check_settings, load_serve_options, resolve,
};
use crate::process::program_path;
use kkt_clock_system::SystemClock;
use kkt_service::{DryRun, Manager, PlanError, Request, Spec};
use kkt_settings::{Env, Platform, ServeOptions};
use std::io;
use std::net::{SocketAddr, TcpListener};
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

/// `kkt service ...`: runs the command against this system's service
/// manager and prints what happened. Installing may wait on the system
/// clock while launchd lets the new service load.
pub async fn run(env: &Env, command: ServiceAction) -> Result<(), String> {
    let manager = Manager::for_this_os(env).map_err(|e| e.to_string())?;
    let system = ServiceSystem;
    let said = match command {
        ServiceAction::Install(ServiceInstall {
            config,
            db,
            access_key_file,
            no_access_key,
            dry_run,
        }) => {
            let plan = service_plan(env, config, db, access_key_file, no_access_key)?;
            let running = manager.state(&system).map_err(|e| e.to_string())?.running;
            check_bind(&plan, running, &try_bind)?;
            if dry_run {
                let dry = DryRun::new(&system);
                manager
                    .install(&dry, &SystemClock, &plan.spec)
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
                said.extend(dry.actions());
                Ok(said)
            } else {
                // Said at once, so they are seen even if the start fails.
                if let Some(key) = &plan.new_web_key {
                    new_key_file(key, false)?;
                    println!(
                        "Created a new web key at {}. Read it with `kkt key show --web`.",
                        key.display()
                    );
                }
                if let Some(key) = &plan.new_key {
                    new_key_file(key, false)?;
                    println!("Created a new access key at {}", key.display());
                }
                match manager.install(&system, &SystemClock, &plan.spec).await {
                    // The service manager took the request; whether the
                    // server stays up is checked before saying it started.
                    Ok(said) => manager
                        .check_started(&system, &SystemClock)
                        .await
                        .map(|()| said),
                    Err(e) => Err(e),
                }
            }
        }
        ServiceAction::Uninstall => manager.uninstall(&system),
        ServiceAction::Start => manager.start(&system),
        ServiceAction::Stop => manager.stop(&system),
        ServiceAction::Status => manager.status(&system),
        ServiceAction::Logs { lines, follow } => manager.logs(&system, lines, follow),
    }
    .map_err(|e| e.to_string())?;
    for line in said {
        println!("{line}");
    }
    Ok(())
}

/// What `kkt service install` does: the service to install, and the access
/// key to create first, if any.
pub struct ServicePlan {
    pub spec: Spec,
    new_key: Option<PathBuf>,
    pub new_web_key: Option<PathBuf>,
    /// The address the server will listen on.
    bind: SocketAddr,
    /// The config file that sets `bind`: the one the settings were read
    /// from, else the default one, if there is a home to keep it in.
    config_file: Option<String>,
}

/// Refuses to install a service whose server could not listen on its
/// address, such as one another program has, before anything is written:
/// installed, it would fail and be started again every few seconds (#371).
/// `try_bind` listens on an address and lets it go at once. When the kkt
/// service is `running`, it holds its own address, and installing again
/// replaces it, so the address is not tried; `check_started` catches a new
/// address that turns out to be taken.
fn check_bind(
    plan: &ServicePlan,
    running: bool,
    try_bind: &dyn Fn(SocketAddr) -> io::Result<()>,
) -> Result<(), String> {
    if running {
        return Ok(());
    }
    let Err(e) = try_bind(plan.bind) else {
        return Ok(());
    };
    let bind = plan.bind;
    let set_it = match &plan.config_file {
        Some(file) => format!("set another address with `bind` in {file}"),
        None => "set another address with `bind` in a config file".to_string(),
    };
    Err(if e.kind() == io::ErrorKind::AddrInUse {
        format!(
            "{bind} is already in use, so the server could not listen there. Stop the program \
             using it, or {set_it}, then run `kkt service install` again."
        )
    } else {
        format!(
            "the server could not listen on {bind}: {e}. To fix it, {set_it}, then run \
             `kkt service install` again."
        )
    })
}

/// Listens on `address` and lets it go at once: [`check_bind`]'s test of
/// whether the server could listen there.
fn try_bind(address: SocketAddr) -> io::Result<()> {
    TcpListener::bind(address).map(drop)
}

/// What the service runs, as [`kkt_service::plan`] works it out. This reads
/// the config file, finds this program's path and which key files exist,
/// and checks the settings now, so a bad one fails here rather than in a
/// loop of restarts.
pub fn service_plan(
    env: &Env,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    access_key_file: Option<PathBuf>,
    no_access_key: bool,
) -> Result<ServicePlan, String> {
    let program = program_path().map_err(|e| format!("could not find this program's path: {e}"))?;
    let config = match config {
        Some(path) => Some(path.canonicalize().map_err(|e| {
            format!(
                "invalid setting: config could not be read from {}: {e}",
                path.display()
            )
        })?),
        None => None,
    };
    let absolute = |path: PathBuf| {
        std::path::absolute(&path)
            .map_err(|e| format!("could not make {} absolute: {e}", path.display()))
    };
    let flags = ServeOptions {
        db: db.map(absolute).transpose()?,
        access_key_file: access_key_file.map(absolute).transpose()?,
        no_access_key: no_access_key.then_some(true),
        ..ServeOptions::NONE
    };
    let loaded = load_serve_options(env, config, flags)?;
    let sources = loaded.sources();
    let said = |e: PlanError| match e {
        PlanError::Invalid(e) => sources.invalid(e),
        other => other.to_string(),
    };
    let plan = kkt_service::plan(
        Request {
            config_file: loaded.config_file,
            flags: loaded.flags,
            from_file: loaded.from_file,
            default_db: env.default_db(Platform::CURRENT),
            default_key: env.access_key_file(),
        },
        &|path: &Path| path.exists(),
    )
    .map_err(said)?;
    // The database is the one passed or the config file's, so it is always
    // known here: a missing home directory was refused above.
    let in_effect = DefaultDb::Required(NO_HOME_TO_KEEP_DB);
    let mut settings = resolve(env, plan.check.clone(), in_effect, Report::Traced(&sources))?;
    settings.web_key = plan.check_web_key.clone();
    let checked = check_settings(settings, &sources)?;
    Ok(ServicePlan {
        spec: plan.spec(program).map_err(said)?,
        new_key: plan.new_key,
        new_web_key: plan.new_web_key,
        bind: checked.bind,
        config_file: checked
            .config_file
            .or_else(|| env.config_file().map(|p| p.display().to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{test_env, write_default_config};
    use crate::scratch;
    use kkt_settings::Platform;

    fn args(plan: &ServicePlan) -> Vec<&str> {
        plan.spec.args.iter().map(String::as_str).collect()
    }

    #[test]
    fn without_a_home_the_database_must_be_given() {
        assert_eq!(
            service_plan(&test_env(None), None, None, None, false).map(|p| p.spec.args),
            Err("no home directory to keep the database in: set HOME or pass --db".to_string())
        );
    }

    #[test]
    fn without_a_home_the_key_must_be_given_or_turned_off() {
        let dir = scratch::Dir::new("service-no-home-key");
        let db = dir.join("kkt.db");
        assert_eq!(
            service_plan(&test_env(None), None, Some(db.clone()), None, false).map(|p| p.spec.args),
            Err(
                "no home directory to keep the access key in: set HOME or pass --access-key-file"
                    .to_string()
            )
        );
        let plan =
            service_plan(&test_env(None), None, Some(db.clone()), None, true).expect("valid");
        let db_text = db.to_str().expect("utf-8");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--db",
                db_text,
                "--no-access-key"
            ]
        );
        assert_eq!(plan.spec.dirs, [dir.to_path_buf()]);
        assert_eq!((plan.new_key, plan.new_web_key), (None, None));
    }

    #[test]
    fn with_nothing_given_the_defaults_are_passed_and_the_keys_made() {
        let home = scratch::Dir::new("service-defaults");
        let env = test_env(Some(&home));
        let plan = service_plan(&env, None, None, None, false).expect("valid");
        let db = env.default_db(Platform::CURRENT).expect("db");
        let key = env.access_key_file().expect("key");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--db",
                db.to_str().expect("utf-8"),
                "--access-key-file",
                key.to_str().expect("utf-8"),
            ]
        );
        assert_eq!(plan.spec.dirs, [db.parent().expect("folder").to_path_buf()]);
        assert_eq!(plan.new_key, Some(key.clone()));
        assert_eq!(plan.new_web_key, Some(kkt_settings::web_key_path(&key)));
    }

    #[test]
    fn what_the_config_file_gives_is_not_passed() {
        let home = scratch::Dir::new("service-config-gives");
        let env = test_env(Some(&home));
        let db = home.join("data").join("file.db");
        let config = write_default_config(
            &home,
            &format!("db = \"{}\"\nno_access_key = true", db.display()),
        );
        let plan = service_plan(&env, None, None, None, false).expect("valid");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--config",
                config.to_str().expect("utf-8")
            ]
        );
        assert_eq!(plan.spec.dirs, [home.join("data")]);
        assert_eq!((plan.new_key, plan.new_web_key), (None, None));
    }

    #[test]
    fn a_bad_setting_names_the_config_file_that_set_it() {
        let home = scratch::Dir::new("service-bad-setting");
        let env = test_env(Some(&home));
        let config = write_default_config(&home, "name_pack = \"missing\"");
        let err = service_plan(&env, None, None, None, false)
            .map(|p| p.spec.args)
            .expect_err("bad pack");
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(
            err.ends_with(&format!(" (set in {})", config.display())),
            "{err}"
        );
    }

    /// A plan for `bind`, as set in `config_file`.
    fn plan_for(bind: SocketAddr, config_file: Option<&str>) -> ServicePlan {
        ServicePlan {
            spec: Spec {
                program: PathBuf::from("/bin/kkt"),
                args: Vec::new(),
                dirs: Vec::new(),
            },
            new_key: None,
            new_web_key: None,
            bind,
            config_file: config_file.map(String::from),
        }
    }

    /// #371: with another program listening on the address, the install
    /// is refused, naming the address and where to change it, rather than
    /// installing a server that fails and is started again every few
    /// seconds.
    #[test]
    fn an_address_in_use_is_refused_before_installing() {
        let other = TcpListener::bind("127.0.0.1:0").expect("listen");
        let bind = other.local_addr().expect("address");
        let plan = plan_for(bind, Some("/home/sam/.config/kkt/config.toml"));
        assert_eq!(
            check_bind(&plan, false, &try_bind),
            Err(format!(
                "{bind} is already in use, so the server could not listen there. Stop the \
                 program using it, or set another address with `bind` in \
                 /home/sam/.config/kkt/config.toml, then run `kkt service install` again."
            ))
        );
    }

    /// A free address passes, and is free again afterwards for the server.
    #[test]
    fn a_free_address_passes() {
        let free = TcpListener::bind("127.0.0.1:0").expect("listen");
        let bind = free.local_addr().expect("address");
        drop(free);
        assert_eq!(check_bind(&plan_for(bind, None), false, &try_bind), Ok(()));
        TcpListener::bind(bind).expect("still free");
    }

    /// The running kkt service holds its own address, and installing again
    /// replaces it, so the address is not tried.
    #[test]
    fn a_running_service_is_not_refused_for_its_own_address() {
        let ours = TcpListener::bind("127.0.0.1:0").expect("listen");
        let bind = ours.local_addr().expect("address");
        let tried = std::cell::Cell::new(false);
        let try_bind = |_: SocketAddr| {
            tried.set(true);
            Ok(())
        };
        assert_eq!(check_bind(&plan_for(bind, None), true, &try_bind), Ok(()));
        assert!(!tried.get());
    }

    /// Other reasons the server could not listen are refused too, saying
    /// what the system said.
    #[test]
    fn other_listen_failures_are_refused_with_the_reason() {
        let bind: SocketAddr = "192.0.2.1:7420".parse().expect("address");
        let unavailable = |_: SocketAddr| Err(io::Error::from(io::ErrorKind::AddrNotAvailable));
        let err = check_bind(&plan_for(bind, None), false, &unavailable).expect_err("refused");
        assert!(
            err.starts_with("the server could not listen on 192.0.2.1:7420: "),
            "{err}"
        );
        assert!(
            err.ends_with(
                "To fix it, set another address with `bind` in a config file, then run \
                 `kkt service install` again."
            ),
            "{err}"
        );
    }

    #[test]
    fn the_plan_names_the_default_config_file_for_bind() {
        let home = scratch::Dir::new("service-bind-config");
        let env = test_env(Some(&home));
        let plan = service_plan(&env, None, None, None, false).expect("valid");
        assert_eq!(plan.bind, kkt_config::DEFAULT_BIND);
        assert_eq!(
            plan.config_file,
            env.config_file().map(|p| p.display().to_string())
        );
    }
}
