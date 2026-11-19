//! Lanes on this machine's server: `kkt lanes export` and `kkt lanes
//! import`, the server they act on as a contract ([`LaneServer`], which
//! the program picks through [`LaneServers`]), which server that is for a
//! deployment ([`lanes_on`]), its two implementations ([`LocalLanes`] and
//! [`ContainerLanes`]), the report of an import, and reading a lane
//! folder, checking it and importing it, which `kkt setup` also uses.

mod container;
mod report;

pub use container::{Carried, ContainerLanes, RunArg, ServerInContainer};

use crate::{LaneParts, OperatorParts, PartError};
use kkt_api_agent::{LaneSource, RuleError};
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

/// Why a lane command failed: a source's own error, or the step here
/// that failed.
#[derive(Debug, thiserror::Error)]
pub enum LaneError {
    /// The server `kkt lanes` acts on could not be picked.
    #[error(transparent)]
    Here(PartError),
    /// The serve config could not be read.
    #[error(transparent)]
    Config(PartError),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Rule(#[from] RuleError),
    #[error(transparent)]
    Read(#[from] lane_files::ReadError),
    #[error(transparent)]
    Write(#[from] lane_files::WriteError),
    /// A run of kkt in the container failed.
    #[error(transparent)]
    Run(PartError),
    /// `--retire-missing` would retire every lane: the rule's message.
    #[error("{}", .0.message)]
    RetireAll(RuleError),
    /// A lane in `folder` that the lane rules refuse.
    #[error("{}: {}", folder.display(), source.message)]
    Check {
        folder: PathBuf,
        #[source]
        source: RuleError,
    },
    #[error("--source-folder is for `kkt lanes import` in the container itself")]
    SourceInContainer,
    #[error("could not resolve {}: {source}", path.display())]
    Resolve {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read the lane text limit of kkt in Docker: {0}")]
    LimitUnread(PartError),
    #[error(
        "could not read the lane text limit of kkt in Docker: unexpected output from the kkt \
         container's config file: {0}"
    )]
    LimitSetting(PartError),
}

/// Why `kkt lanes` cannot act on this machine's server.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LanesOnError {
    #[error(
        "kkt in Docker keeps its lanes in the container; --db and --config apply only to a \
         local server"
    )]
    DockerTakesNoLocalOptions,
    #[error("this machine is a client; run `kkt lanes` on the machine where the server runs")]
    OnAClient,
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
    fn check_source(&self, source: Option<&LaneSource>) -> Result<(), LaneError>;
    /// Imports the lanes read from a folder. Returns the report to print.
    fn import(&self, lanes: LaneImport) -> LaneFuture<'_, String>;
    /// The longest lane text the server takes.
    fn lane_limit(&self) -> Result<usize, LaneError>;
}

/// What a [`LaneServer`] gives back, in time.
pub type LaneFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, LaneError>> + 'a>>;

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
pub fn lanes_on(
    deployment: Option<&Deployment>,
    local_options: bool,
    action: &LanesAction,
) -> Result<LanesOn, LanesOnError> {
    match deployment {
        Some(Deployment::Docker { folder, .. }) => {
            if local_options {
                return Err(LanesOnError::DockerTakesNoLocalOptions);
            }
            Ok(LanesOn::Docker(folder.clone()))
        }
        Some(Deployment::Client)
            if local_options && matches!(action, LanesAction::Import { .. }) =>
        {
            Ok(LanesOn::Local)
        }
        Some(Deployment::Client) => Err(LanesOnError::OnAClient),
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
        action: &LanesAction,
        db: Option<PathBuf>,
        config: Option<PathBuf>,
    ) -> Result<Box<dyn LaneServer + '_>, PartError>;
}

