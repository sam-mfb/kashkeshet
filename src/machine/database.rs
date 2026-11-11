//! The database file: opening it with the core's tables, what opening it
//! would upgrade, and copying it. The file is SQLite's (`kkt-db-sqlite`);
//! the store sees only the database it opens.

use kkt_db::Upgrade;
use kkt_db_sqlite::{Legacy, Options, SqlitePlace, copy_database};
use kkt_store::Db;
use std::path::Path;

fn place(path: &Path) -> SqlitePlace {
    SqlitePlace::new(path, Options::DEFAULT).adopting(legacy())
}

/// The databases kkt made before kkt-db, which counted their schema in
/// SQLite's `user_version`: 0.10.0's (21) is the core's version 1, and
/// `next`'s with swarms (22) its version 2. The two handoff-note columns,
/// whose notes migration 0019 copied into memory entries, are left behind. Older
/// databases upgrade through 0.10 first.
fn legacy() -> Legacy {
    Legacy {
        set: kkt_store::CORE,
        adopted: vec![(21, 1), (22, 2)],
        dropped: vec![("agents", "handoff_note"), ("agents", "handoff_note_set_at")],
        too_old: "this database is from a kkt version older than 0.10.0; upgrade it with kkt 0.10 first, then with this version".into(),
    }
}

/// An upgrade opening the file would make, for the log and the backup
/// before it: from what, to what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingUpgrade {
    pub from: String,
    pub to: String,
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
/// there, adopting a database from before kkt-db included. `None` when
/// there is no file, it has no tables yet, or they are up to date.
/// Changes nothing.
pub fn pending_upgrade(path: &Path) -> Result<Option<PendingUpgrade>, String> {
    let place = place(path);
    let unreadable = |e| format!("could not read {}: {e}", path.display());
    let upgrades = place.pending(&[kkt_store::schema()]).map_err(unreadable)?;
    let core = |u: &Upgrade| format!("{} {}", u.set, u.to);
    if let Some(old) = place.adoption().map_err(unreadable)? {
        let to = match upgrades.first() {
            Some(u) => core(u),
            None => format!("{} {}", kkt_store::CORE, kkt_store::schema().latest()),
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
/// `to`, even while a server writes to `from`.
pub fn copy(from: &Path, to: &Path) -> Result<(), String> {
    copy_database(from, to, Options::DEFAULT.busy_timeout).map_err(|e| e.to_string())
}
