//! Lanes on this machine's server: `kkt lanes export` and `kkt lanes
//! import`, and reading a lane folder, checking it, and importing it into a
//! local database or kkt in Docker, which `kkt setup` also uses.

mod disk;
pub(crate) use disk::LaneDisk;

use crate::machine::container::ContainerHost;
use crate::machine::record::{docker_folder, read_setup};
use crate::machine::settings::{
    ContainerConfigs, DefaultDb, NO_HOME_TO_FIND_DB, database, read_config,
};
use kkt_api_agent::LaneSource;
use kkt_api_operator::LaneSave;
use kkt_clock_system::SystemClock;
use kkt_lane_files as lane_files;
use kkt_machine::{Env, Platform};
use kkt_rules::{CommandLine, ImportedLane, LaneImport, LaneOutcome};
use kkt_settings::Limits;
use kkt_setup as setup;
use kkt_terminal_os::Terminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What `kkt lanes` does.
pub enum LanesAction {
    Export {
        folder: PathBuf,
    },
    Import {
        folder: PathBuf,
        retire_missing: bool,
        source_folder: Option<String>,
        source_commit: Option<String>,
    },
}

/// `kkt lanes`: on the database `kkt serve` would use unless `--db` names
/// another, or for kkt in Docker, in its container.
pub async fn run(
    env: &Env,
    platform: Platform,
    command: LanesAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let mode = read_setup(env)?.map(|record| record.mode);
    match setup::lanes_on(mode.as_ref(), db.is_some() || config.is_some())? {
        setup::LanesOn::Docker(folder) => {
            let folder = kkt_container::Folder(folder);
            match command {
                LanesAction::Export { folder: dest } => {
                    let dest = kkt_setup_os::absolute(&dest)
                        .map_err(|e| format!("could not resolve {}: {e}", dest.display()))?;
                    let said = kkt_container::export_lanes(&ContainerHost, &folder, &dest)
                        .map_err(|e| e.to_string())?;
                    Terminal.output(format_args!("{said}"));
                    Terminal.errors(format_args!("Exported to {}\n", dest.display()));
                }
                LanesAction::Import {
                    folder: from,
                    retire_missing,
                    source_folder,
                    ..
                } => {
                    if source_folder.is_some() {
                        return Err(
                            "--source-folder is for `kkt lanes import` in the container \
                                    itself"
                                .into(),
                        );
                    }
                    let read = read_lanes_warning(&from, retire_missing)?;
                    Terminal.output(format_args!(
                        "{}",
                        docker_import_lanes(&folder, read, retire_missing)?
                    ));
                }
            }
            Ok(())
        }
        setup::LanesOn::Local => match command {
            LanesAction::Export { folder } => {
                let (operator, _) = local_operator(env, platform, db, config)?;
                let lanes = operator.current_lanes().await.map_err(|e| e.to_string())?;
                let files: Vec<lane_files::LaneFile> = lanes
                    .iter()
                    .map(|l| lane_files::LaneFile {
                        name: l.name.clone(),
                        description: l.current.description.clone(),
                        text: l.current.text.clone(),
                    })
                    .collect();
                let written = lane_files::write_folder(&LaneDisk, &folder, &files)?;
                for l in &lanes {
                    Terminal.output(format_args!(
                        "{} (version {})\n",
                        lane_files::file_name(&l.name),
                        l.current.version
                    ));
                }
                Terminal.errors(format_args!(
                    "Exported {} lanes to {}\n",
                    lanes.len(),
                    written.display()
                ));
                Ok(())
            }
            LanesAction::Import {
                folder,
                retire_missing,
                source_folder,
                source_commit,
            } => {
                let source = source_folder.map(|folder| LaneSource {
                    folder,
                    commit: source_commit,
                });
                let read = read_lanes_warning(&folder, retire_missing)?;
                let report =
                    local_import_lanes(env, platform, db, config, read, retire_missing, source)
                        .await?;
                Terminal.output(format_args!("{report}"));
                Ok(())
            }
        },
    }
}

/// The database file `kkt serve` would use given the config file's `db`,
/// unless `db` names another. It must exist.
pub(crate) fn existing_db(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config_db: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let path = database(
        env,
        platform,
        db.or(config_db),
        DefaultDb::Required(NO_HOME_TO_FIND_DB),
    )?;
    if !kkt_db_sqlite::SqlitePlace::exists(&path) {
        return Err(format!(
            "no database at {}; pass --db with the server's database file",
            path.display()
        ));
    }
    Ok(path)
}