/// `kkt lanes`: on the server [`LaneServers::here`] picks.
pub async fn run(
    parts: &OperatorParts<'_>,
    command: LanesAction,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), LaneError> {
    let server = parts
        .lane_servers
        .here(&command, db, config)
        .map_err(LaneError::Here)?;
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
            let (access, _) = local_operator(parts, self.db.clone(), self.config.clone())?;
            let lanes = access.current_lanes(&command_line_operator()).await?;
            let files: Vec<lane_files::LaneFile> = lanes
                .iter()
                .map(|l| lane_files::LaneFile {
                    name: l.name.clone(),
                    description: l.current.description.clone(),
                    text: l.current.text.clone(),
                })
                .collect();
            let written = lane_files::write_folder(parts.lanes, dest, &files)?;
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
    fn check_source(&self, _: Option<&LaneSource>) -> Result<(), LaneError> {
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
    fn lane_limit(&self) -> Result<usize, LaneError> {
        Ok(self
            .parts
            .serve
            .config(self.config.clone())
            .map_err(LaneError::Config)?
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
) -> Result<(DirectAccess, usize), LaneError> {
    let from_file = parts.serve.config(config).map_err(LaneError::Config)?;
    let max_lane_chars = from_file.max_lane_chars;
    let path = existing_db(parts, db, from_file.db)?;
    let db = parts.database.open(&path).map_err(DatabaseError::Open)?;
    // Key mode: the one swarm. Lanes belong to the server, not to a swarm.
    Ok((
        DirectAccess::new(db, kkt_api_agent::SwarmId::main(), parts.clock.clone()),
        max_lane_chars,
    ))
}

/// The operator of a command run against the database file: every
/// permission, since whoever can open the file may do anything to it
/// already, recorded as the command line.
pub(crate) fn command_line_operator() -> kkt_api_agent::Operator {
    kkt_api_agent::Operator::new(
        kkt_api_agent::Permissions::of(&[
            kkt_api_agent::Permission::Read,
            kkt_api_agent::Permission::Change,
            kkt_api_agent::Permission::IssueRecoveryCodes,
        ]),
        kkt_api_agent::Audit::CommandLine,
    )
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
) -> Result<String, LaneError> {
    let (access, max_lane_chars) = local_operator(parts, db, config)?;
    let skipped_files = read.skipped_names();
    let source = source.unwrap_or_else(|| LaneSource {
        folder: read.folder.display().to_string(),
        commit: read.commit,
    });
    let lanes = lane_saves(read.lanes);
    let done = access
        .import_lanes(
            &command_line_operator(),
            kkt_rules::LaneImport {
                lanes,
                source: source.clone(),
                retire_missing,
                skipped_files,
                max_lane_chars,
            },
        )
        .await?;
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
) -> Result<lane_files::Read, LaneError> {
    let read = lane_files::read_folder(disk, commits, folder)?;
    warn_skipped(output, &read.skipped);
    if retire_missing {
        kkt_rules::check_retire_missing(
            &read.folder.display().to_string(),
            read.lanes.len(),
            read.skipped.len(),
        )
        .map_err(LaneError::RetireAll)?;
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
pub fn local_lane_limit(parts: &LaneParts<'_>) -> Result<usize, LaneError> {
    Ok(parts
        .serve
        .config(None)
        .map_err(LaneError::Config)?
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
) -> Result<usize, LaneError> {
    let read = lane_files::read_folder(disk, commits, folder)?;
    warn_skipped(output, &read.skipped);
    let count = read.lanes.len();
    kkt_rules::check_lane_import(&lane_saves(read.lanes), max_lane_chars).map_err(|source| {
        LaneError::Check {
            folder: read.folder.clone(),
            source,
        }
    })?;
    Ok(count)
}

/// Reads the lane files in `folder` and checks them as an import into
/// `server`, the server `kkt setup` sets up, would, changing nothing:
/// against that server's lane limit. Returns how many lanes there are.
pub fn check_lanes_for_setup(
    parts: &LaneParts<'_>,
    server: &dyn LaneServer,
    folder: &Path,
) -> Result<usize, LaneError> {
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
) -> Result<String, LaneError> {
    let read = lane_files::read_folder(parts.lanes, parts.lane_commit, folder)?;
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

    fn export() -> LanesAction {
        LanesAction::Export {
            folder: "out".into(),
        }
    }

    const SERVICE: Deployment = Deployment::Service { tailscale: false };

    #[test]
    fn lanes_act_on_docker_in_its_folder_without_local_options() {
        assert_eq!(
            lanes_on(Some(&docker()), false, &export()),
            Ok(LanesOn::Docker(PathBuf::from("/data/docker")))
        );
        assert_eq!(
            lanes_on(Some(&docker()), true, &export()).map_err(|e| e.to_string()),
            Err(
                "kkt in Docker keeps its lanes in the container; --db and --config apply only \
                 to a local server"
                    .to_string()
            )
        );
    }

    #[test]
    fn lanes_act_on_the_local_database_for_a_service_or_no_record() {
        for local_options in [false, true] {
            assert_eq!(
                lanes_on(Some(&SERVICE), local_options, &export()),
                Ok(LanesOn::Local)
            );
            assert_eq!(lanes_on(None, local_options, &export()), Ok(LanesOn::Local));
        }
    }

    #[test]
    fn lanes_are_refused_on_a_client() {
        assert_eq!(
            lanes_on(Some(&Deployment::Client), false, &export()).map_err(|e| e.to_string()),
            Err(
                "this machine is a client; run `kkt lanes` on the machine where the server runs"
                    .to_string()
            )
        );
    }

    /// Lane files in memory, under `/work`.
    struct Files(Vec<(&'static str, &'static str)>);

    impl lane_files::Disk for Files {
        fn file_names(&self, folder: &Path) -> std::io::Result<Vec<String>> {
            Ok(self
                .0
                .iter()
                .map(|(path, _)| Path::new(path))
                .filter(|path| path.parent() == Some(folder))
                .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
                .collect())
        }
        fn read(&self, path: &Path) -> std::io::Result<String> {
            self.0
                .iter()
                .find(|(at, _)| Path::new(at) == path)
                .map(|(_, text)| text.to_string())
                .ok_or_else(|| std::io::Error::other("no such file"))
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            unreachable!("nothing is written")
        }
        fn write(&self, _: &Path, _: &str) -> std::io::Result<()> {
            unreachable!("nothing is written")
        }
        fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
            Ok(Path::new("/work").join(path))
        }
    }

    struct Quiet;
    impl crate::Output for Quiet {
        fn output(&self, _: std::fmt::Arguments<'_>) {}
        fn errors(&self, _: std::fmt::Arguments<'_>) {}
    }

    /// The two refusals that print a lane rule's message alone, word for
    /// word: retiring every lane, and a lane the rules refuse, named by its
    /// folder.
    #[test]
    fn the_lane_rules_refusals_are_pinned() {
        let commits = kkt_lane_files::testing::Memory::new();
        let notes = Files(vec![("/work/lanes/notes.md", "# Notes")]);
        assert_eq!(
            read_lanes_warning(&notes, &commits, &Quiet, Path::new("lanes"), true)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(
                "/work/lanes has no lane files, only skipped .md files, so --retire-missing would \
                 retire every lane; name the folder that holds the lanes"
                    .to_string()
            )
        );
        let long = Files(vec![(
            "/work/lanes/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews.\n---\n\nRead every line carefully.\n",
        )]);
        assert_eq!(
            check_lanes_folder(&long, &commits, &Quiet, Path::new("lanes"), 5)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(
                "/work/lanes: lane reviewer: the lane's instructions must be at most 5 characters"
                    .to_string()
            )
        );
    }
}
