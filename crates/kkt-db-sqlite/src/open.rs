//! Opening a database: the connections, reads and writes over them,
//! and migrations.

#[cfg(any(test, feature = "real"))]
use crate::Options;
use crate::adopt::adopt;
use crate::conn::{SqliteConnection, db_error, index_words};
use crate::{MANAGED, ddl, sql};
use kkt_db::Legacy;
use kkt_db::{Backend, DbError, Done, Evolve, Schema, Set, Step, Upgrade, Work};
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

/// An open database: what `open` gives as a kkt-db [`Backend`]. Its work
/// runs on tokio's blocking pool, since SQLite blocks.
struct Store(Arc<Connections>);

struct Connections {
    writer: Arc<Mutex<Connection>>,
    /// Empty for an in-memory database: reads use the writer.
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
    schema: Schema,
}

impl Connections {
    fn read(&self, work: Work) -> Result<(), DbError> {
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

    fn write(&self, work: Work) -> Result<(), DbError> {
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

/// Runs `job` on tokio's blocking pool.
async fn blocking(
    job: impl FnOnce() -> Result<(), DbError> + Send + 'static,
) -> Result<(), DbError> {
    match tokio::task::spawn_blocking(job).await {
        Ok(done) => done,
        Err(join) => Err(DbError::Task(join.to_string())),
    }
}

impl Backend for Store {
    fn read(&self, work: Work) -> Done<'_> {
        let connections = self.0.clone();
        Box::pin(blocking(move || connections.read(work)))
    }

    fn write(&self, work: Work) -> Done<'_> {
        let connections = self.0.clone();
        Box::pin(blocking(move || connections.write(work)))
    }
}

fn configure(conn: &Connection, busy_timeout: Duration) -> Result<(), DbError> {
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(failed)?;
    conn.busy_timeout(busy_timeout).map_err(failed)
}

/// What a file holds, as far as opening it goes.
pub(crate) enum Found {
    /// Nothing, or kkt-db's tables: migrations take it from here.
    Ready,
    /// A database from before kkt-db, to adopt as this version of the
    /// legacy set.
    Adopt(u32),
}

/// What the file on `conn` holds: refuses a database from before kkt-db
/// that `legacy` does not adopt, and a file with tables kkt did not make.
pub(crate) fn found(conn: &Connection, legacy: Option<&Legacy>) -> Result<Found, DbError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(failed)?;
    if version == MANAGED {
        return Ok(Found::Ready);
    }
    if version == 0 {
        // A new file has no tables; one with tables but no record of
        // versions was not made by kkt.
        let tables: bool = conn
            .query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master)", [], |r| {
                r.get(0)
            })
            .map_err(failed)?;
        if !tables {
            return Ok(Found::Ready);
        }
        return Err(DbError::Backend(
            "this file holds tables kkt did not make; kkt will not open it".into(),
        ));
    }
    let Some(legacy) = legacy else {
        return Err(DbError::Backend(format!(
            "this database's tables are at kkt schema {version}, from before kkt-db; this version cannot open it"
        )));
    };
    if let Some((_, adopted)) = legacy.adopted.iter().find(|(v, _)| *v == version) {
        return Ok(Found::Adopt(*adopted));
    }
    let oldest = legacy.adopted.iter().map(|(v, _)| *v).min().unwrap_or(0);
    if version < oldest {
        return Err(DbError::Backend(legacy.too_old.clone()));
    }
    Err(DbError::Backend(format!(
        "this database's tables are at kkt schema {version}, which this version does not know"
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
    let mut schema = before.clone();
    for step in steps {
        make(&tx, step)?;
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
    record(&tx, set, version)?;
    check_references(&tx)?;
    tx.commit().map_err(failed)
}

/// Makes what a step declares: its tables, columns, indexes, triggers or
/// search index. Fills and backfills are the caller's.
pub(crate) fn make(conn: &Connection, step: &Step) -> Result<(), DbError> {
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
        conn.execute(&statement, [])
            .map_err(|e| db_error(e, table))?;
    }
    Ok(())
}

/// Records `version` of `set` in `kkt_db_versions`, and marks the file as
/// recording its versions there.
pub(crate) fn record(conn: &Connection, set: &str, version: u32) -> Result<(), DbError> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {VERSIONS} (name TEXT PRIMARY KEY NOT NULL, version INTEGER NOT NULL)"
    ))
    .map_err(failed)?;
    conn.execute(
        &format!(
            "INSERT INTO {VERSIONS} (name, version) VALUES (?1, ?2)
             ON CONFLICT (name) DO UPDATE SET version = excluded.version"
        ),
        rusqlite::params![set, version],
    )
    .map_err(failed)?;
    conn.pragma_update(None, "user_version", MANAGED)
        .map_err(failed)
}

