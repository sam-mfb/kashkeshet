//! Opening a database: the connections, reads and writes over them,
//! and migrations.

#[cfg(any(test, feature = "real"))]
use crate::Options;
use crate::conn::{SqliteConnection, db_error, index_words};
use crate::{MANAGED, ddl, sql};
use kkt_db::{
    Backend, Database, DbError, Evolve, Schema, Set, Step, Upgrade, Work, migrate, pending,
};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Where versions are recorded: one row per set.
const VERSIONS: &str = "kkt_db_versions";

fn poisoned<T>(_: T) -> DbError {
    DbError::Backend("a database connection's lock is poisoned".into())
}

fn lock(conn: &Mutex<Connection>) -> Result<MutexGuard<'_, Connection>, DbError> {
    conn.lock().map_err(poisoned)
}

fn failed(e: rusqlite::Error) -> DbError {
    DbError::Backend(e.to_string())
}

struct Store {
    writer: Arc<Mutex<Connection>>,
    /// Empty for an in-memory database: reads use the writer.
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
    schema: Schema,
}

impl Backend for Store {
    fn read(&self, work: Work<'_>) -> Result<(), DbError> {
        let connection = if self.readers.is_empty() {
            &*self.writer
        } else {
            let i = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
            &self.readers[i]
        };
        let mut conn = lock(connection)?;
        // A deferred transaction takes its snapshot at its first read and
        // keeps it to the end: one state. Dropping it ends it.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(failed)?;
        work(&SqliteConnection {
            conn: &tx,
            schema: &self.schema,
            write: false,
        })
    }

    fn write(&self, work: Work<'_>) -> Result<(), DbError> {
        let mut conn = lock(&self.writer)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(failed)?;
        work(&SqliteConnection {
            conn: &tx,
            schema: &self.schema,
            write: true,
        })?;
        tx.commit().map_err(failed)
    }
}

fn configure(conn: &Connection, busy_timeout: Duration) -> Result<(), DbError> {
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(failed)?;
    conn.busy_timeout(busy_timeout).map_err(failed)
}

/// Refuses a database kkt made before kkt-db, which records its schema
/// in `user_version` alone.
fn check_managed(conn: &Connection) -> Result<(), DbError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(failed)?;
    if version == 0 || version == MANAGED {
        return Ok(());
    }
    Err(DbError::Backend(format!(
        "this database's tables are at kkt schema {version}, from before kkt-db; this version cannot open it"
    )))
}

fn versions_table(conn: &Connection) -> Result<bool, DbError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [VERSIONS],
        |r| r.get(0),
    )
    .map_err(failed)
}

/// Migrations over one connection.
struct Evolver<'a> {
    conn: &'a Mutex<Connection>,
}

impl Evolve for Evolver<'_> {
    fn version(&self, set: &'static str) -> Result<u32, DbError> {
        let conn = lock(self.conn)?;
        if !versions_table(&conn)? {
            return Ok(0);
        }
        let version: Option<i64> = conn
            .query_row(
                &format!("SELECT version FROM {VERSIONS} WHERE name = ?1"),
                [set],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
            .map_err(failed)?;
        Ok(version.map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX)))
    }

    fn apply(
        &self,
        set: &'static str,
        version: u32,
        before: &Schema,
        _after: &Schema,
        steps: &[Step],
    ) -> Result<(), DbError> {
        let mut conn = lock(self.conn)?;
        // A step may add a column that references another table with a
        // default (SQLite allows that only with foreign keys off); every
        // reference is checked before the migration commits instead.
        conn.pragma_update(None, "foreign_keys", "OFF")
            .map_err(failed)?;
        let result = apply(&mut conn, set, version, before, steps);
        let restored = conn
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(failed);
        result.and(restored)
    }
}

