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
use kkt_machine::{Env, Platform};
use kkt_service::{DryRun, Manager, PlanError, Request, Spec, check_bind};
use kkt_settings::ServeOptions;
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
pub async fn run(env: &Env, platform: Platform, command: ServiceAction) -> Result<(), String> {
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
            let plan = service_plan(env, platform, config, db, access_key_file, no_access_key)?;
            let running = manager.state(&system).map_err(|e| e.to_string())?.running;
            check_bind(running, plan.bind, plan.config_file.as_deref(), &try_bind)?;
            if dry_run {
                let dry = DryRun::new(&system);
                manager
                    .install(
                        &dry,
                        &SystemClock,
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
                match manager
                    .install(
                        &system,
                        &SystemClock,
                        kkt_service::Settings::DEFAULT,
                        &plan.spec,
                    )
                    .await
                {
                    // The service manager took the request; whether the
                    // server stays up is checked before saying it started.
                    Ok(said) => manager
                        .check_started(&system, &SystemClock, kkt_service::Settings::DEFAULT)
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

/// Listens on `address` and lets it go at once: the program's answer to
/// [`kkt_service::check_bind`], whether the server could listen there.
fn try_bind(address: SocketAddr) -> io::Result<()> {
    TcpListener::bind(address).map(drop)
}

/// What the service runs, as [`kkt_service::plan`] works it out. This reads
/// the config file, finds this program's path and which key files exist,
/// and checks the settings now, so a bad one fails here rather than in a
/// loop of restarts.
pub fn service_plan(
    env: &Env,
    platform: Platform,
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
            default_db: kkt_setup::default_db(env, platform),
            default_key: crate::machine::settings::default_access_key(env),
        },
        &|path: &Path| path.exists(),
    )
    .map_err(said)?;
    // The database is the one passed or the config file's, so it is always
    // known here: a missing home directory was refused above.
    let in_effect = DefaultDb::Required(NO_HOME_TO_KEEP_DB);
    let mut settings = resolve(
        env,
        platform,
        plan.check.clone(),
        in_effect,
        Report::Traced(&sources),
    )?;
    settings.web_key = plan.check_web_key.clone();
    let checked = check_settings(settings, &sources)?;
    Ok(ServicePlan {
        spec: plan.spec(program).map_err(said)?,
        new_key: plan.new_key,
        new_web_key: plan.new_web_key,
        bind: checked.bind,
        config_file: checked
            .config_file
            .or_else(|| kkt_config_file::config_file(env).map(|p| p.display().to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{TEST_PLATFORM, test_env, write_default_config};
    use crate::scratch;

    fn args(plan: &ServicePlan) -> Vec<&str> {
        plan.spec.args.iter().map(String::as_str).collect()
    }

    #[test]
    fn without_a_home_the_database_must_be_given() {
        assert_eq!(
            service_plan(&test_env(None), TEST_PLATFORM, None, None, None, false)
                .map(|p| p.spec.args),
            Err("no home directory to keep the database in: set HOME or pass --db".to_string())
        );
    }

    #[test]
    fn without_a_home_the_key_must_be_given_or_turned_off() {
        let dir = scratch::Dir::new("service-no-home-key");
        let db = dir.join("kkt.db");
        assert_eq!(
            service_plan(
                &test_env(None),
                TEST_PLATFORM,
                None,
                Some(db.clone()),
                None,
                false
            )
            .map(|p| p.spec.args),
            Err(
                "no home directory to keep the access key in: set HOME or pass --access-key-file"
                    .to_string()
            )
        );
        let plan = service_plan(
            &test_env(None),
            TEST_PLATFORM,
            None,
            Some(db.clone()),
            None,
            true,
        )
        .expect("valid");
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
        let plan = service_plan(&env, TEST_PLATFORM, None, None, None, false).expect("valid");
        let db = kkt_setup::default_db(&env, TEST_PLATFORM).expect("db");
        let key = crate::machine::settings::default_access_key(&env).expect("key");
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
        let plan = service_plan(&env, TEST_PLATFORM, None, None, None, false).expect("valid");
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
        let err = service_plan(&env, TEST_PLATFORM, None, None, None, false)
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

    /// #371, with a real socket: with another program listening on the
    /// address, the install is refused, naming the address and where to change it, rather than
    /// installing a server that fails and is started again every few
    /// seconds.
    #[test]
    fn an_address_in_use_is_refused_before_installing() {
        let other = TcpListener::bind("127.0.0.1:0").expect("listen");
        let bind = other.local_addr().expect("address");
        assert_eq!(
            check_bind(
                false,
                bind,
                Some("/home/sam/.config/kkt/config.toml"),
                &try_bind
            ),
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
        assert_eq!(check_bind(false, bind, None, &try_bind), Ok(()));
        TcpListener::bind(bind).expect("still free");
    }

    #[test]
    fn the_plan_names_the_default_config_file_for_bind() {
        let home = scratch::Dir::new("service-bind-config");
        let env = test_env(Some(&home));
        let plan = service_plan(&env, TEST_PLATFORM, None, None, None, false).expect("valid");
        assert_eq!(plan.bind, kkt_settings::DEFAULT_BIND);
        assert_eq!(
            plan.config_file,
            kkt_config_file::config_file(&env).map(|p| p.display().to_string())
        );
    }
}