/// A reference that names no row: the table and rowid of the row that
/// holds it, and the table it names.
pub(crate) fn dangling(
    conn: &Connection,
) -> Result<Option<(String, Option<i64>, String)>, DbError> {
    let mut statement = conn
        .prepare(r#"SELECT "table", rowid, parent FROM pragma_foreign_key_check"#)
        .map_err(failed)?;
    let mut rows = statement.query([]).map_err(failed)?;
    match rows.next().map_err(failed)? {
        Some(r) => Ok(Some((
            r.get(0).map_err(failed)?,
            r.get(1).map_err(failed)?,
            r.get(2).map_err(failed)?,
        ))),
        None => Ok(None),
    }
}

/// Fails if any reference names no row, naming the table that holds it.
pub(crate) fn check_references(conn: &Connection) -> Result<(), DbError> {
    if let Some((table, _, _)) = dangling(conn)? {
        return Err(DbError::Constraint {
            table,
            column: None,
            kind: kkt_db::ConstraintKind::Reference,
        });
    }
    Ok(())
}

/// Brings the database on `writer` up to date with `sets` and opens it.
fn open_with(
    writer: Arc<Mutex<Connection>>,
    sets: &[Set],
    legacy: Option<&Legacy>,
    file: bool,
    readers: impl FnOnce() -> Result<Vec<Mutex<Connection>>, DbError>,
) -> Result<Arc<dyn Backend>, DbError> {
    let found = found(&*lock(&writer)?, legacy)?;
    if let (Found::Adopt(version), Some(legacy)) = (found, legacy) {
        let Some(set) = sets.iter().find(|s| s.name == legacy.set) else {
            return Err(DbError::Schema(format!(
                "no set {} to adopt the database as",
                legacy.set
            )));
        };
        adopt(&mut *lock(&writer)?, set, version, legacy)?;
    }
    if file {
        lock(&writer)?
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(failed)?;
    }
    Evolver { conn: &writer }.migrate(sets)?;
    let schema = Schema::latest(sets)?;
    Ok(Arc::new(Store(Arc::new(Connections {
        writer,
        readers: readers()?,
        next_reader: AtomicUsize::new(0),
        schema,
    }))))
}

/// A database file.
#[cfg(any(test, feature = "real"))]
pub struct SqlitePlace {
    pub(crate) path: std::path::PathBuf,
    options: Options,
    legacy: Option<Legacy>,
}

#[cfg(any(test, feature = "real"))]
impl SqlitePlace {
    pub fn new(path: &std::path::Path, options: Options) -> SqlitePlace {
        SqlitePlace {
            path: path.to_path_buf(),
            options,
            legacy: None,
        }
    }

    /// Adopts a database from before kkt-db as `legacy` says, instead of
    /// refusing it.
    pub fn adopting(mut self, legacy: Legacy) -> SqlitePlace {
        self.legacy = Some(legacy);
        self
    }

    /// The `user_version` of a database from before kkt-db that opening
    /// the file would adopt; `None` for any other file. Changes nothing.
    pub fn adoption(&self) -> Result<Option<i64>, DbError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = read_only(&self.path)?;
        match found(&conn, self.legacy.as_ref())? {
            Found::Adopt(_) => conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .map(Some)
                .map_err(failed),
            Found::Ready => Ok(None),
        }
    }

    /// Opens the file (creating it if needed) and brings its tables up to
    /// date with `sets`.
    pub fn open(&self, sets: &[Set]) -> Result<Arc<dyn Backend>, DbError> {
        let writer = Connection::open(&self.path).map_err(failed)?;
        configure(&writer, self.options.busy_timeout)?;
        // Nothing is written before the file is known to be one kkt opens,
        // or has been adopted: a refused file is left exactly as it was.
        // Only then does it go over to WAL.
        let (path, options) = (self.path.clone(), self.options);
        open_with(
            Arc::new(Mutex::new(writer)),
            sets,
            self.legacy.as_ref(),
            true,
            move || {
                (0..options.readers)
                    .map(|_| {
                        let reader = read_only(&path)?;
                        configure(&reader, options.busy_timeout)?;
                        Ok(Mutex::new(reader))
                    })
                    .collect()
            },
        )
    }

    /// The upgrades opening the file with `sets` would make. Changes
    /// nothing; a file that is not there would be made from nothing.
    pub fn pending(&self, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
        if !self.path.exists() {
            return Empty.pending(sets);
        }
        let conn = read_only(&self.path)?;
        if let Found::Adopt(version) = found(&conn, self.legacy.as_ref())? {
            // After adopting, the legacy set is at that version and the
            // others are new.
            return Ok(sets
                .iter()
                .map(|set| {
                    let from = match &self.legacy {
                        Some(l) if l.set == set.name => version,
                        _ => 0,
                    };
                    Upgrade {
                        set: set.name,
                        from,
                        to: set.latest(),
                    }
                })
                .filter(|u| u.from < u.to)
                .collect());
        }
        Evolver {
            conn: &Mutex::new(conn),
        }
        .pending(sets)
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

    pub fn open(&self, sets: &[Set]) -> Result<Arc<dyn Backend>, DbError> {
        open_with(self.conn.clone(), sets, None, false, || Ok(Vec::new()))
    }

    pub fn pending(&self, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
        Evolver { conn: &self.conn }.pending(sets)
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