fn apply(
    conn: &mut Connection,
    set: &'static str,
    version: u32,
    before: &Schema,
    steps: &[Step],
) -> Result<(), DbError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failed)?;
    tx.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {VERSIONS} (name TEXT PRIMARY KEY NOT NULL, version INTEGER NOT NULL)"
    ))
    .map_err(failed)?;
    let mut schema = before.clone();
    for step in steps {
        let (table, statements) = match step {
            Step::AddTable(t) => (t.name, ddl::add_table(t)),
            Step::AddColumn { table, column } => (*table, ddl::add_column(table, column)),
            Step::AddIndex { table, index } => (
                *table,
                vec![ddl::add_index(table, index.name, &index.columns)],
            ),
            Step::AddConstraint { table, constraint } => {
                (*table, ddl::add_constraint(table, constraint))
            }
            Step::AddSearch { table, .. } => (*table, vec![ddl::add_search(table)]),
            Step::Fill { .. } => ("", Vec::new()),
        };
        for statement in statements {
            tx.execute(&statement, []).map_err(|e| db_error(e, table))?;
        }
        step.apply_to(&mut schema)?;
        match step {
            Step::AddConstraint { table, constraint } => {
                if let Some(query) = ddl::breaking(table, constraint) {
                    let broken: bool = tx
                        .prepare(&query)
                        .and_then(|mut s| s.exists([]))
                        .map_err(failed)?;
                    if broken {
                        return Err(DbError::Constraint {
                            table: table.to_string(),
                            column: None,
                            kind: kkt_db::ConstraintKind::Positive,
                        });
                    }
                }
            }
            Step::AddSearch { table, column } => {
                let rows: Vec<(i64, Option<String>)> = tx
                    .prepare(&format!(
                        "SELECT rowid, {} FROM {}",
                        sql::q(column),
                        sql::q(table)
                    ))
                    .and_then(|mut s| {
                        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .map_err(failed)?;
                for (rowid, text) in rows {
                    if let Some(text) = text {
                        index_words(&tx, table, rowid, &text).map_err(failed)?;
                    }
                }
            }
            Step::Fill { run, .. } => run(&SqliteConnection {
                conn: &tx,
                schema: &schema,
                write: true,
            })?,
            Step::AddTable(_) | Step::AddColumn { .. } | Step::AddIndex { .. } => {}
        }
    }
    tx.execute(
        &format!(
            "INSERT INTO {VERSIONS} (name, version) VALUES (?1, ?2)
             ON CONFLICT (name) DO UPDATE SET version = excluded.version"
        ),
        rusqlite::params![set, version],
    )
    .map_err(failed)?;
    tx.pragma_update(None, "user_version", MANAGED)
        .map_err(failed)?;
    let dangling = tx
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut s| s.exists([]))
        .map_err(failed)?;
    if dangling {
        return Err(DbError::Constraint {
            table: set.to_string(),
            column: None,
            kind: kkt_db::ConstraintKind::Reference,
        });
    }
    tx.commit().map_err(failed)
}

/// Brings the database on `writer` up to date with `sets` and opens it.
fn open_with(
    writer: Arc<Mutex<Connection>>,
    sets: &[Set],
    readers: impl FnOnce() -> Result<Vec<Mutex<Connection>>, DbError>,
) -> Result<Database, DbError> {
    check_managed(&*lock(&writer)?)?;
    migrate(&Evolver { conn: &writer }, sets)?;
    let schema = kkt_db::schema(sets)?;
    Ok(Database::new(Store {
        writer,
        readers: readers()?,
        next_reader: AtomicUsize::new(0),
        schema,
    }))
}

/// A database file.
#[cfg(any(test, feature = "real"))]
pub struct SqlitePlace {
    pub(crate) path: std::path::PathBuf,
    options: Options,
}

#[cfg(any(test, feature = "real"))]
impl SqlitePlace {
    pub fn new(path: &std::path::Path, options: Options) -> SqlitePlace {
        SqlitePlace {
            path: path.to_path_buf(),
            options,
        }
    }

