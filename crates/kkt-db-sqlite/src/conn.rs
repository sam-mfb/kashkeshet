//! [`kkt_db::Connection`] over one SQLite connection: runs the SQL
//! [`crate::sql`] writes, reads rows back as kkt-db values, keeps search
//! indexes in step with their tables, and turns SQLite's errors into
//! kkt-db's.

use crate::sql::{OnConflict, Sql, Statement, match_query, q, search_table};
use kkt_db::search::{Hit, Search};
use kkt_db::{
    ColumnType, Connection, ConstraintKind, Count, DbError, Delete, Group, Insert, Row, Schema,
    Select, Table, Update, Value,
};
use rusqlite::types::ValueRef;
use rusqlite::{ErrorCode, ffi};

pub(crate) struct SqliteConnection<'a> {
    pub(crate) conn: &'a rusqlite::Connection,
    pub(crate) schema: &'a Schema,
    pub(crate) write: bool,
}

/// SQLite's error as kkt-db's, for a change to `table` of `schema`.
pub(crate) fn constraint_error(error: rusqlite::Error, schema: &Schema, table: &str) -> DbError {
    match db_error(error, table) {
        // A clash on the columns of a primary key kept as `UNIQUE` (beside
        // an increasing key) is a primary-key clash.
        DbError::Constraint {
            kind: ConstraintKind::Unique,
            column,
            ..
        } if schema
            .table(table)
            .is_some_and(|t| t.key().is_some_and(|k| t.primary_key != [k.name]))
            && column.as_deref().is_some_and(|c| {
                schema
                    .table(table)
                    .is_some_and(|t| t.primary_key.contains(&c))
            }) =>
        {
            DbError::Constraint {
                table: table.to_string(),
                column: None,
                kind: ConstraintKind::PrimaryKey,
            }
        }
        other => other,
    }
}

/// SQLite's error as kkt-db's, for a change to `table`.
pub(crate) fn db_error(error: rusqlite::Error, table: &str) -> DbError {
    if let rusqlite::Error::SqliteFailure(failure, message) = &error
        && failure.code == ErrorCode::ConstraintViolation
    {
        let message = message.clone().unwrap_or_default();
        if let Some(raised) = message.strip_prefix("kkt-db: ") {
            // Raised by a trigger of ours: `<kind> <table>[.<column>]`.
            let mut parts = raised.splitn(2, ' ');
            let kind = match parts.next() {
                Some("Immutable") => Some(ConstraintKind::Immutable),
                Some("NoDelete") => Some(ConstraintKind::NoDelete),
                Some("Fixed") => Some(ConstraintKind::Fixed),
                Some("Positive") => Some(ConstraintKind::Positive),
                Some("Reference") => Some(ConstraintKind::Reference),
                _ => None,
            };
            if let Some(kind) = kind {
                let at = parts.next().unwrap_or(table);
                let (table, column) = match at.split_once('.') {
                    Some((t, c)) => (t.to_string(), Some(c.to_string())),
                    None => (at.to_string(), None),
                };
                return DbError::Constraint {
                    table,
                    column,
                    kind,
                };
            }
        }
        let kind = match failure.extended_code {
            ffi::SQLITE_CONSTRAINT_PRIMARYKEY => Some(ConstraintKind::PrimaryKey),
            ffi::SQLITE_CONSTRAINT_UNIQUE => Some(ConstraintKind::Unique),
            ffi::SQLITE_CONSTRAINT_NOTNULL => Some(ConstraintKind::NotNull),
            ffi::SQLITE_CONSTRAINT_FOREIGNKEY => Some(ConstraintKind::Reference),
            _ => None,
        };
        if let Some(kind) = kind {
            // SQLite names the column as `table.column` after a colon,
            // except for foreign keys.
            let column = message
                .rsplit_once(": ")
                .and_then(|(_, at)| at.split(", ").next())
                .and_then(|at| at.split_once('.'))
                .map(|(_, c)| c.to_string());
            return DbError::Constraint {
                table: table.to_string(),
                column,
                kind,
            };
        }
    }
    DbError::Backend(error.to_string())
}

