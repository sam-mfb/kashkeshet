//! kkt's database contract, for any database.
//!
//! A store declares its tables as migrations ([`Set`], [`Step`]), and asks
//! and changes through a [`Connection`]: [`Select`], [`Count`] and
//! [`Group`] to ask, [`Insert`], [`Update`] and [`Delete`] to change. The
//! language is narrow on purpose: it holds what kkt's stores need, and no
//! SQL or other database's terms. A backend, such as kkt-db-sqlite,
//! implements [`Backend`] and [`Evolve`] for one database and must pass
//! [`testing::conformance`]; a store never learns which backend it has.
//!
//! [`search`] defines words, matching and excerpts once for every backend:
//! a backend indexes the words [`search::words`] gives and never splits
//! text itself.
//!
//! [`Database`] is what a store is given. [`Database::read`] runs a
//! closure over a connection that sees one consistent state and cannot
//! change anything; [`Database::write`] runs one inside a transaction that
//! commits only if the closure succeeds, and runs as if no other write
//! could commit while it does (see [`Backend`]).
//!
//! Constraints ([`Constraint`], and the keys, references, uniqueness and
//! nullability of [`Column`]) are enforced by the backend on every change:
//! a change that would break one fails with [`DbError::Constraint`] and
//! changes nothing.

mod change;
pub mod check;
mod connection;
mod migration;
mod query;
mod schema;
pub mod search;
mod value;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use change::{Assign, Delete, Insert, Update};
pub use connection::{Backend, Connection, Work};
pub use migration::{Evolve, Set, Step, Upgrade, migrate, pending, schema, schema_at};
pub use query::{
    Condition, Count, Field, Group, Operand, OrderBy, Related, Select, Sort, col, through,
};
pub use schema::{Column, ColumnType, Constraint, Index, Reference, Schema, Table};
pub use value::{FromValue, Row, Value};

use std::sync::Arc;

/// Which rule a refused change would have broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    /// A second row with the same primary key.
    PrimaryKey,
    /// A second row with the same value in a unique column.
    Unique,
    /// Null in a column that is not nullable.
    NotNull,
    /// A reference to no row, or a change or delete that would leave a
    /// reference to no row.
    Reference,
    /// See [`Constraint::Positive`].
    Positive,
    /// See [`Constraint::Immutable`].
    Immutable,
    /// See [`Constraint::NoDelete`].
    NoDelete,
    /// See [`Constraint::Fixed`].
    Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DbError {
    /// The database itself failed, or refused in a way the contract does
    /// not name.
    #[error("database error: {0}")]
    Backend(String),
    /// A change would have broken a rule of the schema; nothing changed.
    #[error("{table}: the change breaks {kind:?}{}", column.as_ref().map(|c| format!(" on {c}")).unwrap_or_default())]
    Constraint {
        table: String,
        column: Option<String>,
        kind: ConstraintKind,
    },
    /// A question or change that does not fit the schema: a bug in the
    /// store that asked it.
    #[error("not in the schema: {0}")]
    Schema(String),
    /// A value read as the wrong kind: a bug in the store that read it.
    #[error("expected a {expected} value, found {found}")]
    Type {
        expected: &'static str,
        found: &'static str,
    },
    /// A change inside [`Database::read`].
    #[error("a read cannot change the database")]
    ReadOnly,
    /// The database has a set at a version newer than this program knows.
    #[error(
        "the database's {set} tables are at version {found}, newer than this version knows ({known})"
    )]
    Newer {
        set: &'static str,
        found: u32,
        known: u32,
    },
    /// The blocking task running a read or write failed.
    #[error("database task failed: {0}")]
    Task(String),
}

/// A database, as a store is given it. Cloning shares it.
#[derive(Clone)]
pub struct Database {
    backend: Arc<dyn Backend>,
    #[cfg(any(test, feature = "testing"))]
    faults: testing::Faults,
}

impl Database {
    pub fn new(backend: impl Backend) -> Database {
        Database {
            backend: Arc::new(backend),
            #[cfg(any(test, feature = "testing"))]
            faults: testing::Faults::default(),
        }
    }

    /// Runs `f` over a connection that sees one consistent state and
    /// cannot change anything.
    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
        F: FnOnce(&dyn Connection) -> Result<T, E> + Send + 'static,
    {
        self.run(false, f).await
    }

    /// Runs `f` inside one transaction, which commits only if `f` returns
    /// `Ok`.
    pub async fn write<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
        F: FnOnce(&dyn Connection) -> Result<T, E> + Send + 'static,
    {
        self.run(true, f).await
    }

    async fn run<T, E, F>(&self, write: bool, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
        F: FnOnce(&dyn Connection) -> Result<T, E> + Send + 'static,
    {
        let backend = self.backend.clone();
        #[cfg(any(test, feature = "testing"))]
        let faults = self.faults.clone();
        let task = tokio::task::spawn_blocking(move || {
            let mut f = Some(f);
            let mut out: Option<Result<T, E>> = None;
            let mut work = |connection: &dyn Connection| -> Result<(), DbError> {
                let Some(f) = f.take() else {
                    return Err(DbError::Backend("the backend ran the work twice".into()));
                };
                #[cfg(any(test, feature = "testing"))]
                let faulty = testing::Faulty::new(connection, &faults);
                #[cfg(any(test, feature = "testing"))]
                let connection: &dyn Connection = &faulty;
                let result = f(connection);
                let failed = result.is_err();
                out = Some(result);
                if failed {
                    // Tells the backend to roll back; the closure's own
                    // error is what the caller gets.
                    Err(DbError::Backend("rolled back".into()))
                } else {
                    Ok(())
                }
            };
            let done = if write {
                backend.write(&mut work)
            } else {
                backend.read(&mut work)
            };
            match (done, out) {
                (Ok(()), Some(Ok(value))) => Ok(value),
                (_, Some(Err(e))) => Err(e),
                (Err(e), _) => Err(E::from(e)),
                (Ok(()), None) => Err(E::from(DbError::Backend(
                    "the backend did not run the work".into(),
                ))),
            }
        });
        match task.await {
            Ok(result) => result,
            Err(join) => Err(E::from(DbError::Task(join.to_string()))),
        }
    }
}
