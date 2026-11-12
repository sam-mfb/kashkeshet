//! Wires the operator's lane commands to this program's database, disk and configuration.
mod disk;
use crate::machine::settings::read_config;
pub(crate) use disk::LaneDisk;
use kkt_api_agent::LaneSource;
use kkt_lane_files as lane_files;
use kkt_machine::{Env, Platform};
pub use kkt_operate::lanes::LanesAction;
use std::path::{Path, PathBuf};

pub async fn run(
    env: &Env,
    platform: Platform,
    command: LanesAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform, None);
    kkt_operate::lanes::run(&native.parts(), env, command, db, config).await
}

pub(crate) fn existing_db(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config_db: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform, None);
    kkt_operate::lanes::existing_db(&native.parts(), db, config_db)
}

#[cfg(test)]
fn local_operator(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(kkt_rules::CommandLine, usize), String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform, None);
    kkt_operate::lanes::local_operator(&native.parts(), db, config)
}

pub(crate) async fn local_import_lanes(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
    read: lane_files::Read,
    retire_missing: bool,
    source: Option<LaneSource>,
) -> Result<String, String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform, None);
    kkt_operate::lanes::local_import_lanes(
        &native.parts(),
        db,
        config,
        read,
        retire_missing,
        source,
    )
    .await
}

pub(crate) fn docker_lane_limit(env: &Env, platform: Platform) -> Result<usize, String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform, None);
    kkt_operate::lanes::docker_lane_limit(&native.parts(), env, platform)
}

pub(crate) fn check_lanes_folder(folder: &Path, max_lane_chars: usize) -> Result<usize, String> {
    kkt_operate::lanes::check_lanes_folder(
        &LaneDisk,
        &crate::machine::operate::NativeOutput,
        folder,
        max_lane_chars,
    )
}

pub(crate) fn docker_import_lanes(
    folder: &kkt_container::Folder,
    read: lane_files::Read,
    retire_missing: bool,
) -> Result<String, String> {
    kkt_operate::lanes::docker_import_lanes(
        &crate::machine::container::ContainerHost,
        folder,
        read,
        retire_missing,
    )
}

pub(crate) fn local_lane_limit(env: &Env) -> Result<usize, String> {
    let (_, from_file) = read_config(env, None)?;
    Ok(from_file.lane_limit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{TEST_PLATFORM, test_env, write_default_config};
    use crate::scratch;

    #[test]
    fn the_database_is_the_given_one_else_the_config_files_else_the_default() {
        let home = scratch::Dir::new("lanes-existing-db");
        let env = test_env(Some(&home));
        let given = home.join("given.db");
        let from_file = home.join("file.db");
        let missing = home.join("missing.db");
        std::fs::write(&given, "").expect("file");
        std::fs::write(&from_file, "").expect("file");
        assert_eq!(
            existing_db(
                &env,
                TEST_PLATFORM,
                Some(given.clone()),
                Some(from_file.clone())
            ),
            Ok(given)
        );
        assert_eq!(
            existing_db(&env, TEST_PLATFORM, None, Some(from_file.clone())),
            Ok(from_file)
        );
        assert_eq!(
            existing_db(&env, TEST_PLATFORM, Some(missing.clone()), None),
            Err(format!(
                "no database at {}; pass --db with the server's database file",
                missing.display()
            ))
        );
        let default = kkt_setup::default_db(&env, TEST_PLATFORM).expect("default");
        assert_eq!(
            existing_db(&env, TEST_PLATFORM, None, None),
            Err(format!(
                "no database at {}; pass --db with the server's database file",
                default.display()
            ))
        );
        std::fs::create_dir_all(default.parent().expect("folder")).expect("folder");
        std::fs::write(&default, "").expect("file");
        assert_eq!(existing_db(&env, TEST_PLATFORM, None, None), Ok(default));
    }

    #[test]
    fn without_a_home_the_database_must_be_given() {
        assert_eq!(
            existing_db(&test_env(None), TEST_PLATFORM, None, None),
            Err("no home directory to find the database in: set HOME or pass --db".to_string())
        );
    }

    #[test]
    fn the_lane_limit_is_the_config_files_else_the_default() {
        let home = scratch::Dir::new("lanes-limit");
        let env = test_env(Some(&home));
        let db = home.join("kkt.db");
        crate::machine::database::open(&db).expect("database");
        assert_eq!(
            local_lane_limit(&env),
            Ok(kkt_rules::Settings::DEFAULT.max_lane_chars)
        );
        let limit =
            |env: &Env| local_operator(env, TEST_PLATFORM, Some(db.clone()), None).map(|(_, l)| l);
        assert_eq!(limit(&env), Ok(kkt_rules::Settings::DEFAULT.max_lane_chars));
        write_default_config(&home, "max_lane_chars = 1234\nname_pack = \"missing\"");
        assert_eq!(local_lane_limit(&env), Ok(1234));
        assert_eq!(limit(&env), Ok(1234));
    }
}