/// A value SQLite returned, as the column type says it is.
fn decode(value: ValueRef<'_>, ty: ColumnType) -> Result<Value, DbError> {
    Ok(match (value, ty) {
        (ValueRef::Null, _) => Value::Null,
        (ValueRef::Integer(v), ColumnType::Bool) => Value::Bool(v != 0),
        (ValueRef::Integer(v), _) => Value::Int(v),
        (ValueRef::Text(v), _) => Value::Text(
            std::str::from_utf8(v)
                .map_err(|e| DbError::Backend(format!("text that is not UTF-8: {e}")))?
                .to_string(),
        ),
        (ValueRef::Blob(v), _) => Value::Bytes(v.to_vec()),
        (ValueRef::Real(v), _) => {
            return Err(DbError::Backend(format!(
                "a real number, {v}, where kkt-db has none"
            )));
        }
    })
}

impl SqliteConnection<'_> {
    fn rows(&self, sql: &Sql, table: &str) -> Result<Vec<Row>, DbError> {
        let mut statement = self
            .conn
            .prepare_cached(&sql.text)
            .map_err(|e| db_error(e, table))?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(sql.params.iter()))
            .map_err(|e| db_error(e, table))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| db_error(e, table))? {
            let mut values = Vec::with_capacity(sql.types.len());
            for (i, ty) in sql.types.iter().enumerate() {
                let value = row.get_ref(i).map_err(|e| db_error(e, table))?;
                values.push(decode(value, *ty)?);
            }
            out.push(Row::new(values));
        }
        Ok(out)
    }

    fn execute(&self, sql: &Sql, table: &str) -> Result<usize, DbError> {
        let mut statement = self
            .conn
            .prepare_cached(&sql.text)
            .map_err(|e| db_error(e, table))?;
        statement
            .execute(rusqlite::params_from_iter(sql.params.iter()))
            .map_err(|e| constraint_error(e, self.schema, table))
    }

    fn writable(&self) -> Result<(), DbError> {
        if self.write {
            Ok(())
        } else {
            Err(DbError::ReadOnly)
        }
    }

    fn rowids(&self, table: &str, filter: &kkt_db::Condition) -> Result<Vec<i64>, DbError> {
        let sql = Statement::new(self.schema).rowids(table, filter)?;
        self.rows(&sql, table)?
            .into_iter()
            .map(|r| r.get::<i64>(0))
            .collect()
    }

    /// Brings the search index of `table` up to date for these rows: drops
    /// their words, and indexes the words of those still there.
    fn reindex(&self, table: &Table, rowids: &[i64]) -> Result<(), DbError> {
        let Some(column) = table.search else {
            return Ok(());
        };
        let index = q(&search_table(table.name));
        let failed = |e| db_error(e, table.name);
        for rowid in rowids {
            self.conn
                .execute(&format!("DELETE FROM {index} WHERE rowid = ?1"), [rowid])
                .map_err(failed)?;
            let text: Option<Option<String>> = self
                .conn
                .query_row(
                    &format!(
                        "SELECT {} FROM {} WHERE rowid = ?1",
                        q(column),
                        q(table.name)
                    ),
                    [rowid],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(e),
                })
                .map_err(failed)?;
            if let Some(Some(text)) = text {
                index_words(self.conn, table.name, *rowid, &text).map_err(failed)?;
            }
        }
        Ok(())
    }
}

/// Indexes the words of `text` for row `rowid` of `table`.
pub(crate) fn index_words(
    conn: &rusqlite::Connection,
    table: &str,
    rowid: i64,
    text: &str,
) -> rusqlite::Result<()> {
    let words: Vec<String> = kkt_search::words(text)
        .into_iter()
        .map(|w| w.text)
        .collect();
    conn.execute(
        &format!(
            "INSERT INTO {} (rowid, words) VALUES (?1, ?2)",
            q(&search_table(table))
        ),
        rusqlite::params![rowid, words.join(" ")],
    )?;
    Ok(())
}

