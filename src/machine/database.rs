//! The database file: opening it with the core's tables, what opening it
//! would upgrade, and copying it. The file is SQLite's (`kkt-db-sqlite`);
//! the store sees only the database it opens.

use kkt_db::{DbError, Upgrade};
use kkt_db_sqlite::{Options, SqlitePlace, copy_database};
use kkt_rules::Db;
use std::path::{Path, PathBuf};

/// Why the database file could not be opened.
#[derive(Debug, thiserror::Error)]
#[error("could not open {}: {source}", path.display())]
pub struct OpenError {
    pub path: PathBuf,
    #[source]
    pub source: DbError,
}

/// Why the upgrade opening the database file would make could not be read.
#[derive(Debug, thiserror::Error)]
#[error("could not read {}: {source}", path.display())]
pub struct PendingError {
    pub path: PathBuf,
    #[source]
    pub source: DbError,
}

fn place(path: &Path) -> SqlitePlace {
    SqlitePlace::new(path, Options::DEFAULT).adopting(kkt_rules::legacy(TOO_OLD.into()))
}

/// What a database older than kkt-rules' history adopts is told: older
/// databases upgrade through 0.10 first.
const TOO_OLD: &str = "this database is from a kkt version older than 0.10.0; upgrade it with kkt 0.10 first, then with this version";

/// An upgrade opening the file would make, for the log and the backup
/// before it: from what, to what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingUpgrade {
    pub from: String,
    pub to: String,
}

/// Opens the database file at `path`, creating it if needed, and brings its
/// tables up to date.
pub fn open(path: &Path) -> Result<Db, OpenError> {
    place(path)
        .open(&[kkt_rules::schema()])
        .map(Db::new)
        .map_err(|source| OpenError {
            path: path.to_path_buf(),
            source,
        })
}

/// The upgrade opening the file at `path` would make to tables already
/// there, adopting a database from before kkt-db included. `None` when
/// there is no file, it has no tables yet, or they are up to date.
/// Changes nothing.
pub fn pending_upgrade(path: &Path) -> Result<Option<PendingUpgrade>, PendingError> {
    let place = place(path);
    let unreadable = |source| PendingError {
        path: path.to_path_buf(),
        source,
    };
    let upgrades = place.pending(&[kkt_rules::schema()]).map_err(unreadable)?;
    let core = |u: &Upgrade| format!("{} {}", u.set, u.to);
    if let Some(old) = place.adoption().map_err(unreadable)? {
        let to = match upgrades.first() {
            Some(u) => core(u),
            None => format!("{} {}", kkt_rules::CORE, kkt_rules::schema().latest()),
        };
        return Ok(Some(PendingUpgrade {
            from: old.to_string(),
            to,
        }));
    }
    Ok(upgrades
        .into_iter()
        .find(|u| u.from > 0)
        .map(|u| PendingUpgrade {
            from: format!("{} {}", u.set, u.from),
            to: core(&u),
        }))
}

/// The highest message ID in the database at `path`, for a backup's
/// about file. Reads only the finished copy, even if its schema is older.
/// The query needs the messages table and its ID column. It never migrates
/// the file, creates journal files or waits for a writer.
/// `Ok(None)` when there are no messages. The read runs on this
/// runtime's blocking pool, as the database's own reads do, so it is
/// called from inside the runtime, on the finished copy a backup made.
pub fn highest_message_id(path: &Path) -> Result<Option<i64>, HighestError> {
    let backend = place(path)
        .open_snapshot(&[kkt_rules::schema()])
        .map_err(|source| HighestError::Open {
            path: path.to_path_buf(),
            source,
        })?;
    let db = kkt_rules::Db::new(backend);
    let access = kkt_rules::DirectAccess::new(
        db,
        kkt_api_agent::SwarmId::main(),
        std::sync::Arc::new(kkt_clock_system::SystemClock),
    );
    let operator = kkt_api_agent::Operator::new(
        kkt_api_agent::Permissions::of(&[kkt_api_agent::Permission::Read]),
        kkt_api_agent::Audit::CommandLine,
    );
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(access.highest_message_id(&operator))
    })
    .map_err(|source| HighestError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// Why the highest message ID in a database could not be read.
#[derive(Debug, thiserror::Error)]
pub enum HighestError {
    /// The database could not be opened read-only.
    #[error("could not open {} read-only: {source}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: DbError,
    },
    /// The highest ID could not be read from the opened database.
    #[error("could not read the highest message ID in {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: kkt_api_agent::RuleError,
    },
}

/// Writes a consistent copy of the database file at `from` to the new file
/// `to`, even while a server writes to `from`. A failure is the database's
/// own error, for the backup to give as its cause.
pub fn copy(from: &Path, to: &Path) -> Result<(), kkt_backup_os::CopyError> {
    Ok(copy_database(from, to, Options::DEFAULT.busy_timeout)?)
}
