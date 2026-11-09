//! Declaring tables: their columns, keys, references, indexes and
//! constraints. A store declares its tables through migrations
//! ([`crate::Step`]); [`Schema`] is what they add up to.

use crate::{DbError, Value};

/// What a column holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    Text,
    /// A 64-bit integer.
    Int,
    Bool,
    Bytes,
    /// An increasing key: an integer the database assigns on insert,
    /// above every key the table has ever had, even after deletes. At most
    /// one per table, and it is the table's primary key.
    Key,
}

impl ColumnType {
    /// Whether `value` is of this type. Null is checked separately.
    pub(crate) fn admits(self, value: &Value) -> bool {
        matches!(
            (self, value),
            (_, Value::Null)
                | (ColumnType::Text, Value::Text(_))
                | (ColumnType::Int | ColumnType::Key, Value::Int(_))
                | (ColumnType::Bool, Value::Bool(_))
                | (ColumnType::Bytes, Value::Bytes(_))
        )
    }
}

/// A column that must name a row of another table (or of its own): its
/// value is null, or equals `column` in some row of `table`, or equals
/// `except`, a value that names no row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub table: &'static str,
    pub column: &'static str,
    /// A value the column may hold that names no row, such as `operator`
    /// for an author who is no avatar.
    pub except: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: &'static str,
    pub ty: ColumnType,
    pub nullable: bool,
    /// What a row gets when an insert leaves the column out, and what rows
    /// already there get when a migration adds the column.
    pub default: Option<Value>,
    /// No two rows hold the same value; nulls do not count.
    pub unique: bool,
    pub references: Option<Reference>,
}

impl Column {
    fn new(name: &'static str, ty: ColumnType) -> Column {
        Column {
            name,
            ty,
            nullable: false,
            default: None,
            unique: false,
            references: None,
        }
    }

    pub fn text(name: &'static str) -> Column {
        Column::new(name, ColumnType::Text)
    }

    pub fn int(name: &'static str) -> Column {
        Column::new(name, ColumnType::Int)
    }

    pub fn bool(name: &'static str) -> Column {
        Column::new(name, ColumnType::Bool)
    }

    pub fn bytes(name: &'static str) -> Column {
        Column::new(name, ColumnType::Bytes)
    }

    /// An increasing key (see [`ColumnType::Key`]).
    pub fn key(name: &'static str) -> Column {
        Column::new(name, ColumnType::Key)
    }

    pub fn nullable(mut self) -> Column {
        self.nullable = true;
        self
    }

    pub fn default(mut self, value: impl Into<Value>) -> Column {
        self.default = Some(value.into());
        self
    }

    pub fn unique(mut self) -> Column {
        self.unique = true;
        self
    }

    pub fn references(mut self, table: &'static str, column: &'static str) -> Column {
        self.references = Some(Reference {
            table,
            column,
            except: None,
        });
        self
    }

    /// Lets the reference hold `value` without naming a row. Call after
    /// [`Column::references`].
    pub fn except(mut self, value: impl Into<Value>) -> Column {
        if let Some(reference) = &mut self.references {
            reference.except = Some(value.into());
        }
        self
    }
}

/// An index over columns, for speed: it changes no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub name: &'static str,
    pub columns: Vec<&'static str>,
}

