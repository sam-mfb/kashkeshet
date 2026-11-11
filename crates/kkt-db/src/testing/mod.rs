//! For tests: [`MemoryPlace`], a database held in memory that keeps the
//! contract; [`conformance`], the checks every backend must pass; and
//! [`FaultyBackend`], which wraps a backend and makes a change fail on
//! purpose.

mod conformance;
mod memory;

pub use conformance::{Place, conformance};
pub use memory::MemoryPlace;

use crate::search::{Hit, Search};
use crate::{
    Backend, Connection, Count, DbError, Delete, Done, Group, Insert, Row, Select, Update, Work,
};
use std::sync::{Arc, Mutex};

/// The tables whose next change fails, shared by a backend's clones.
#[derive(Clone, Default)]
struct Faults(Arc<Mutex<Vec<&'static str>>>);

impl Faults {
    /// Whether a change to `table` fails now; if so, only this once.
    fn take(&self, table: &str) -> bool {
        let Ok(mut tables) = self.0.lock() else {
            return true;
        };
        match tables.iter().position(|t| *t == table) {
            Some(i) => {
                tables.remove(i);
                true
            }
            None => false,
        }
    }
}

/// A backend that wraps another and fails the next change to a table on
/// cue. Cloning shares both the backend and the cues.
#[derive(Clone)]
pub struct FaultyBackend {
    inner: Arc<dyn Backend>,
    faults: Faults,
}

impl FaultyBackend {
    pub fn new(inner: Arc<dyn Backend>) -> FaultyBackend {
        FaultyBackend {
            inner,
            faults: Faults::default(),
        }
    }

    /// Makes the next insert, save, update or delete on `table`, in any
    /// write through this backend or its clones, fail with
    /// [`DbError::Backend`], as if the database had failed.
    pub fn fail_next_change_to(&self, table: &'static str) {
        if let Ok(mut tables) = self.faults.0.lock() {
            tables.push(table);
        }
    }

    fn wrap(&self, work: Work) -> Work {
        let faults = self.faults.clone();
        Box::new(move |connection| work(&Faulty::new(connection, &faults)))
    }
}

impl Backend for FaultyBackend {
    fn read(&self, work: Work) -> Done<'_> {
        self.inner.read(self.wrap(work))
    }

    fn write(&self, work: Work) -> Done<'_> {
        self.inner.write(self.wrap(work))
    }
}

/// A connection whose changes fail where [`Faults`] says.
struct Faulty<'a> {
    inner: &'a dyn Connection,
    faults: &'a Faults,
}

impl<'a> Faulty<'a> {
    fn new(inner: &'a dyn Connection, faults: &'a Faults) -> Faulty<'a> {
        Faulty { inner, faults }
    }

    fn check(&self, table: &str) -> Result<(), DbError> {
        if self.faults.take(table) {
            return Err(DbError::Backend(format!(
                "a change to {table} failed on purpose"
            )));
        }
        Ok(())
    }
}

impl Connection for Faulty<'_> {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError> {
        self.inner.select(select)
    }

    fn count(&self, count: &Count) -> Result<i64, DbError> {
        self.inner.count(count)
    }

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError> {
        self.inner.group(group)
    }

    fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError> {
        self.inner.search(search)
    }

    fn insert(&self, insert: &Insert) -> Result<Option<i64>, DbError> {
        self.check(insert.table)?;
        self.inner.insert(insert)
    }

    fn insert_unless_present(&self, insert: &Insert) -> Result<bool, DbError> {
        self.check(insert.table)?;
        self.inner.insert_unless_present(insert)
    }

    fn save(&self, insert: &Insert) -> Result<(), DbError> {
        self.check(insert.table)?;
        self.inner.save(insert)
    }

    fn update(&self, update: &Update) -> Result<u64, DbError> {
        self.check(update.table)?;
        self.inner.update(update)
    }

    fn delete(&self, delete: &Delete) -> Result<u64, DbError> {
        self.check(delete.table)?;
        self.inner.delete(delete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_memory_database_keeps_the_contract() {
        conformance(|| Box::new(MemoryPlace::new())).await;
    }
}
