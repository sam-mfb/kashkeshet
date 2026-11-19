//! Lanes on this machine's server: `kkt lanes export` and `kkt lanes
//! import`, the server they act on as a contract ([`LaneServer`], which
//! the program picks through [`LaneServers`]), which server that is for a
//! deployment ([`lanes_on`]), its two implementations ([`LocalLanes`] and
//! [`ContainerLanes`]), the report of an import, and reading a lane
//! folder, checking it and importing it, which `kkt setup` also uses.

mod container;
mod report;

pub use container::{Carried, ContainerLanes, RunArg, ServerInContainer};

use crate::{LaneParts, OperatorParts};
use kkt_api_agent::LaneSource;
use kkt_api_operator::LaneSave;
use kkt_deployment::Deployment;
use kkt_lane_files as lane_files;
use kkt_rules::{DirectAccess, ImportedLane, LaneOutcome};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

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

/// A server's lanes, as the lane commands reach them: on a database this
/// machine opens ([`LocalLanes`]), or wherever else the program finds the
/// server.
pub trait LaneServer {
    /// Writes every current lane into the folder `dest` and says what it
    /// wrote.
    fn export<'a>(&'a self, dest: &'a Path) -> LaneFuture<'a, ()>;
    /// Whether this server can record `source` as where an import came
    /// from; the refusal to print if not. Asked before the folder is read.
    fn check_source(&self, source: Option<&LaneSource>) -> Result<(), String>;
    /// Imports the lanes read from a folder. Returns the report to print.
    fn import(&self, lanes: LaneImport) -> LaneFuture<'_, String>;
    /// The longest lane text the server takes.
    fn lane_limit(&self) -> Result<usize, String>;
}

/// What a [`LaneServer`] gives back, in time.
pub type LaneFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + 'a>>;

/// Lanes read from a folder, to import.
pub struct LaneImport {
    /// The folder's lanes, its skipped files and its commit.
    pub read: lane_files::Read,
    /// Retire the server's lanes the folder does not have.
    pub retire_missing: bool,
    /// Where the lanes came from, if not the folder they were read from.
    pub source: Option<LaneSource>,
}

/// The server `kkt lanes` acts on, by how kkt is deployed here.
#[derive(Debug, PartialEq, Eq)]
pub enum LanesOn {
    /// kkt in Docker, run from this folder.
    Docker(PathBuf),
    /// The local database `kkt serve` would use, or the one `--db` names.
    Local,
}

/// Which server `kkt lanes` acts on, given this machine's `deployment`
/// (`None` with no record) and whether `--db` or `--config` was given
/// (`local_options`).
pub fn lanes_on(deployment: Option<&Deployment>, local_options: bool) -> Result<LanesOn, String> {
    match deployment {
        Some(Deployment::Docker { folder, .. }) => {
            if local_options {
                return Err(
                    "kkt in Docker keeps its lanes in the container; --db and --config \
                            apply only to a local server"
                        .into(),
                );
            }
            Ok(LanesOn::Docker(folder.clone()))
        }
        Some(Deployment::Client) => Err("this machine is a client; run `kkt lanes` on the \
                                   machine where the server runs"
            .into()),
        Some(Deployment::Service { .. }) | None => Ok(LanesOn::Local),
    }
}

/// Which server `kkt lanes` acts on. The program picks it from what is
/// installed here.
pub trait LaneServers {
    /// The server `kkt lanes` acts on: what this machine runs, on the
    /// database and config file `kkt serve` would use unless `db` or
    /// `config` names others.
    fn here(
        &self,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    ) -> Result<Box<dyn LaneServer + '_>, String>;
}

/// `kkt lanes`: on the server [`LaneServers::here`] picks.
pub async fn run(
    parts: &OperatorParts<'_>,
    command: LanesAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let server = parts.lane_servers.here(db, config)?;
    match command {
        LanesAction::Export { folder } => server.export(&folder).await,
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
            server.check_source(source.as_ref())?;
            let read = read_lanes_warning(
                parts.lanes,
                parts.lane_commit,
                parts.output,
                &folder,
                retire_missing,
            )?;
            let report = server
                .import(LaneImport {
                    read,
                    retire_missing,
                    source,
                })
                .await?;
            parts.output.output(format_args!("{report}"));
            Ok(())
        }
    }
}

/// The lanes of the local database `kkt serve` would use, unless `db`
/// names another, with the config file `config` names, else the default.
pub struct LocalLanes<'a> {
    parts: LaneParts<'a>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
}