    /// Opens the file (creating it if needed) and brings its tables up to
    /// date with `sets`.
    pub fn open(&self, sets: &[Set]) -> Result<Database, DbError> {
        let writer = Connection::open(&self.path).map_err(failed)?;
        configure(&writer, self.options.busy_timeout)?;
        writer
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(failed)?;
        let (path, options) = (self.path.clone(), self.options);
        open_with(Arc::new(Mutex::new(writer)), sets, move || {
            (0..options.readers)
                .map(|_| {
                    let reader = read_only(&path)?;
                    configure(&reader, options.busy_timeout)?;
                    Ok(Mutex::new(reader))
                })
                .collect()
        })
    }

    /// The upgrades opening the file with `sets` would make. Changes
    /// nothing; a file that is not there would be made from nothing.
    pub fn pending(&self, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
        if !self.path.exists() {
            return pending(&Empty, sets);
        }
        let conn = read_only(&self.path)?;
        check_managed(&conn)?;
        pending(
            &Evolver {
                conn: &Mutex::new(conn),
            },
            sets,
        )
    }
}

#[cfg(any(test, feature = "real"))]
fn read_only(path: &std::path::Path) -> Result<Connection, DbError> {
    use rusqlite::OpenFlags;
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(failed)
}

/// A database with no tables: every set at version 0.
#[cfg(any(test, feature = "real"))]
struct Empty;

#[cfg(any(test, feature = "real"))]
impl Evolve for Empty {
    fn version(&self, _: &'static str) -> Result<u32, DbError> {
        Ok(0)
    }

    fn apply(
        &self,
        _: &'static str,
        _: u32,
        _: &Schema,
        _: &Schema,
        _: &[Step],
    ) -> Result<(), DbError> {
        Err(DbError::Backend(
            "an empty database cannot be migrated".into(),
        ))
    }
}

/// A SQLite database held in memory, with one connection. Opening the same
/// place again finds what was written there.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone)]
pub struct MemoryPlace {
    conn: Arc<Mutex<Connection>>,
}

#[cfg(any(test, feature = "testing"))]
impl MemoryPlace {
    pub fn new() -> Result<MemoryPlace, DbError> {
        let conn = Connection::open_in_memory().map_err(failed)?;
        configure(&conn, Duration::ZERO)?;
        Ok(MemoryPlace {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open(&self, sets: &[Set]) -> Result<Database, DbError> {
        open_with(self.conn.clone(), sets, || Ok(Vec::new()))
    }

    pub fn pending(&self, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
        pending(&Evolver { conn: &self.conn }, sets)
    }
}

/// Writes a copy of the database file at `from` to the new file `to` with
/// SQLite's online backup, so the copy is consistent even while a server
/// writes to `from`. The copy is one file (no write-ahead log beside it),
/// and it passes SQLite's integrity check before this returns.
#[cfg(any(test, feature = "real"))]
pub fn copy_database(
    from: &std::path::Path,
    to: &std::path::Path,
    busy_timeout: Duration,
) -> Result<(), DbError> {
    use rusqlite::backup::{Backup, StepResult};
    if to.exists() {
        return Err(DbError::Backend(format!("{} already exists", to.display())));
    }
    let source = read_only(from)?;
    configure(&source, busy_timeout)?;
    let mut copy = Connection::open(to).map_err(failed)?;
    configure(&copy, busy_timeout)?;
    {
        let backup = Backup::new(&source, &mut copy).map_err(failed)?;
        // All pages in one step (-1): one read transaction, so the copy is
        // the database at one moment. In WAL mode it does not block writers.
        // Both connections wait `busy_timeout` for locks, so busy is final.
        loop {
            match backup.step(-1).map_err(failed)? {
                StepResult::Done => break,
                StepResult::More => continue,
                waiting => {
                    return Err(DbError::Backend(format!(
                        "the database stayed {waiting:?}; try again"
                    )));
                }
            }
        }
    }
    copy.pragma_update(None, "journal_mode", "DELETE")
        .map_err(failed)?;
    let check: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(failed)?;
    if check != "ok" {
        return Err(DbError::Backend(format!(
            "the copy failed SQLite's integrity check: {check}"
        )));
    }
    Ok(())
}
