//! What a backend implements: [`BackendConnection`], its side of one
//! connection, and [`Backend`], which runs a closure over one inside a
//! read or a write; [`Connection`], what a store asks and changes through,
//! which applies the search rules around the backend's side; and
//! [`BackendExt`], the typed read and write a store calls on any backend.

use crate::search::{Found, Hit, Reindex, Search, Terms, Words, excerpt_of};
use crate::{Assign, Count, DbError, Delete, Group, Insert, Row, Schema, Select, Update, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A backend's side of one connection: the questions and changes, with
/// the search index's words handed to it by kkt-db. Synchronous: a
/// [`Backend`] runs the closure that uses it where blocking is fine.
///
/// A backend indexes, for each row it writes, the [`Words`] its
/// [`Reindex`] gives, and matches a search's [`Terms`] against them. Both
/// come from kkt-db, so every backend splits and matches text the same way.
///
/// Inside [`BackendExt::write`], every change is part of one transaction.
/// Inside [`BackendExt::read`], a change fails with [`DbError::ReadOnly`].
pub trait BackendConnection {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError>;

    fn count(&self, count: &Count) -> Result<i64, DbError>;

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError>;

    /// The rows of `select` whose indexed words contain every one of
    /// `terms`, each term's words together and in order, with each row's
    /// searchable text. No terms find nothing. `select` must name a
    /// searchable table ([`Schema::check_search_select`]).
    fn search(&self, select: &Select, terms: &Terms) -> Result<Vec<Found>, DbError>;

    /// Adds the row, indexed as `index` says. Returns its increasing key
    /// if the table has one.
    fn insert(&self, insert: &Insert, index: &Reindex) -> Result<Option<i64>, DbError>;

    /// Adds the row, indexed as `index` says, unless a row with its
    /// primary key exists, in which case it changes nothing. Returns
    /// whether it added the row.
    fn insert_unless_present(&self, insert: &Insert, index: &Reindex) -> Result<bool, DbError>;

    /// Adds the row, or, if a row with its primary key exists, sets that
    /// row's other columns to the values given; either way the row is
    /// indexed as `index` says.
    fn save(&self, insert: &Insert, index: &Reindex) -> Result<(), DbError>;

    /// Changes the rows that meet the filter, reindexing them as `index`
    /// says. Returns how many rows met the filter.
    fn update(&self, update: &Update, index: &Reindex) -> Result<u64, DbError>;

    /// Removes the rows, and their index entries. Returns how many it
    /// removed.
    fn delete(&self, delete: &Delete) -> Result<u64, DbError>;

    /// This connection as a store uses it, with the schema the backend
    /// holds: what a [`Backend`] hands each read's or write's work.
    fn connection<'a>(&'a self, schema: &'a Schema) -> Connection<'a>
    where
        Self: Sized,
    {
        Connection {
            backend: self,
            schema,
        }
    }
}

/// The questions and changes a store makes, on one connection. kkt-db
/// applies the search rules here: it works out the words each write
/// indexes, splits a search's terms, and cuts each hit's excerpt; the
/// backend's side does the rest. A [`Backend`] makes one for each read or
/// write with [`BackendConnection::connection`].
pub struct Connection<'a> {
    backend: &'a dyn BackendConnection,
    schema: &'a Schema,
}

impl<'a> Connection<'a> {
    /// The backend's side of this connection, for a wrapper that passes
    /// its operations on (as [`crate::testing`]'s faulty backend does).
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn backend(&self) -> &'a dyn BackendConnection {
        self.backend
    }

    /// The schema this connection works with.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn schema(&self) -> &'a Schema {
        self.schema
    }

    pub fn select(&self, select: &Select) -> Result<Vec<Row>, DbError> {
        self.backend.select(select)
    }

    pub fn count(&self, count: &Count) -> Result<i64, DbError> {
        self.backend.count(count)
    }

    pub fn group(&self, group: &Group) -> Result<Vec<Row>, DbError> {
        self.backend.group(group)
    }

    /// The rows that match every term, as [`crate::search`] defines words
    /// and matching, with their excerpts.
    pub fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError> {
        self.schema.check_search(search)?;
        let terms = Terms::of(&search.terms);
        Ok(self
            .backend
            .search(&search.select, &terms)?
            .into_iter()
            .map(|found| Hit {
                excerpt: excerpt_of(&found.text, &terms, &search.excerpt),
                row: found.row,
            })
            .collect())
    }

    /// Adds the row. Returns its increasing key if the table has one.
    pub fn insert(&self, insert: &Insert) -> Result<Option<i64>, DbError> {
        self.backend.insert(insert, &self.added(insert))
    }

    /// Adds the row unless a row with its primary key exists, in which case
    /// it changes nothing. Returns whether it added the row.
    pub fn insert_unless_present(&self, insert: &Insert) -> Result<bool, DbError> {
        self.backend
            .insert_unless_present(insert, &self.added(insert))
    }

    /// Adds the row, or, if a row with its primary key exists, sets that
    /// row's other columns to the values given.
    pub fn save(&self, insert: &Insert) -> Result<(), DbError> {
        self.backend.save(insert, &self.added(insert))
    }

    /// Returns how many rows met the update's filter.
    pub fn update(&self, update: &Update) -> Result<u64, DbError> {
        let index = match self.searched(update.table) {
            Some(column) => match update.set.iter().find(|(c, _)| *c == column) {
                Some((_, Assign::To(value))) => Reindex::Set(words(value)),
                Some((_, Assign::IfNull(value))) => match words(value) {
                    Some(words) => Reindex::SetIfNull(words),
                    None => Reindex::Keep,
                },
                None => Reindex::Keep,
            },
            None => Reindex::Keep,
        };
        self.backend.update(update, &index)
    }

    /// Returns how many rows it removed.
    pub fn delete(&self, delete: &Delete) -> Result<u64, DbError> {
        self.backend.delete(delete)
    }

    /// The first row [`Connection::select`] gives, if any.
    pub fn first(&self, select: &Select) -> Result<Option<Row>, DbError> {
        let select = select.clone().limit(1);
        Ok(self.select(&select)?.into_iter().next())
    }

    /// How a row an insert or save writes is indexed: the words of the
    /// searchable column's value. A column the insert does not name is
    /// left as it is: null in a new row, which a searchable column cannot
    /// default.
    fn added(&self, insert: &Insert) -> Reindex {
        match self.searched(insert.table) {
            Some(column) => match insert.values.iter().find(|(c, _)| *c == column) {
                Some((_, value)) => Reindex::Set(words(value)),
                None => Reindex::Keep,
            },
            None => Reindex::Keep,
        }
    }

    /// The searchable column of `table`, if it has one. An unknown table
    /// has none here; the backend refuses the change.
    fn searched(&self, table: &str) -> Option<&'static str> {
        self.schema.require(table).ok().and_then(|t| t.search)
    }
}

/// The words of a searchable column's value: none for null.
fn words(value: &Value) -> Option<Words> {
    match value {
        Value::Text(text) => Some(Words::of(text)),
        _ => None,
    }
}

/// The closure a [`Backend`] runs, once: it gets a connection, and its
/// error rolls a write back.
pub type Work = Box<dyn FnOnce(&Connection) -> Result<(), DbError> + Send>;

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
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
    {
        run(self, false, f)
    }

    /// Runs `f` inside one transaction, which commits only if `f` returns
    /// `Ok`.
    fn write<T, E, F>(&self, f: F) -> impl Future<Output = Result<T, E>> + Send
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
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
    F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
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