impl<'a> LocalLanes<'a> {
    /// Nothing is read until a lane command asks.
    pub fn new(parts: LaneParts<'a>, db: Option<PathBuf>, config: Option<PathBuf>) -> Self {
        LocalLanes { parts, db, config }
    }
}

impl LaneServer for LocalLanes<'_> {
    fn export<'a>(&'a self, dest: &'a Path) -> LaneFuture<'a, ()> {
        Box::pin(async move {
            let parts = &self.parts;
            let (operator, _) = local_operator(parts, self.db.clone(), self.config.clone())?;
            let lanes = operator.current_lanes().await.map_err(|e| e.to_string())?;
            let files: Vec<lane_files::LaneFile> = lanes
                .iter()
                .map(|l| lane_files::LaneFile {
                    name: l.name.clone(),
                    description: l.current.description.clone(),
                    text: l.current.text.clone(),
                })
                .collect();
            let written =
                lane_files::write_folder(parts.lanes, dest, &files).map_err(|e| e.to_string())?;
            for l in &lanes {
                parts.output.output(format_args!(
                    "{} (version {})\n",
                    lane_files::file_name(&l.name),
                    l.current.version
                ));
            }
            parts.output.errors(format_args!(
                "Exported {} lanes to {}\n",
                lanes.len(),
                written.display()
            ));
            Ok(())
        })
    }
    fn check_source(&self, _: Option<&LaneSource>) -> Result<(), String> {
        Ok(())
    }
    fn import(&self, lanes: LaneImport) -> LaneFuture<'_, String> {
        Box::pin(local_import_lanes(
            &self.parts,
            self.db.clone(),
            self.config.clone(),
            lanes.read,
            lanes.retire_missing,
            lanes.source,
        ))
    }
    fn lane_limit(&self) -> Result<usize, String> {
        Ok(self
            .parts
            .serve
            .config(self.config.clone())
            .map_err(|e| e.to_string())?
            .max_lane_chars)
    }
}

/// Why the local database could not be used: where it is could not be
/// worked out, there is none there, or it could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error(transparent)]
    Path(crate::PartError),
    #[error("no database at {}; pass --db with the server's database file", path.display())]
    Missing { path: PathBuf },
    #[error(transparent)]
    Open(crate::PartError),
}

/// The database file `kkt serve` would use given the config file's `db`,
/// unless `db` names another. It must exist.
pub fn existing_db(
    parts: &LaneParts<'_>,
    db: Option<PathBuf>,
    config_db: Option<PathBuf>,
) -> Result<PathBuf, DatabaseError> {
    let path = parts
        .serve
        .database(db, config_db)
        .map_err(DatabaseError::Path)?;
    if !parts.database.exists(&path) {
        return Err(DatabaseError::Missing { path });
    }
    Ok(path)
}

/// The operator's commands on the local database `kkt serve` would use,
/// unless `db` names another, and the server's `max_lane_chars` from the
/// config file.
pub fn local_operator(
    parts: &LaneParts<'_>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(DirectAccess, usize), String> {
    let from_file = parts.serve.config(config).map_err(|e| e.to_string())?;
    let max_lane_chars = from_file.max_lane_chars;
    let path = existing_db(parts, db, from_file.db).map_err(|e| e.to_string())?;
    let db = parts.database.open(&path).map_err(|e| e.to_string())?;
    // Key mode: the one swarm. Lanes belong to the server, not to a swarm.
    Ok((
        DirectAccess::new(db, kkt_api_agent::SwarmId::main(), parts.clock.clone()),
        max_lane_chars,
    ))
}

