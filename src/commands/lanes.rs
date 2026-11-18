//! The lane folder on this machine's disk and git, which the operator's lane
//! commands take (`LaneDisk`, `LaneGit`), and kkt installed in a container as
//! they take it (`LanesInContainer`); and tests of those commands over this
//! program's real parts, among them kkt-operator-commands' `ContainerLanes`
//! over `LanesInContainer`.
mod commit;
mod disk;
mod in_container;
pub(crate) use commit::LaneGit;
pub(crate) use disk::LaneDisk;
pub(crate) use in_container::LanesInContainer;
#[cfg(test)]
use kkt_machine::{Env, Platform};
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
fn existing_db(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config_db: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let native = crate::machine::native::NativeParts::new(env, platform);
    kkt_operator_commands::lanes::existing_db(&native.lane_parts(), db, config_db)
}

#[cfg(test)]
fn local_operator(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(kkt_rules::CommandLine, usize), String> {
    let native = crate::machine::native::NativeParts::new(env, platform);
    kkt_operator_commands::lanes::local_operator(&native.lane_parts(), db, config)
}

#[cfg(test)]
fn local_lane_limit(env: &Env) -> Result<usize, String> {
    let native =
        crate::machine::native::NativeParts::new(env, crate::machine::settings::TEST_PLATFORM);
    kkt_operator_commands::lanes::local_lane_limit(&native.lane_parts())
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
        let default = kkt_deployment::default_db(&env, TEST_PLATFORM).expect("default");
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

    /// kkt-operator-commands' `ContainerLanes` over `LanesInContainer`, with
    /// this program's reader of the container's config file, in a container
    /// installed in `/data/docker`.
    mod container {
        use futures::future::BoxFuture;
        use kkt_clock::{Clock, Time};
        use kkt_container::Folder;
        use kkt_containerized_service::testing::{Memory, Run};
        use kkt_containerized_service::{
            Arg, Carry, Inputs, KeyCopies, LoginLink, ServiceContainer,
        };
        use kkt_operator_commands::lanes::{ContainerLanes, LaneServer};
        use kkt_operator_commands::{Output, Paths};
        use std::fmt::Arguments;
        use std::path::{Path, PathBuf};

        struct Unused;
        impl Clock for Unused {
            fn now(&self) -> Time {
                panic!("no clock is read")
            }
            fn sleep_until(&self, _: Time) -> BoxFuture<'_, ()> {
                panic!("nothing sleeps")
            }
        }
        impl LoginLink for Unused {
            fn show(&self, _: &str) {
                panic!("no login")
            }
            fn open_requested(&self) -> bool {
                panic!("no login")
            }
        }
        impl Paths for Unused {
            fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
                panic!("no path is resolved")
            }
            fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
                panic!("no path is resolved")
            }
        }
        impl Output for Unused {
            fn output(&self, _: Arguments<'_>) {
                panic!("nothing is said")
            }
            fn errors(&self, _: Arguments<'_>) {
                panic!("nothing is said")
            }
        }

        fn folder() -> Folder {
            Folder("/data/docker".into())
        }

        fn installed() -> Memory {
            let memory = Memory::new(None, String::new());
            let inputs = Inputs {
                version: "0.4.0".into(),
                project: "kkt".into(),
                port: 7420,
                allow_hosts: vec![],
                sharing: None,
            };
            let keys = KeyCopies {
                access: "/k/a".into(),
                web: "/k/w".into(),
                admin: "/k/m".into(),
            };
            futures::executor::block_on(memory.provision(
                &Unused,
                &Unused,
                &folder(),
                &inputs,
                &keys,
            ))
            .expect("installed");
            memory
        }

        fn lanes(memory: &Memory) -> ContainerLanes<'_> {
            ContainerLanes {
                container: Box::new(super::super::LanesInContainer {
                    container: memory,
                    configs: &crate::machine::settings::ContainerConfigs,
                    folder: folder(),
                }),
                paths: &Unused,
                output: &Unused,
            }
        }

        /// An import runs kkt in the container in its folder, with each
        /// argument and the files carried in as the lane commands gave them.
        #[test]
        fn an_import_runs_in_the_containers_folder_with_its_arguments() {
            let memory = installed();
            let read = kkt_lane_files::Read {
                lanes: vec![kkt_lane_files::LaneFile {
                    name: "reviewer".into(),
                    description: String::new(),
                    text: "Do the work.".into(),
                }],
                skipped: vec![],
                folder: "/home/me/lanes".into(),
                commit: None,
            };
            futures::executor::block_on(lanes(&memory).import(
                kkt_operator_commands::lanes::LaneImport {
                    read,
                    retire_missing: false,
                    source: None,
                },
            ))
            .expect("imported");
            let text = |text: &str| Arg::Text(text.into());
            assert_eq!(
                memory.runs(),
                [Run {
                    folder: folder(),
                    args: vec![
                        text("lanes"),
                        text("import"),
                        Arg::Folder,
                        text("--source-folder"),
                        text("/home/me/lanes"),
                    ],
                    folder_prefix: "/tmp/kkt-lanes-import".into(),
                    carry: Carry::In(vec!["/home/me/lanes/reviewer.md".into()]),
                }]
            );
        }

        #[test]
        fn the_lane_limit_is_the_containers_own_setting_or_the_default() {
            let default = kkt_rules::Settings::DEFAULT.max_lane_chars;

            // No install yet: the default, and nothing is read.
            let memory = Memory::new(None, String::new());
            assert_eq!(lanes(&memory).lane_limit(), Ok(default));
            assert!(memory.config_home_reads().is_empty());

            // An install with no config file, or one without the setting.
            let memory = installed();
            assert_eq!(lanes(&memory).lane_limit(), Ok(default));
            memory
                .set_config_home_file("kkt/config.toml".into(), "name_pack = \"animals\"\n".into());
            assert_eq!(lanes(&memory).lane_limit(), Ok(default));

            // Its own setting, read from where `kkt` in the container finds
            // its config file; a path under the home folder reads too.
            memory.set_config_home_file(
                "kkt/config.toml".into(),
                "db = \"~/kkt.db\"\nmax_lane_chars = 20\n".into(),
            );
            assert_eq!(lanes(&memory).lane_limit(), Ok(20));
            assert_eq!(
                memory.config_home_reads(),
                vec![PathBuf::from("kkt/config.toml"); 3]
            );
        }

        #[test]
        fn a_lane_limit_that_cannot_be_read_names_the_setting() {
            let memory = installed();
            memory.set_config_home_file("kkt/config.toml".into(), "max_lane_chars = 0.5\n".into());
            let err = lanes(&memory).lane_limit().expect_err("refused");
            assert!(
                err.starts_with(
                    "could not read the lane text limit of kkt in Docker: unexpected output from \
                     the kkt container's config file: "
                ),
                "{err}"
            );
            assert!(err.contains("max_lane_chars"), "{err}");
        }
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
