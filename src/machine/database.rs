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

/// Writes a consistent copy of the database file at `from` to the new file
/// `to`, even while a server writes to `from`. A failure is the database's
/// own error, for the backup to give as its cause.
pub fn copy(from: &Path, to: &Path) -> Result<(), kkt_backup_os::CopyError> {
    Ok(copy_database(from, to, Options::DEFAULT.busy_timeout)?)
}
