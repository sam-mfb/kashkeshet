//! Adapts the program's service command entry points to the operator assembly.
use kkt_machine::{Env, Platform};
pub use kkt_operate::service::{ServiceAction, ServiceInstall, ServicePlan};
use std::path::PathBuf;

pub async fn run(env: &Env, platform: Platform, command: ServiceAction) -> Result<(), String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform);
    kkt_operate::service::run(&native.parts(), env, command).await
}
pub fn service_plan(
    env: &Env,
    platform: Platform,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    access_key_file: Option<PathBuf>,
    no_access_key: bool,
) -> Result<ServicePlan, String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform);
    kkt_operate::service::service_plan(
        &native.parts(),
        ServiceInstall {
            config,
            db,
            access_key_file,
            no_access_key,
            dry_run: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{TEST_PLATFORM, test_env, write_default_config};
    use crate::scratch;
    use kkt_service::check_bind;
    use kkt_service_os::try_bind;
    use std::net::TcpListener;

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
        assert_eq!(plan.new_web_key, Some(kkt_admission::web_key_path(&key)));
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

    /// A key file given with no key is refused by the check after the plan,
    /// naming the setting the flag gave, not the config file.
    #[test]
    fn a_key_file_given_with_no_key_is_refused() {
        let home = scratch::Dir::new("service-key-and-no-key");
        let env = test_env(Some(&home));
        write_default_config(&home, "access_key_file = \"/keys/key\"");
        assert_eq!(
            service_plan(&env, TEST_PLATFORM, None, None, None, true).map(|p| p.spec.args),
            Err(
                "invalid setting: no_access_key cannot be set together with access_key_file"
                    .to_string()
            )
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
        assert_eq!(plan.bind, kkt_hosts::DEFAULT_BIND);
        assert_eq!(
            plan.config_file,
            kkt_config_file::config_file(env.config_home().as_deref())
                .map(|p| p.display().to_string())
        );
    }
}
