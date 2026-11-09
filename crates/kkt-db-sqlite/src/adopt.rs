//! Adopting a database made before kkt-db, whose schema an older store
//! counted in `user_version` alone.
//!
//! In one transaction, with foreign keys checked at the end: the old
//! triggers, indexes and full-text tables go; each declared table is
//! renamed aside; the tables are made again by the set's migration steps
//! up to the adopted version, exactly as a new database gets them; each
//! row is copied across by column name, keeping its rowid; each increasing
//! key's counter starts above the largest value it ever reached; search
//! indexes are filled; the old tables go; and the version is recorded. A
//! database adopted at a version and one made new at that version have the
//! same tables, indexes and triggers, byte for byte.
//!
//! Adopting knows no product: the caller says, in [`Legacy`], which old
//! numbers map to which versions and which old columns are left behind on
//! purpose. Anything else that does not match the declarations refuses
//! the file, unchanged.

use crate::conn::index_words;
use crate::open::{check_references, make, record};
use crate::sql::q;
use kkt_db::{ColumnType, DbError, Set, Step, schema_at};
use rusqlite::{Connection, TransactionBehavior};
use std::collections::BTreeMap;

/// A database an older store made, before kkt-db, which counted its
/// schema in `user_version`.
#[derive(Clone, Debug)]
pub struct Legacy {
    /// The set its tables become.
    pub set: &'static str,
    /// Each `user_version` that is adopted, and the version of the set its
    /// tables are.
    pub adopted: Vec<(i64, u32)>,
    /// Columns those databases have that the set leaves out on purpose:
    /// `(table, column)`. Their values are not kept.
    pub dropped: Vec<(&'static str, &'static str)>,
    /// What a database older than any adopted is told when it is refused.
    pub too_old: String,
}

/// The name a declared table has while its rows are copied out.
fn aside(table: &str) -> String {
    format!("kkt_adopting_{table}")
}

fn failed(e: rusqlite::Error) -> DbError {
    DbError::Backend(e.to_string())
}

fn refuse(what: String) -> DbError {
    DbError::Backend(format!(
        "this database cannot be adopted: {what}; it was not changed"
    ))
}

fn strings(conn: &Connection, sql: &str) -> Result<Vec<(String, String)>, DbError> {
    let mut statement = conn.prepare(sql).map_err(failed)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })
        .map_err(failed)?;
    rows.collect::<rusqlite::Result<_>>().map_err(failed)
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>, DbError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT name FROM pragma_table_info({})",
            crate::ddl::literal(&table.into())
        ))
        .map_err(failed)?;
    let rows = statement.query_map([], |r| r.get(0)).map_err(failed)?;
    rows.collect::<rusqlite::Result<_>>().map_err(failed)
}

/// Adopts the database on `conn`, whose tables are `version` of `set`.
pub(crate) fn adopt(
    conn: &mut Connection,
    set: &Set,
    version: u32,
    legacy: &Legacy,
) -> Result<(), DbError> {
    conn.pragma_update(None, "foreign_keys", "OFF")
        .map_err(failed)?;
    let result = adopt_in(conn, set, version, legacy);
    let restored = conn
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(failed);
    result.and(restored)
}

