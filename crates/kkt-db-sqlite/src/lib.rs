//! kkt's database contract ([`kkt_db`]) over SQLite: the only crate that
//! knows SQLite. Stores never see it; the program picks it and hands
//! stores the [`kkt_db::Database`] it opens.
//!
//! One connection writes, behind a lock, so writes run one at a time; each
//! write is an immediate transaction. Several read-only connections read,
//! each read in a deferred transaction, so it sees one state. In WAL mode
//! readers and the writer do not block each other. An in-memory database
//! has the one connection only.
//!
//! The schema is made by kkt-db's migration steps ([`ddl`]); the versions
//! applied are recorded per set in `kkt_db_versions`, and `user_version`
//! is set to [`MANAGED`] to say so. Search indexes are FTS5 tables over
//! kkt-search's words, kept in the same transaction as
//! their rows.

#[cfg(any(test, feature = "real", feature = "testing"))]
mod adopt;
#[cfg(any(test, feature = "real", feature = "testing"))]
mod conn;
#[cfg(any(test, feature = "real", feature = "testing"))]
mod ddl;
#[cfg(any(test, feature = "real", feature = "testing"))]
mod open;
#[cfg(any(test, feature = "real", feature = "testing"))]
mod sql;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "testing"))]
pub use open::MemoryPlace;
#[cfg(any(test, feature = "real"))]
pub use open::{SqlitePlace, copy_database};

use std::time::Duration;

/// The `user_version` of a database whose versions are recorded in
/// `kkt_db_versions`. Databases kkt made before kkt-db count their
/// migrations in `user_version`, from 1 to 22.
pub const MANAGED: i64 = 1000;

/// How a database file is opened.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// How many read-only connections to keep.
    pub readers: usize,
    /// How long a connection waits for a lock another process holds.
    pub busy_timeout: Duration,
}
