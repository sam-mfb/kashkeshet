//! Migrations, described for any database.
//!
//! A store's tables come from a [`Set`]: a name and an ordered list of
//! migrations, each a list of [`Step`]s. Every step adds something; none
//! removes or rewrites, so a migration that has shipped stays true. A
//! database records, for each set, how many of its migrations it has
//! applied, and applying one is one transaction with its record.
//!
//! A backend implements [`Evolve`]; [`migrate`] brings a database up to
//! date through it, and [`pending`] says what that would do.

use crate::{Column, ColumnType, Connection, Constraint, DbError, Index, Schema, Table};

/// One addition to the schema.
#[derive(Clone, Debug)]
pub enum Step {
    AddTable(Table),
    /// A column added to a table that may already hold rows. A column that
    /// is not nullable needs a default, which rows already there get. It
    /// cannot be an increasing key.
    AddColumn {
        table: &'static str,
        column: Column,
    },
    AddIndex {
        table: &'static str,
        index: Index,
    },
    AddConstraint {
        table: &'static str,
        constraint: Constraint,
    },
    /// Makes a text column searchable ([`crate::search`]): its words are
    /// indexed, those of rows already there included. One per table.
    AddSearch {
        table: &'static str,
        column: &'static str,
    },
    /// Fills in data, by Rust code using the connection's operations,
    /// inside the migration's transaction: for a value computed from other
    /// rows, which a column's default cannot give.
    Fill {
        name: &'static str,
        run: fn(&dyn Connection) -> Result<(), DbError>,
    },
}

impl Step {
    /// Adds this step to `schema`, or says why it cannot be added. A
    /// backend uses it to know the schema partway through a migration.
    pub fn apply_to(&self, schema: &mut Schema) -> Result<(), DbError> {
        match self {
            Step::AddTable(table) => schema.add_table(table.clone()),
            Step::AddColumn { table, column } => {
                let target = schema.table_mut(table)?;
                if target.column(column.name).is_some() {
                    return Err(DbError::Schema(format!(
                        "table {table} already has a column {}",
                        column.name
                    )));
                }
                if column.ty == ColumnType::Key {
                    return Err(DbError::Schema(format!(
                        "{table}.{}: a table's increasing key comes with the table",
                        column.name
                    )));
                }
                if !column.nullable && column.default.is_none() {
                    return Err(DbError::Schema(format!(
                        "{table}.{}: a column added to a table needs a default or to be nullable",
                        column.name
                    )));
                }
                target.columns.push(column.clone());
                target.check()
            }
            Step::AddIndex { table, index } => {
                let target = schema.table_mut(table)?;
                target.indexes.push(index.clone());
                target.check()
            }
            Step::AddConstraint { table, constraint } => {
                let target = schema.table_mut(table)?;
                if target.constraints.contains(constraint) {
                    return Err(DbError::Schema(format!(
                        "table {table} already has {constraint:?}"
                    )));
                }
                target.constraints.push(constraint.clone());
                target.check()
            }
            Step::AddSearch { table, column } => {
                let target = schema.table_mut(table)?;
                if target.search.is_some() {
                    return Err(DbError::Schema(format!(
                        "table {table} is already searchable"
                    )));
                }
                target.search = Some(column);
                target.check()
            }
            Step::Fill { .. } => Ok(()),
        }
    }
}

/// A named, ordered list of migrations: one store's tables. Migration
/// `n` (counting from 1) takes a database from version `n - 1` to `n`.
#[derive(Clone, Debug)]
pub struct Set {
    pub name: &'static str,
    pub migrations: Vec<Vec<Step>>,
}

impl Set {
    pub fn latest(&self) -> u32 {
        u32::try_from(self.migrations.len()).unwrap_or(u32::MAX)
    }
}

/// What applying migrations does, or did, to one set: from version `from`
/// to `to`. A set the database has never had starts at 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upgrade {
    pub set: &'static str,
    pub from: u32,
    pub to: u32,
}

