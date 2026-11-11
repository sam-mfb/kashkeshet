//! The SQL that makes each migration step: tables, columns, indexes,
//! constraints as triggers, and search indexes.
//!
//! A database's schema is always made by these steps in order, whether it
//! is new or upgraded, so two databases at one version have the same
//! `sqlite_master`, byte for byte.
//!
//! What SQLite enforces itself: primary keys, `UNIQUE`, `NOT NULL`, and
//! references without an exception value (`REFERENCES`, with foreign keys
//! on). Everything else is a trigger that raises `kkt-db: <kind> …`, which
//! [`crate::conn`] reads back as the constraint's kind.

use crate::sql::{q, search_table};
use kkt_db::{Column, ColumnType, Constraint, Reference, Table, Value};

/// A value written into SQL text, for defaults and trigger conditions,
/// where parameters cannot go.
pub(crate) fn literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => i64::from(*v).to_string(),
        Value::Text(v) => format!("'{}'", v.replace('\'', "''")),
        Value::Bytes(v) => {
            let hex: String = v.iter().map(|b| format!("{b:02X}")).collect();
            format!("X'{hex}'")
        }
    }
}

fn sql_type(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Text => "TEXT",
        ColumnType::Int | ColumnType::Key | ColumnType::Bool => "INTEGER",
        ColumnType::Bytes => "BLOB",
    }
}

/// A trigger body that refuses the change as a `kind` of constraint.
fn raise(kind: &str, table: &str, column: Option<&str>) -> String {
    let at = match column {
        Some(c) => format!("{table}.{c}"),
        None => table.to_string(),
    };
    format!("BEGIN SELECT RAISE(ABORT, 'kkt-db: {kind} {at}'); END")
}

/// A column's definition. `added` is for `ALTER TABLE … ADD COLUMN`, which
/// cannot declare `UNIQUE`: an index does that instead.
fn column_definition(column: &Column, added: bool) -> String {
    let mut text = format!("{} {}", q(column.name), sql_type(column.ty));
    if column.ty == ColumnType::Key {
        text.push_str(" PRIMARY KEY AUTOINCREMENT");
    }
    if !column.nullable && column.ty != ColumnType::Key {
        text.push_str(" NOT NULL");
    }
    if let Some(default) = &column.default {
        text.push_str(&format!(" DEFAULT {}", literal(default)));
    }
    if column.unique && !added {
        text.push_str(" UNIQUE");
    }
    if let Some(Reference {
        table,
        column: target,
        except: None,
    }) = &column.references
    {
        text.push_str(&format!(" REFERENCES {}({})", q(table), q(target)));
    }
    text
}

/// The triggers that keep a reference with an exception value, which
/// SQLite's `REFERENCES` cannot express.
fn reference_triggers(table: &str, column: &Column) -> Vec<String> {
    let Some(Reference {
        table: parent,
        column: key,
        except: Some(except),
    }) = &column.references
    else {
        return Vec::new();
    };
    let (c, t, p, k) = (q(column.name), q(table), q(parent), q(key));
    let except = literal(except);
    let name = |what: &str| q(&format!("kkt_{table}_{}_reference_{what}", column.name));
    let names_no_row = |new: &str| {
        format!(
            "{new}.{c} IS NOT NULL AND {new}.{c} IS NOT {except} AND NOT EXISTS (SELECT 1 FROM {p} WHERE {k} = {new}.{c})"
        )
    };
    let referenced =
        format!("EXISTS (SELECT 1 FROM {t} WHERE {c} = OLD.{k} AND {c} IS NOT {except})");
    let refuse = raise("Reference", table, Some(column.name));
    vec![
        format!(
            "CREATE TRIGGER {} BEFORE INSERT ON {t} WHEN {} {refuse}",
            name("insert"),
            names_no_row("NEW")
        ),
        format!(
            "CREATE TRIGGER {} BEFORE UPDATE OF {c} ON {t} WHEN {} {refuse}",
            name("update"),
            names_no_row("NEW")
        ),
        format!(
            "CREATE TRIGGER {} BEFORE DELETE ON {p} WHEN {referenced} {refuse}",
            name("delete")
        ),
        format!(
            "CREATE TRIGGER {} BEFORE UPDATE OF {k} ON {p} WHEN NEW.{k} IS NOT OLD.{k} AND {referenced} {refuse}",
            name("rename")
        ),
    ]
}

