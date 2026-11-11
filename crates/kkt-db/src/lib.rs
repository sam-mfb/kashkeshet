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
//! [`search`] says what a search asks for and finds. Words, matching and
//! excerpts are kkt-search's, the same for every backend: a backend indexes
//! the words kkt-search gives and never splits text itself.
//!
//! A store is given a [`Backend`] and calls [`BackendExt`]'s typed
//! methods on it. [`BackendExt::read`] runs a closure over a connection
//! that sees one consistent state and cannot change anything;
//! [`BackendExt::write`] runs one inside a transaction that commits only
//! if the closure succeeds, and runs as if no other write could commit
//! while it does (see [`Backend`]).
//!
//! Constraints ([`Constraint`], and the keys, references, uniqueness and
//! nullability of [`Column`]) are enforced by the backend on every change:
//! a change that would break one fails with [`DbError::Constraint`] and
//! changes nothing.

mod change;
mod check;
mod connection;
mod migration;
mod query;
mod schema;
pub mod search;
mod value;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use change::{Assign, Delete, Insert, Update};
pub use connection::{Backend, BackendExt, Connection, Done, Work};
pub use migration::{Evolve, Legacy, Set, Step, Upgrade};
pub use query::{
    Condition, Count, Field, Group, Operand, OrderBy, Related, Select, Sort, col, through,
};
pub use schema::{Column, ColumnType, Constraint, Index, Reference, Schema, Table};
pub use value::{FromValue, Row, Value};

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
    /// A change inside [`BackendExt::read`].
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
    /// The task a backend ran a read or write on failed.
    #[error("database task failed: {0}")]
    Task(String),
}