/// What a backend does to run migrations.
pub trait Evolve {
    /// How many of `set`'s migrations the database has applied: 0 if it has
    /// never had the set.
    fn version(&self, set: &'static str) -> Result<u32, DbError>;

    /// In one transaction: makes `steps` (`before` is the whole schema
    /// before them, `after` the schema after them), and records `version`
    /// for `set`. On any error nothing is kept.
    fn apply(
        &self,
        set: &'static str,
        version: u32,
        before: &Schema,
        after: &Schema,
        steps: &[Step],
    ) -> Result<(), DbError>;
}

/// The schema `sets` declare, each at the version given beside it, in the
/// order given. Later sets may reference earlier sets' tables.
pub fn schema_at(sets: &[(&Set, u32)]) -> Result<Schema, DbError> {
    let mut schema = Schema::default();
    for (set, version) in sets {
        for steps in set.migrations.iter().take(*version as usize) {
            for step in steps {
                step.apply_to(&mut schema)?;
            }
            schema.check_references()?;
        }
    }
    Ok(schema)
}

/// The schema `sets` declare at their latest versions.
pub fn schema(sets: &[Set]) -> Result<Schema, DbError> {
    schema_at(&sets.iter().map(|s| (s, s.latest())).collect::<Vec<_>>())
}

/// The upgrades [`migrate`] would make, without making them. Fails if the
/// database has a set at a version newer than `sets` declare, and if two
/// sets share a name.
pub fn pending(target: &dyn Evolve, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
    for (i, set) in sets.iter().enumerate() {
        if sets[..i].iter().any(|s| s.name == set.name) {
            return Err(DbError::Schema(format!("two sets are named {}", set.name)));
        }
    }
    let mut upgrades = Vec::new();
    for set in sets {
        let from = target.version(set.name)?;
        if from > set.latest() {
            return Err(DbError::Newer {
                set: set.name,
                found: from,
                known: set.latest(),
            });
        }
        if from < set.latest() {
            upgrades.push(Upgrade {
                set: set.name,
                from,
                to: set.latest(),
            });
        }
    }
    Ok(upgrades)
}

/// Brings the database up to date: applies each set's missing migrations,
/// one transaction each, sets in the order given. Returns what it did.
pub fn migrate(target: &dyn Evolve, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
    let upgrades = pending(target, sets)?;
    let mut versions: Vec<(&Set, u32)> = Vec::new();
    for set in sets {
        versions.push((set, target.version(set.name)?));
    }
    for i in 0..versions.len() {
        let (set, from) = versions[i];
        for version in from + 1..=set.latest() {
            let before = schema_at(&versions)?;
            versions[i].1 = version;
            let after = schema_at(&versions)?;
            let steps = &set.migrations[version as usize - 1];
            target.apply(set.name, version, &before, &after, steps)?;
        }
    }
    Ok(upgrades)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> Table {
        Table::new(
            "people",
            vec![Column::key("id"), Column::text("name")],
            vec!["id"],
        )
    }

    #[test]
    fn a_set_declares_its_schema_step_by_step() {
        let set = Set {
            name: "core",
            migrations: vec![
                vec![Step::AddTable(people())],
                vec![Step::AddColumn {
                    table: "people",
                    column: Column::bool("away").default(false),
                }],
            ],
        };
        let one = schema_at(&[(&set, 1)]).unwrap();
        assert!(one.require("people").unwrap().column("away").is_none());
        let two = schema(std::slice::from_ref(&set)).unwrap();
        assert!(two.require("people").unwrap().column("away").is_some());
    }

    #[test]
    fn a_step_that_cannot_hold_is_refused() {
        let refused = |steps: Vec<Step>| {
            let set = Set {
                name: "core",
                migrations: vec![vec![Step::AddTable(people())], steps],
            };
            matches!(schema(&[set]), Err(DbError::Schema(_)))
        };
        // A column rows already there could not fill.
        assert!(refused(vec![Step::AddColumn {
            table: "people",
            column: Column::text("email"),
        }]));
        // A second table of the same name.
        assert!(refused(vec![Step::AddTable(people())]));
        // An index on a column that is not there.
        assert!(refused(vec![Step::AddIndex {
            table: "people",
            index: Index {
                name: "by_email",
                columns: vec!["email"],
            },
        }]));
        // A reference to a table that is not there.
        assert!(refused(vec![Step::AddTable(Table::new(
            "pets",
            vec![
                Column::key("id"),
                Column::int("owner").references("owners", "id")
            ],
            vec!["id"],
        ))]));
        // A reference of another type.
        assert!(refused(vec![Step::AddTable(Table::new(
            "pets",
            vec![
                Column::key("id"),
                Column::text("owner").references("people", "id")
            ],
            vec!["id"],
        ))]));
        // Positive on text.
        assert!(refused(vec![Step::AddConstraint {
            table: "people",
            constraint: Constraint::Positive("name"),
        }]));
    }
}