/// The operator's commands on the local database `kkt serve` would use,
/// unless `db` names another, and the server's `max_lane_chars` from the
/// config file.
fn local_operator(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(CommandLine, usize), String> {
    let (_, from_file) = read_config(env, config)?;
    let max_lane_chars = from_file.lane_limit();
    let path = existing_db(env, platform, db, from_file.db)?;
    let db = crate::machine::database::open(&path)?;
    // Key mode: the one swarm. Lanes belong to the server, not to a swarm.
    Ok((
        CommandLine::new(db, kkt_api_agent::SwarmId::main(), Arc::new(SystemClock)),
        max_lane_chars,
    ))
}

/// Imports the lanes `read` from a folder into the local database. The
/// source recorded is `source` if given, else the folder and its git
/// commit. Returns the report to print.
pub(crate) async fn local_import_lanes(
    env: &Env,
    platform: Platform,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
    read: lane_files::Read,
    retire_missing: bool,
    source: Option<LaneSource>,
) -> Result<String, String> {
    let (operator, max_lane_chars) = local_operator(env, platform, db, config)?;
    let skipped_files = read.skipped_names();
    let source = source.unwrap_or_else(|| LaneSource {
        folder: read.folder.display().to_string(),
        commit: read.commit,
    });
    let lanes = lane_saves(read.lanes);
    let done = operator
        .import_lanes(LaneImport {
            lanes,
            source: source.clone(),
            retire_missing,
            skipped_files,
            max_lane_chars,
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(import_report(&done, &source))
}

/// Reads the lane folder for `kkt lanes import`, printing a warning on
/// standard error for each file it skips before anything is imported. With
/// `retire_missing`, a folder that could only retire every lane is refused.
fn read_lanes_warning(folder: &Path, retire_missing: bool) -> Result<lane_files::Read, String> {
    let read = lane_files::read_folder(&LaneDisk, folder)?;
    warn_skipped(&read.skipped);
    if retire_missing {
        kkt_rules::check_retire_missing(
            &read.folder.display().to_string(),
            read.lanes.len(),
            read.skipped.len(),
        )
        .map_err(|e| e.message)?;
    }
    Ok(read)
}

/// One warning line on standard error for each file a lane folder's read
/// skipped.
fn warn_skipped(skipped: &[lane_files::Skipped]) {
    for file in skipped {
        Terminal.errors(format_args!("warning: {file}\n"));
    }
}

/// Lane files as the saves an import makes.
fn lane_saves(files: Vec<lane_files::LaneFile>) -> Vec<LaneSave> {
    files
        .into_iter()
        .map(|l| LaneSave {
            name: l.name,
            description: l.description,
            text: l.text,
        })
        .collect()
}

/// The `max_lane_chars` of a server on this machine run on the config file
/// `kkt serve` finds here: its setting, else the default.
pub(crate) fn local_lane_limit(env: &Env) -> Result<usize, String> {
    let (_, from_file) = read_config(env, None)?;
    Ok(from_file.lane_limit())
}

/// The `max_lane_chars` of kkt in Docker set up in the folder `kkt setup`
/// uses: the container's own setting when the folder holds an install, and
/// the default for a new one, whose container starts with no config file.
/// The host's config file plays no part; the container never reads it.
pub(crate) fn docker_lane_limit(env: &Env, platform: Platform) -> Result<usize, String> {
    let folder = docker_folder(env, platform)?;
    setup::lane_reports::docker_limit(
        kkt_container::Host::exists(&ContainerHost, &folder.join(kkt_container::ENV_FILE)),
        Limits::DEFAULT.max_lane_chars,
        || {
            kkt_container::lane_limit(
                &ContainerHost,
                &kkt_container::Folder(folder),
                &ContainerConfigs,
                Limits::DEFAULT.max_lane_chars,
            )
            .map_err(|e| e.to_string())
        },
    )
}

/// Reads the lane files in `folder` and checks them as `kkt lanes import`
/// would on a server whose limit is `max_lane_chars`, changing nothing: the
/// files' format, then each lane by the lane rules. Warns of each file it
/// skips, as the import would. Returns how many lanes there are; an error
/// names the file or lane that is wrong.
pub(crate) fn check_lanes_folder(folder: &Path, max_lane_chars: usize) -> Result<usize, String> {
    let read = lane_files::read_folder(&LaneDisk, folder)?;
    warn_skipped(&read.skipped);
    let count = read.lanes.len();
    kkt_rules::check_lane_import(&lane_saves(read.lanes), max_lane_chars)
        .map_err(|e| format!("{}: {}", read.folder.display(), e.message))?;
    Ok(count)
}

/// Imports the lanes `read` from a folder on the host into kkt in Docker
/// in `folder`. The files were read here first, so a mistake is named by
/// its path on the host; [`lane_files::Read::files_to_copy`] says which go
/// in. Returns the report to print.
pub(crate) fn docker_import_lanes(
    folder: &kkt_container::Folder,
    read: lane_files::Read,
    retire_missing: bool,
) -> Result<String, String> {
    let import = kkt_container::LanesImport {
        files: read.files_to_copy(retire_missing),
        source_folder: read.folder.display().to_string(),
        source_commit: read.commit,
        retire_missing,
    };
    kkt_container::import_lanes(&ContainerHost, folder, &import).map_err(|e| e.to_string())
}

/// One line for each lane an import touched, then one saying where from and
/// how many of each.
fn import_report(done: &[ImportedLane], source: &LaneSource) -> String {
    setup::lane_reports::import_report(
        done.iter().map(|lane| {
            use setup::lane_reports::Outcome;
            let outcome = match lane.outcome {
                LaneOutcome::Created => Outcome::Created,
                LaneOutcome::Updated(version) => Outcome::Updated(version),
                LaneOutcome::Unchanged(version) => Outcome::Unchanged(version),
                LaneOutcome::RetiredSkipped => Outcome::RetiredSkipped,
                LaneOutcome::Retired => Outcome::Retired,
                LaneOutcome::FileSkipped => Outcome::FileSkipped,
            };
            (lane.name.as_str(), outcome)
        }),
        &source.folder,
        source.commit.as_deref(),
    )
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
        assert_eq!(local_lane_limit(&env), Ok(Limits::DEFAULT.max_lane_chars));
        let limit =
            |env: &Env| local_operator(env, TEST_PLATFORM, Some(db.clone()), None).map(|(_, l)| l);
        assert_eq!(limit(&env), Ok(Limits::DEFAULT.max_lane_chars));
        write_default_config(&home, "max_lane_chars = 1234\nname_pack = \"missing\"");
        assert_eq!(local_lane_limit(&env), Ok(1234));
        assert_eq!(limit(&env), Ok(1234));
    }
}