fn adopt_in(
    conn: &mut Connection,
    set: &Set,
    version: u32,
    legacy: &Legacy,
) -> Result<(), DbError> {
    let target = schema_at(&[(set, version)])?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failed)?;

    // What is there: tables (with their SQL), and the counters of
    // increasing keys.
    let tables = strings(
        &tx,
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let virtual_tables: Vec<&str> = tables
        .iter()
        .filter(|(_, sql)| sql.starts_with("CREATE VIRTUAL TABLE"))
        .map(|(name, _)| name.as_str())
        .collect();
    let counters: BTreeMap<String, i64> = if tables.is_empty() {
        BTreeMap::new()
    } else {
        let has_sequence: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'sqlite_sequence')",
                [],
                |r| r.get(0),
            )
            .map_err(failed)?;
        if has_sequence {
            let mut statement = tx
                .prepare("SELECT name, seq FROM sqlite_sequence")
                .map_err(failed)?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(failed)?;
            rows.collect::<rusqlite::Result<_>>().map_err(failed)?
        } else {
            BTreeMap::new()
        }
    };

    // It must be what the set declares at this version, less the columns
    // left behind on purpose, plus full-text tables (made again here).
    for (name, _) in &tables {
        let declared = target.table(name).is_some();
        let full_text = virtual_tables
            .iter()
            .any(|v| name == v || name.starts_with(&format!("{v}_")));
        if !declared && !full_text {
            return Err(refuse(format!(
                "it has a table {name} that kkt does not declare"
            )));
        }
    }
    let mut copied: Vec<(&str, Vec<String>, Option<&str>)> = Vec::new();
    for table in target.tables() {
        if !tables.iter().any(|(name, _)| name == table.name) {
            return Err(refuse(format!("it has no table {}", table.name)));
        }
        let have = columns(&tx, table.name)?;
        let mut from_rowid = None;
        let mut keep = Vec::new();
        for column in &table.columns {
            if have.iter().any(|c| c == column.name) {
                keep.push(column.name.to_string());
            } else if column.ty == ColumnType::Key {
                // A key the old table did not have: its rowid, which keeps
                // the order rows were added in.
                from_rowid = Some(column.name);
            } else {
                return Err(refuse(format!(
                    "{} has no column {}",
                    table.name, column.name
                )));
            }
        }
        for column in &have {
            let declared = table.column(column).is_some();
            let left = legacy.dropped.contains(&(table.name, column.as_str()));
            if !declared && !left {
                return Err(refuse(format!(
                    "{} has a column {column} that kkt does not declare",
                    table.name
                )));
            }
        }
        copied.push((table.name, keep, from_rowid));
    }

    // Clear the old triggers, indexes and full-text tables, and set the
    // old tables aside.
    for (name, _) in strings(
        &tx,
        "SELECT name, NULL FROM sqlite_master WHERE type = 'trigger'",
    )? {
        tx.execute(&format!("DROP TRIGGER {}", q(&name)), [])
            .map_err(failed)?;
    }
    for (name, _) in strings(
        &tx,
        "SELECT name, NULL FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL",
    )? {
        tx.execute(&format!("DROP INDEX {}", q(&name)), [])
            .map_err(failed)?;
    }
    for name in &virtual_tables {
        tx.execute(&format!("DROP TABLE {}", q(name)), [])
            .map_err(failed)?;
    }
    for table in target.tables() {
        tx.execute(
            &format!(
                "ALTER TABLE {} RENAME TO {}",
                q(table.name),
                q(&aside(table.name))
            ),
            [],
        )
        .map_err(failed)?;
    }

    // The tables again, made as a new database makes them.
    for steps in set.migrations.iter().take(version as usize) {
        for step in steps {
            make(&tx, step)?;
        }
    }

    // The rows, by column name, keeping their rowids.
    for (table, keep, from_rowid) in &copied {
        let mut into: Vec<String> = keep.iter().map(|c| q(c)).collect();
        let mut from: Vec<String> = into.clone();
        match from_rowid {
            Some(key) => {
                into.push(q(key));
                from.push("rowid".into());
            }
            None => {
                into.push("rowid".into());
                from.push("rowid".into());
            }
        }
        tx.execute(
            &format!(
                "INSERT INTO {} ({}) SELECT {} FROM {} ORDER BY rowid",
                q(table),
                into.join(", "),
                from.join(", "),
                q(&aside(table))
            ),
            [],
        )
        .map_err(|e| refuse(format!("a row of {table} does not fit: {e}")))?;
    }

    // Each increasing key goes on above the largest it ever reached.
    for table in target.tables() {
        let Some(key) = table.key() else { continue };
        let largest: Option<i64> = tx
            .query_row(
                &format!("SELECT max({}) FROM {}", q(key.name), q(table.name)),
                [],
                |r| r.get(0),
            )
            .map_err(failed)?;
        let counter = counters.get(table.name).copied();
        let next = largest.into_iter().chain(counter).max();
        if let Some(next) = next {
            let changed = tx
                .execute(
                    "UPDATE sqlite_sequence SET seq = max(seq, ?2) WHERE name = ?1",
                    rusqlite::params![table.name, next],
                )
                .map_err(failed)?;
            if changed == 0 {
                tx.execute(
                    "INSERT INTO sqlite_sequence (name, seq) VALUES (?1, ?2)",
                    rusqlite::params![table.name, next],
                )
                .map_err(failed)?;
            }
        }
    }

    // Search indexes over the copied rows.
    for steps in set.migrations.iter().take(version as usize) {
        for step in steps {
            if let Step::AddSearch { table, column } = step {
                let mut statement = tx
                    .prepare(&format!("SELECT rowid, {} FROM {}", q(column), q(table)))
                    .map_err(failed)?;
                let rows: Vec<(i64, Option<String>)> = statement
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(failed)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(failed)?;
                drop(statement);
                for (rowid, text) in rows {
                    if let Some(text) = text {
                        index_words(&tx, table, rowid, &text).map_err(failed)?;
                    }
                }
            }
        }
    }

    for table in target.tables() {
        tx.execute(&format!("DROP TABLE {}", q(&aside(table.name))), [])
            .map_err(failed)?;
    }
    record(&tx, set.name, version)?;
    check_references(&tx, set.name)?;
    tx.commit().map_err(failed)
}
