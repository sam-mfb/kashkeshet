//! The changes a store can make: [`Insert`], [`Update`] and [`Delete`].
//! An insert also serves `Connection::insert_unless_present` and
//! `Connection::save`.

use crate::{Condition, Value};

/// A new row of `table`: a value for each column named. A column left out
/// gets its default, or null; an increasing key is assigned and may not be
/// given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insert {
    pub table: &'static str,
    pub values: Vec<(&'static str, Value)>,
}

impl Insert {
    pub fn new(table: &'static str) -> Insert {
        Insert {
            table,
            values: Vec::new(),
        }
    }

    pub fn value(mut self, column: &'static str, value: impl Into<Value>) -> Insert {
        self.values.push((column, value.into()));
        self
    }
}

/// What an update does to one column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Assign {
    To(Value),
    /// Sets the value only in rows where the column is null.
    IfNull(Value),
}

/// Changes columns in every row of `table` that meets `filter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub table: &'static str,
    pub set: Vec<(&'static str, Assign)>,
    pub filter: Condition,
}

impl Update {
    pub fn new(table: &'static str, filter: Condition) -> Update {
        Update {
            table,
            set: Vec::new(),
            filter,
        }
    }

    pub fn set(mut self, column: &'static str, value: impl Into<Value>) -> Update {
        self.set.push((column, Assign::To(value.into())));
        self
    }

    pub fn set_if_null(mut self, column: &'static str, value: impl Into<Value>) -> Update {
        self.set.push((column, Assign::IfNull(value.into())));
        self
    }
}

/// Removes every row of `table` that meets `filter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delete {
    pub table: &'static str,
    pub filter: Condition,
}

impl Delete {
    pub fn new(table: &'static str, filter: Condition) -> Delete {
        Delete { table, filter }
    }
}