impl Connection for SqliteConnection<'_> {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError> {
        let sql = Statement::new(self.schema).select(select)?;
        self.rows(&sql, select.table)
    }

    fn count(&self, count: &Count) -> Result<i64, DbError> {
        let sql = Statement::new(self.schema).count(count)?;
        match self.rows(&sql, count.table)?.first() {
            Some(row) => row.get(0),
            None => Ok(0),
        }
    }

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError> {
        let sql = Statement::new(self.schema).group(group)?;
        self.rows(&sql, group.table)
    }

    fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError> {
        let terms = kkt_search::terms(&search.terms);
        let sql = Statement::new(self.schema).search(search, match_query(&terms))?;
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        self.rows(&sql, search.select.table)?
            .into_iter()
            .map(|row| {
                let mut values = row.into_values();
                let text = match values.pop() {
                    Some(Value::Text(text)) => text,
                    _ => String::new(),
                };
                Ok(Hit {
                    excerpt: kkt_search::excerpt(
                        &text,
                        &terms,
                        search.excerpt.open,
                        search.excerpt.close,
                        search.excerpt.ellipsis,
                        search.excerpt.words,
                    ),
                    row: Row::new(values),
                })
            })
            .collect()
    }

    fn insert(&self, insert: &Insert) -> Result<Option<i64>, DbError> {
        self.writable()?;
        let sql = Statement::new(self.schema).insert(insert, OnConflict::Fail)?;
        self.execute(&sql, insert.table)?;
        let table = self.schema.require(insert.table)?;
        let rowid = self.conn.last_insert_rowid();
        self.reindex(table, &[rowid])?;
        Ok(table.key().map(|_| rowid))
    }

    fn insert_unless_present(&self, insert: &Insert) -> Result<bool, DbError> {
        self.writable()?;
        let sql = Statement::new(self.schema).insert(insert, OnConflict::Keep)?;
        let added = self.execute(&sql, insert.table)? > 0;
        if added {
            let table = self.schema.require(insert.table)?;
            self.reindex(table, &[self.conn.last_insert_rowid()])?;
        }
        Ok(added)
    }

    fn save(&self, insert: &Insert) -> Result<(), DbError> {
        self.writable()?;
        let table = self.schema.require(insert.table)?;
        let sql = Statement::new(self.schema).insert(insert, OnConflict::Replace)?;
        self.execute(&sql, insert.table)?;
        if table.search.is_some() {
            // The row inserted or updated, found by its primary key.
            let key = kkt_db::Condition::All(
                table
                    .primary_key
                    .iter()
                    .filter_map(|c| {
                        insert
                            .values
                            .iter()
                            .find(|(n, _)| n == c)
                            .map(|(_, v)| kkt_db::Condition::Eq(kkt_db::col(c), v.clone()))
                    })
                    .collect(),
            );
            let rowids = self.rowids(table.name, &key)?;
            self.reindex(table, &rowids)?;
        }
        Ok(())
    }

    fn update(&self, update: &Update) -> Result<u64, DbError> {
        self.writable()?;
        let table = self.schema.require(update.table)?;
        let searched = table
            .search
            .is_some_and(|c| update.set.iter().any(|(n, _)| *n == c));
        let rowids = if searched {
            self.rowids(table.name, &update.filter)?
        } else {
            Vec::new()
        };
        let sql = Statement::new(self.schema).update(update)?;
        let changed = self.execute(&sql, update.table)?;
        self.reindex(table, &rowids)?;
        Ok(changed as u64)
    }

    fn delete(&self, delete: &Delete) -> Result<u64, DbError> {
        self.writable()?;
        let table = self.schema.require(delete.table)?;
        let rowids = if table.search.is_some() {
            self.rowids(table.name, &delete.filter)?
        } else {
            Vec::new()
        };
        let sql = Statement::new(self.schema).delete(delete)?;
        let removed = self.execute(&sql, delete.table)?;
        self.reindex(table, &rowids)?;
        Ok(removed as u64)
    }
}
