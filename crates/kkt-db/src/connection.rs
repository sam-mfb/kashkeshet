//! What a backend implements: [`Connection`], the operations on one
//! connection, and [`Backend`], which runs a closure over one inside a
//! read or a write; and [`BackendExt`], the typed read and write a store
//! calls on any backend.

use crate::search::{Hit, Search};
use crate::{Count, DbError, Delete, Group, Insert, Row, Select, Update};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// The questions and changes, on one connection. Synchronous: a
/// [`Backend`] runs the closure that uses it where blocking is fine.
///
/// Inside [`BackendExt::write`], every change is part of one transaction.
/// Inside [`BackendExt::read`], a change fails with [`DbError::ReadOnly`].
pub trait Connection {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError>;

    fn count(&self, count: &Count) -> Result<i64, DbError>;

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError>;

    /// The rows that match every term, as [`crate::search`] defines words
    /// and matching, with their excerpts.
    fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError>;

    /// Adds the row. Returns its increasing key if the table has one.
    fn insert(&self, insert: &Insert) -> Result<Option<i64>, DbError>;

    /// Adds the row unless a row with its primary key exists, in which case
    /// it changes nothing. Returns whether it added the row.
    fn insert_unless_present(&self, insert: &Insert) -> Result<bool, DbError>;

    /// Adds the row, or, if a row with its primary key exists, sets that
    /// row's other columns to the values given.
    fn save(&self, insert: &Insert) -> Result<(), DbError>;

    /// Returns how many rows met the update's filter.
    fn update(&self, update: &Update) -> Result<u64, DbError>;

    /// Returns how many rows it removed.
    fn delete(&self, delete: &Delete) -> Result<u64, DbError>;

    /// The first row [`Connection::select`] gives, if any.
    fn first(&self, select: &Select) -> Result<Option<Row>, DbError> {
        let select = select.clone().limit(1);
        Ok(self.select(&select)?.into_iter().next())
    }
}

/// The closure a [`Backend`] runs, once: it gets a connection, and its
/// error rolls a write back.
pub type Work = Box<dyn FnOnce(&dyn Connection) -> Result<(), DbError> + Send>;

/// What a [`Backend`]'s read or write gives back: a future of whether the
/// work ran and, for a write, committed.
pub type Done<'a> = Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>>;

/// A database a store can run work on, through [`BackendExt`]. `read` and
/// `write` are async: a backend that blocks, as SQLite does, runs the work
/// on a thread pool of its own.
///
/// **Reads** see one consistent state: no write commits partly into a
/// read's view, and a read of several steps sees the same database
/// throughout.
///
/// **Writes** are transactions that run as if alone: no other write
/// commits between a write's reads and its changes. A store relies on
/// this to read something, such as whether a name is taken, and then
/// change the database on that basis. Because a write's work runs only
/// once, a backend cannot get this by retrying a transaction that failed
/// to serialize: it holds a lock that lets one write run at a time, as
/// SQLite's one writer connection does, or an equivalent such as a
/// Postgres advisory lock taken at the start of each write.
pub trait Backend: Send + Sync + 'static {
    /// Runs `work` once over a connection that cannot change anything.
    fn read(&self, work: Work) -> Done<'_>;

    /// Runs `work` once inside a transaction, and commits only if it
    /// returns `Ok`. On `Err`, nothing it changed is kept, and the error
    /// is returned.
    fn write(&self, work: Work) -> Done<'_>;
}

/// The typed read and write a store calls, written once in terms of
/// [`Backend`]'s async read and write: the caller's own result and error
/// types, the closure's error rolling a write back, and the guard that the
/// work ran. Every [`Backend`] has them, `dyn Backend` included.
///
/// Its methods share their names with [`Backend`]'s, so a caller names the
/// trait: `BackendExt::read(backend, f)`.
pub trait BackendExt: Backend {
    /// Runs `f` over a connection that sees one consistent state and
    /// cannot change anything.
    fn read<T, E, F>(&self, f: F) -> impl Future<Output = Result<T, E>> + Send
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
        F: FnOnce(&dyn Connection) -> Result<T, E> + Send + 'static,
    {
        run(self, false, f)
    }

    /// Runs `f` inside one transaction, which commits only if `f` returns
    /// `Ok`.
    fn write<T, E, F>(&self, f: F) -> impl Future<Output = Result<T, E>> + Send
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
        F: FnOnce(&dyn Connection) -> Result<T, E> + Send + 'static,
    {
        run(self, true, f)
    }
}

impl<B: Backend + ?Sized> BackendExt for B {}

async fn run<B, T, E, F>(backend: &B, write: bool, f: F) -> Result<T, E>
where
    B: Backend + ?Sized,
    T: Send + 'static,
    E: From<DbError> + Send + 'static,
    F: FnOnce(&dyn Connection) -> Result<T, E> + Send + 'static,
{
    let out: Arc<Mutex<Option<Result<T, E>>>> = Arc::new(Mutex::new(None));
    let slot = out.clone();
    let work: Work = Box::new(move |connection| {
        let result = f(connection);
        let failed = result.is_err();
        let Ok(mut slot) = slot.lock() else {
            return Err(DbError::Backend("the work's result was lost".into()));
        };
        *slot = Some(result);
        if failed {
            // Tells the backend to roll back; the closure's own error is
            // what the caller gets.
            Err(DbError::Backend("rolled back".into()))
        } else {
            Ok(())
        }
    });
    let done = if write {
        backend.write(work).await
    } else {
        backend.read(work).await
    };
    let out = match out.lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => None,
    };
    match (done, out) {
        (Ok(()), Some(Ok(value))) => Ok(value),
        (_, Some(Err(e))) => Err(e),
        (Err(e), _) => Err(E::from(e)),
        (Ok(()), None) => Err(E::from(DbError::Backend(
            "the backend did not run the work".into(),
        ))),
    }
}