/// A rule the database enforces on every change, refusing a change that
/// breaks it with [`DbError::Constraint`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Constraint {
    /// Rows are inserted once and never updated or deleted.
    Immutable,
    /// Rows are never deleted.
    NoDelete,
    /// The column never changes once a row is inserted.
    Fixed(&'static str),
    /// The integer column is above zero.
    Positive(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: &'static str,
    pub columns: Vec<Column>,
    /// The columns that identify a row. A table with a [`ColumnType::Key`]
    /// column has that column as its key.
    pub primary_key: Vec<&'static str>,
    pub indexes: Vec<Index>,
    pub constraints: Vec<Constraint>,
    /// The text column searched by [`crate::search::Search`], if any: at
    /// most one per table, added by [`crate::Step::AddSearch`].
    pub search: Option<&'static str>,
}

impl Table {
    /// A table with these columns and primary key, and no index or
    /// constraint yet.
    pub fn new(name: &'static str, columns: Vec<Column>, primary_key: Vec<&'static str>) -> Table {
        Table {
            name,
            columns,
            primary_key,
            indexes: Vec::new(),
            constraints: Vec::new(),
            search: None,
        }
    }

    pub fn index(mut self, name: &'static str, columns: Vec<&'static str>) -> Table {
        self.indexes.push(Index { name, columns });
        self
    }

    pub fn constraint(mut self, constraint: Constraint) -> Table {
        self.constraints.push(constraint);
        self
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// The column, or a schema error naming it.
    pub fn require(&self, name: &str) -> Result<&Column, DbError> {
        self.column(name)
            .ok_or_else(|| DbError::Schema(format!("table {} has no column {name}", self.name)))
    }

    /// The increasing key column, if the table has one.
    pub fn key(&self) -> Option<&Column> {
        self.columns.iter().find(|c| c.ty == ColumnType::Key)
    }

    /// Checks the table on its own: columns named once, a primary key of
    /// its own columns, at most one increasing key which is the primary
    /// key, defaults of the column's type, indexes and constraints over its
    /// columns.
    pub(crate) fn check(&self) -> Result<(), DbError> {
        let fail = |what: String| Err(DbError::Schema(format!("table {}: {what}", self.name)));
        for (i, column) in self.columns.iter().enumerate() {
            if self.columns[..i].iter().any(|c| c.name == column.name) {
                return fail(format!("column {} is declared twice", column.name));
            }
            let wrong = |v: &Value| v.is_null() || !column.ty.admits(v);
            if column.default.as_ref().is_some_and(wrong) {
                return fail(format!(
                    "column {}'s default is not a {:?}",
                    column.name, column.ty
                ));
            }
            let except = column.references.as_ref().and_then(|r| r.except.as_ref());
            if except.is_some_and(wrong) {
                return fail(format!(
                    "column {}'s exception is not a {:?}",
                    column.name, column.ty
                ));
            }
        }
        if self.primary_key.is_empty() {
            return fail("no primary key".into());
        }
        for name in &self.primary_key {
            if self.require(name)?.nullable {
                return fail(format!("primary key column {name} is nullable"));
            }
        }
        let keys: Vec<_> = self
            .columns
            .iter()
            .filter(|c| c.ty == ColumnType::Key)
            .collect();
        match keys.as_slice() {
            [] => {}
            [key] if self.primary_key == [key.name] && !key.nullable && key.default.is_none() => {}
            _ => return fail("an increasing key must be the whole primary key".into()),
        }
        for index in &self.indexes {
            if index.columns.is_empty() {
                return fail(format!("index {} has no columns", index.name));
            }
            for column in &index.columns {
                self.require(column)?;
            }
        }
        if let Some(column) = self.search
            && self.require(column)?.ty != ColumnType::Text
        {
            return fail(format!("{column} is not text, so cannot be searched"));
        }
        for constraint in &self.constraints {
            match constraint {
                Constraint::Fixed(c) => {
                    self.require(c)?;
                }
                Constraint::Positive(c) => {
                    let ty = self.require(c)?.ty;
                    if !matches!(ty, ColumnType::Int | ColumnType::Key) {
                        return fail(format!("{c} is not an integer, so cannot be positive"));
                    }
                }
                Constraint::Immutable | Constraint::NoDelete => {}
            }
        }
        Ok(())
    }
}

/// Every table a database holds, as its migrations declared them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    tables: Vec<Table>,
}

impl Schema {
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// The table, or a schema error naming it.
    pub fn require(&self, name: &str) -> Result<&Table, DbError> {
        self.table(name)
            .ok_or_else(|| DbError::Schema(format!("no table {name}")))
    }

    pub(crate) fn table_mut(&mut self, name: &str) -> Result<&mut Table, DbError> {
        self.tables
            .iter_mut()
            .find(|t| t.name == name)
            .ok_or_else(|| DbError::Schema(format!("no table {name}")))
    }

    pub(crate) fn add_table(&mut self, table: Table) -> Result<(), DbError> {
        if self.table(table.name).is_some() {
            return Err(DbError::Schema(format!(
                "table {} already exists",
                table.name
            )));
        }
        table.check()?;
        self.tables.push(table);
        Ok(())
    }

    /// Checks what one table cannot check alone: each reference names a
    /// column of a table here, of the same type, that identifies one row
    /// (the whole primary key, or unique), and index names are unique.
    pub(crate) fn check_references(&self) -> Result<(), DbError> {
        let mut index_names = Vec::new();
        for table in &self.tables {
            for index in &table.indexes {
                if index_names.contains(&index.name) {
                    return Err(DbError::Schema(format!(
                        "index {} is declared twice",
                        index.name
                    )));
                }
                index_names.push(index.name);
            }
            for column in &table.columns {
                let Some(reference) = &column.references else {
                    continue;
                };
                let target = self.require(reference.table)?;
                let named = target.require(reference.column)?;
                let same_type = named.ty == column.ty
                    || matches!((named.ty, column.ty), (ColumnType::Key, ColumnType::Int));
                let identifies = target.primary_key == [named.name] || named.unique;
                if !same_type || !identifies {
                    return Err(DbError::Schema(format!(
                        "{}.{} cannot reference {}.{}: it must be of the same type and identify one row",
                        table.name, column.name, target.name, named.name
                    )));
                }
            }
        }
        Ok(())
    }
}