/// The triggers that keep `constraint` on `table`.
fn constraint_triggers(table: &str, constraint: &Constraint) -> Vec<String> {
    let t = q(table);
    match constraint {
        Constraint::Immutable => vec![
            format!(
                "CREATE TRIGGER {} BEFORE UPDATE ON {t} {}",
                q(&format!("kkt_{table}_immutable_update")),
                raise("Immutable", table, None)
            ),
            format!(
                "CREATE TRIGGER {} BEFORE DELETE ON {t} {}",
                q(&format!("kkt_{table}_immutable_delete")),
                raise("Immutable", table, None)
            ),
        ],
        Constraint::NoDelete => vec![format!(
            "CREATE TRIGGER {} BEFORE DELETE ON {t} {}",
            q(&format!("kkt_{table}_no_delete")),
            raise("NoDelete", table, None)
        )],
        Constraint::Fixed(c) => vec![format!(
            "CREATE TRIGGER {} BEFORE UPDATE OF {} ON {t} WHEN NEW.{} IS NOT OLD.{} {}",
            q(&format!("kkt_{table}_{c}_fixed")),
            q(c),
            q(c),
            q(c),
            raise("Fixed", table, Some(c))
        )],
        Constraint::Positive(c) => vec![
            format!(
                "CREATE TRIGGER {} BEFORE INSERT ON {t} WHEN NEW.{} <= 0 {}",
                q(&format!("kkt_{table}_{c}_positive_insert")),
                q(c),
                raise("Positive", table, Some(c))
            ),
            format!(
                "CREATE TRIGGER {} BEFORE UPDATE OF {} ON {t} WHEN NEW.{} <= 0 {}",
                q(&format!("kkt_{table}_{c}_positive_update")),
                q(c),
                q(c),
                raise("Positive", table, Some(c))
            ),
        ],
    }
}

fn index(table: &str, name: &str, columns: &[&str], unique: bool) -> String {
    let columns: Vec<String> = columns.iter().map(|c| q(c)).collect();
    format!(
        "CREATE {}INDEX {} ON {} ({})",
        if unique { "UNIQUE " } else { "" },
        q(name),
        q(table),
        columns.join(", ")
    )
}

/// The statements that make a new table, as a migration declares it.
pub(crate) fn add_table(table: &Table) -> Vec<String> {
    let mut definitions: Vec<String> = table
        .columns
        .iter()
        .map(|c| column_definition(c, false))
        .collect();
    let key: Vec<String> = table.primary_key.iter().map(|c| q(c)).collect();
    match table.key() {
        None => definitions.push(format!("PRIMARY KEY ({})", key.join(", "))),
        // An increasing key beside another primary key is the rowid; the
        // primary key is kept unique. kkt-db-sqlite reads a clash on it as
        // a primary-key clash.
        Some(k) if table.primary_key != [k.name] => {
            definitions.push(format!("UNIQUE ({})", key.join(", ")))
        }
        Some(_) => {}
    }
    let mut statements = vec![format!(
        "CREATE TABLE {} ({})",
        q(table.name),
        definitions.join(", ")
    )];
    for i in &table.indexes {
        statements.push(index(table.name, i.name, &i.columns, false));
    }
    for column in &table.columns {
        statements.extend(reference_triggers(table.name, column));
    }
    for constraint in &table.constraints {
        statements.extend(constraint_triggers(table.name, constraint));
    }
    statements
}

/// The statements that add a column to a table that may hold rows.
pub(crate) fn add_column(table: &str, column: &Column) -> Vec<String> {
    let mut statements = vec![format!(
        "ALTER TABLE {} ADD COLUMN {}",
        q(table),
        column_definition(column, true)
    )];
    if column.unique {
        statements.push(index(
            table,
            &format!("kkt_{table}_{}_unique", column.name),
            &[column.name],
            true,
        ));
    }
    statements.extend(reference_triggers(table, column));
    statements
}

pub(crate) fn add_index(table: &str, name: &str, columns: &[&str]) -> String {
    index(table, name, columns, false)
}

pub(crate) fn add_constraint(table: &str, constraint: &Constraint) -> Vec<String> {
    constraint_triggers(table, constraint)
}

/// A query that finds a row already breaking `constraint`, for a
/// constraint added to a table that may hold rows.
pub(crate) fn breaking(table: &str, constraint: &Constraint) -> Option<String> {
    match constraint {
        Constraint::Positive(c) => Some(format!(
            "SELECT 1 FROM {} WHERE {} <= 0 LIMIT 1",
            q(table),
            q(c)
        )),
        Constraint::Immutable | Constraint::NoDelete | Constraint::Fixed(_) => None,
    }
}

/// The search index of `table`: FTS5 over kkt-search's words, which it stores
/// whole (the `ascii` tokenizer splits only at ASCII punctuation and
/// spaces, which words never hold), keeping no copy of the text.
pub(crate) fn add_search(table: &str) -> String {
    format!(
        "CREATE VIRTUAL TABLE {} USING fts5(words, tokenize = 'ascii', content = '', contentless_delete = 1)",
        q(&search_table(table))
    )
}