/// Imports the lanes `read` from a folder into the local database. The
/// source recorded is `source` if given, else the folder and its git
/// commit. Returns the report to print.
async fn local_import_lanes(
    parts: &LaneParts<'_>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
    read: lane_files::Read,
    retire_missing: bool,
    source: Option<LaneSource>,
) -> Result<String, String> {
    let (operator, max_lane_chars) = local_operator(parts, db, config)?;
    let skipped_files = read.skipped_names();
    let source = source.unwrap_or_else(|| LaneSource {
        folder: read.folder.display().to_string(),
        commit: read.commit,
    });
    let lanes = lane_saves(read.lanes);
    let done = operator
        .import_lanes(kkt_rules::LaneImport {
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
fn read_lanes_warning(
    disk: &dyn lane_files::Disk,
    commits: &dyn kkt_lane_files::LaneCommit,
    output: &dyn crate::Output,
    folder: &Path,
    retire_missing: bool,
) -> Result<lane_files::Read, String> {
    let read = lane_files::read_folder(disk, commits, folder).map_err(|e| e.to_string())?;
    warn_skipped(output, &read.skipped);
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
fn warn_skipped(output: &dyn crate::Output, skipped: &[lane_files::Skipped]) {
    for file in skipped {
        output.errors(format_args!("warning: {file}\n"));
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

/// The lane limit from this machine's default serve config, else the owner's default.
pub fn local_lane_limit(parts: &LaneParts<'_>) -> Result<usize, String> {
    Ok(parts
        .serve
        .config(None)
        .map_err(|e| e.to_string())?
        .max_lane_chars)
}

/// Reads the lane files in `folder` and checks them as `kkt lanes import`
/// would on a server whose limit is `max_lane_chars`, changing nothing: the
/// files' format, then each lane by the lane rules. Warns of each file it
/// skips, as the import would. Returns how many lanes there are; an error
/// names the file or lane that is wrong.
pub(crate) fn check_lanes_folder(
    disk: &dyn lane_files::Disk,
    commits: &dyn kkt_lane_files::LaneCommit,
    output: &dyn crate::Output,
    folder: &Path,
    max_lane_chars: usize,
) -> Result<usize, String> {
    let read = lane_files::read_folder(disk, commits, folder).map_err(|e| e.to_string())?;
    warn_skipped(output, &read.skipped);
    let count = read.lanes.len();
    kkt_rules::check_lane_import(&lane_saves(read.lanes), max_lane_chars)
        .map_err(|e| format!("{}: {}", read.folder.display(), e.message))?;
    Ok(count)
}

/// Reads the lane files in `folder` and checks them as an import into
/// `server`, the server `kkt setup` sets up, would, changing nothing:
/// against that server's lane limit. Returns how many lanes there are.
pub fn check_lanes_for_setup(
    parts: &LaneParts<'_>,
    server: &dyn LaneServer,
    folder: &Path,
) -> Result<usize, String> {
    let max_lane_chars = server.lane_limit()?;
    check_lanes_folder(
        parts.lanes,
        parts.lane_commit,
        parts.output,
        folder,
        max_lane_chars,
    )
}

/// Imports the lanes in `folder` into `server`, the server this machine's
/// setup record says it runs, and returns the report to print. The check
/// before setup began warned of the files the import skips.
pub fn import_lanes_for_setup(
    parts: &LaneParts<'_>,
    server: &dyn LaneServer,
    folder: &Path,
) -> Result<String, String> {
    let read = lane_files::read_folder(parts.lanes, parts.lane_commit, folder)
        .map_err(|e| e.to_string())?;
    let report = tokio::runtime::Handle::current().block_on(server.import(LaneImport {
        read,
        retire_missing: false,
        source: None,
    }))?;
    Ok(report.trim_end().to_string())
}

/// One line for each lane an import touched, then one saying where from and
/// how many of each.
fn import_report(done: &[ImportedLane], source: &LaneSource) -> String {
    report::import_report(
        done.iter().map(|lane| {
            use report::Outcome;
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

    fn docker() -> Deployment {
        Deployment::Docker {
            folder: PathBuf::from("/data/docker"),
            tailscale: false,
            hostname: None,
            tailnet: None,
        }
    }

    const SERVICE: Deployment = Deployment::Service { tailscale: false };

    #[test]
    fn lanes_act_on_docker_in_its_folder_without_local_options() {
        assert_eq!(
            lanes_on(Some(&docker()), false),
            Ok(LanesOn::Docker(PathBuf::from("/data/docker")))
        );
        assert_eq!(
            lanes_on(Some(&docker()), true),
            Err("kkt in Docker keeps its lanes in the container; --db and --config apply only to a local server".into())
        );
    }

    #[test]
    fn lanes_act_on_the_local_database_for_a_service_or_no_record() {
        for local_options in [false, true] {
            assert_eq!(lanes_on(Some(&SERVICE), local_options), Ok(LanesOn::Local));
            assert_eq!(lanes_on(None, local_options), Ok(LanesOn::Local));
        }
    }

    #[test]
    fn lanes_are_refused_on_a_client() {
        assert_eq!(
            lanes_on(Some(&Deployment::Client), false),
            Err(
                "this machine is a client; run `kkt lanes` on the machine where the server runs"
                    .into()
            )
        );
    }
}
