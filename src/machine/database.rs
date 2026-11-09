//! The database file: opening it with the core's tables, what opening it
//! would upgrade, and copying it. The file is SQLite's (`kkt-db-sqlite`);
//! the store sees only the database it opens.

use kkt_config::{DB_BUSY_TIMEOUT, DB_READERS};
use kkt_db::Upgrade;
use kkt_db_sqlite::{Options, SqlitePlace, copy_database};
use kkt_store::Db;
use std::path::Path;

fn place(path: &Path) -> SqlitePlace {
    SqlitePlace::new(
        path,
        Options {
            readers: DB_READERS,
            busy_timeout: DB_BUSY_TIMEOUT,
        },
    )
}

/// Opens the database file at `path`, creating it if needed, and brings its
/// tables up to date.
pub fn open(path: &Path) -> Result<Db, String> {
    place(path)
        .open(&[kkt_store::schema()])
        .map(Db::new)
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

/// The upgrade opening the file at `path` would make to tables already
/// there. `None` when there is no file, it has no tables yet, or they are
/// up to date. Changes nothing.
pub fn pending_upgrade(path: &Path) -> Result<Option<Upgrade>, String> {
    let upgrades = place(path)
        .pending(&[kkt_store::schema()])
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    Ok(upgrades.into_iter().find(|u| u.from > 0))
}

/// Writes a consistent copy of the database file at `from` to the new file
/// `to`, even while a server writes to `from`.
pub fn copy(from: &Path, to: &Path) -> Result<(), String> {
    copy_database(from, to, DB_BUSY_TIMEOUT).map_err(|e| e.to_string())
}
