//! What a backend implements: [`Connection`], the operations on one
//! connection, and [`Backend`], which runs a closure over one inside a
//! read or a write.

use crate::{Count, DbError, Delete, Group, Insert, Row, Select, Update};

/// The questions and changes, on one connection. Synchronous: a
/// [`crate::Database`] runs the closure that uses it on a blocking thread.
///
/// Inside [`crate::Database::write`], every change is part of one
/// transaction. Inside [`crate::Database::read`], a change fails with
/// [`DbError::ReadOnly`].
pub trait Connection {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError>;

    fn count(&self, count: &Count) -> Result<i64, DbError>;

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError>;

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

/// The closure a [`Backend`] runs: it gets a connection, and its error
/// rolls a write back.
pub type Work<'a> = &'a mut dyn FnMut(&dyn Connection) -> Result<(), DbError>;

/// A database a [`crate::Database`] can run work on.
///
/// **Reads** see one consistent state: no write commits partly into a
/// read's view, and a read of several steps sees the same database
/// throughout.
///
/// **Writes** are transactions that run as if alone: no other write
/// commits between a write's reads and its changes. A store relies on
/// this to read something, such as whether a name is taken, and then
/// change the database on that basis.
pub trait Backend: Send + Sync + 'static {
    /// Runs `work` once over a connection that cannot change anything.
    fn read(&self, work: Work<'_>) -> Result<(), DbError>;

    /// Runs `work` once inside a transaction, and commits only if it
    /// returns `Ok`. On `Err`, nothing it changed is kept, and the error
    /// is returned.
    fn write(&self, work: Work<'_>) -> Result<(), DbError>;
}
