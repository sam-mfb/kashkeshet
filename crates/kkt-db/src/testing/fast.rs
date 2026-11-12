//! [`FastPlace`]: a fast stand-in database for tests, built on SQLite in
//! memory. SQLite is the tool here, not the thing imitated: this is not
//! kkt-db-sqlite, and shares nothing with it. Its translation of kkt-db's
//! language into SQL is its own, written to say what each operation means
//! as plainly as [`super::MemoryPlace`] does, which it follows step for
//! step: the same checks, in the same order, giving the same errors. It is
//! fast because SQLite finds rows and keeps them, where `MemoryPlace`
//! copies the whole database on every change.
//!
//! Each table is a SQLite table with untyped columns and no SQL
//! constraints: every value is stored as given (a bool as 0 or 1), and the
//! rules are checked here, after each change, by queries. A change runs in
//! a savepoint and is rolled back if a check fails. Before a change every
//! rule holds, so only the changed table, and the tables whose references
//! point into it, can break one; they are checked in the schema's order, as
//! `MemoryPlace` checks every table, so the first rule broken is the same.
//!
//! Nothing touches the disk: the database is in memory, with its temporary
//! tables there too.

use crate::search::{Hit, Search};
use crate::{
    Assign, Backend, ColumnType, Condition, Connection, Constraint, ConstraintKind, Count, DbError,
    Delete, Done, Evolve, Field, Group, Insert, Operand, OrderBy, Related, Row, Schema, Select,
    Sort, Step, Table, Update, Value, Work,
};
use rusqlite::types::{Value as SqlValue, ValueRef};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// The table that holds each table's last increasing key.
const KEYS: &str = "_fast_keys";

/// Where a fast in-memory database lives. Opening the same place again finds
/// what was written there.
#[derive(Clone)]
pub struct FastPlace {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    conn: rusqlite::Connection,
    schema: Schema,
    versions: BTreeMap<&'static str, u32>,
}

impl FastPlace {
    /// A new, empty database.
    pub fn new() -> Result<FastPlace, DbError> {
        let conn = rusqlite::Connection::open_in_memory().map_err(failed)?;
        conn.execute_batch(&format!(
            "PRAGMA temp_store = MEMORY;
             CREATE TABLE \"{KEYS}\" (name TEXT PRIMARY KEY, last INTEGER NOT NULL);"
        ))
        .map_err(failed)?;
        Ok(FastPlace {
            inner: Arc::new(Mutex::new(Inner {
                conn,
                schema: Schema::default(),
                versions: BTreeMap::new(),
            })),
        })
    }

    /// Brings the database here up to date with `sets` and opens it.
    pub fn open(&self, sets: &[crate::Set]) -> Result<Arc<dyn Backend>, DbError> {
        self.migrate(sets)?;
        Ok(Arc::new(FastBackend {
            inner: self.inner.clone(),
        }))
    }
}

fn lock(inner: &Mutex<Inner>) -> Result<MutexGuard<'_, Inner>, DbError> {
    inner
        .lock()
        .map_err(|_| DbError::Backend("the fast database's lock is poisoned".into()))
}

fn failed(e: rusqlite::Error) -> DbError {
    DbError::Backend(e.to_string())
}

impl Evolve for FastPlace {
    fn version(&self, set: &'static str) -> Result<u32, DbError> {
        Ok(lock(&self.inner)?.versions.get(set).copied().unwrap_or(0))
    }

    fn apply(
        &self,
        set: &'static str,
        version: u32,
        before: &Schema,
        after: &Schema,
        steps: &[Step],
    ) -> Result<(), DbError> {
        let mut guard = lock(&self.inner)?;
        if &guard.schema != before {
            return Err(DbError::Backend(
                "the database's tables are not what the migrations say they were".into(),
            ));
        }
        let inner = &mut *guard;
        inner.conn.execute_batch("BEGIN").map_err(failed)?;
        let mut schema = inner.schema.clone();
        let applied = (|| {
            for step in steps {
                let mut next = schema.clone();
                step.apply_to(&mut next)?;
                make(&inner.conn, step)?;
                schema = next;
                if let Step::Fill { run, .. } = step {
                    run(&FastConnection {
                        conn: &inner.conn,
                        schema: &schema,
                        write: true,
                    })?;
                }
                check(&inner.conn, &schema, None)?;
            }
            if &schema != after {
                return Err(DbError::Backend(
                    "the migration's steps do not add up to its schema".into(),
                ));
            }
            Ok(())
        })();
        match applied {
            Ok(()) => {
                inner.conn.execute_batch("COMMIT").map_err(failed)?;
                inner.schema = schema;
                inner.versions.insert(set, version);
                Ok(())
            }
            Err(e) => {
                inner.conn.execute_batch("ROLLBACK").map_err(failed)?;
                Err(e)
            }
        }
    }
}

/// Makes one migration step in SQL. Only tables, columns and indexes are
/// SQL's; constraints and search are checked and done here.
fn make(conn: &rusqlite::Connection, step: &Step) -> Result<(), DbError> {
    match step {
        Step::AddTable(table) => {
            let columns: Vec<String> = table.columns.iter().map(|c| quote(c.name)).collect();
            let mut sql = format!(
                "CREATE TABLE {} ({});",
                quote(table.name),
                columns.join(", ")
            );
            if table.key().is_some() {
                sql.push_str(&format!(
                    "INSERT INTO \"{KEYS}\" (name, last) VALUES ({}, 0);",
                    literal(&Value::Text(table.name.to_string()))
                ));
            }
            sql.push_str(&index(table.name, "pk", &table.primary_key));
            for column in table.columns.iter().filter(|c| c.unique) {
                sql.push_str(&index(table.name, column.name, &[column.name]));
            }
            for declared in &table.indexes {
                sql.push_str(&index(table.name, declared.name, &declared.columns));
            }
            conn.execute_batch(&sql).map_err(failed)
        }
        Step::AddColumn { table, column } => {
            let mut sql = format!(
                "ALTER TABLE {} ADD COLUMN {}",
                quote(table),
                quote(column.name)
            );
            if let Some(default) = &column.default {
                sql.push_str(&format!(" DEFAULT {}", literal(default)));
            }
            sql.push(';');
            if column.unique {
                sql.push_str(&index(table, column.name, &[column.name]));
            }
            conn.execute_batch(&sql).map_err(failed)
        }
        Step::AddIndex {
            table,
            index: declared,
        } => conn
            .execute_batch(&index(table, declared.name, &declared.columns))
            .map_err(failed),
        Step::AddConstraint { .. } | Step::AddSearch { .. } | Step::Fill { .. } => Ok(()),
    }
}

/// An index, for speed only: it changes no answer.
fn index(table: &str, name: &str, columns: &[&str]) -> String {
    let columns: Vec<String> = columns.iter().map(|c| quote(c)).collect();
    format!(
        "CREATE INDEX {} ON {} ({});",
        quote(&format!("_fast_{table}_{name}")),
        quote(table),
        columns.join(", ")
    )
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A value written into SQL, for a column's default.
fn literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => i64::from(*b).to_string(),
        Value::Text(t) => format!("'{}'", t.replace('\'', "''")),
        Value::Bytes(b) => {
            let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("X'{hex}'")
        }
    }
}

/// `values` as a JSON array that `json_each` reads back as the stored
/// values: a bool as 0 or 1, as stored. None if any is bytes.
fn json(values: &[Value]) -> Option<String> {
    let mut out = String::from("[");
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match value {
            Value::Null => out.push_str("null"),
            Value::Int(n) => out.push_str(&n.to_string()),
            Value::Bool(b) => out.push_str(if *b { "1" } else { "0" }),
            Value::Text(t) => {
                out.push('"');
                for c in t.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Value::Bytes(_) => return None,
        }
    }
    out.push(']');
    Some(out)
}

fn param(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Int(i) => SqlValue::Integer(*i),
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Text(t) => SqlValue::Text(t.clone()),
        Value::Bytes(b) => SqlValue::Blob(b.clone()),
    }
}

/// A stored value read back as the kind its column holds.
fn decode(value: ValueRef<'_>, ty: ColumnType) -> Result<Value, DbError> {
    Ok(match (value, ty) {
        (ValueRef::Null, _) => Value::Null,
        (ValueRef::Integer(i), ColumnType::Bool) => Value::Bool(i != 0),
        (ValueRef::Integer(i), _) => Value::Int(i),
        (ValueRef::Text(t), _) => Value::Text(
            String::from_utf8(t.to_vec())
                .map_err(|e| DbError::Backend(format!("stored text is not UTF-8: {e}")))?,
        ),
        (ValueRef::Blob(b), _) => Value::Bytes(b.to_vec()),
        (ValueRef::Real(_), _) => {
            return Err(DbError::Backend("a stored value is a real number".into()));
        }
    })
}

struct FastBackend {
    inner: Arc<Mutex<Inner>>,
}

impl Backend for FastBackend {
    fn read(&self, work: Work) -> Done<'_> {
        // The lock is held throughout, so a read sees one state: a write
        // made meanwhile waits for it.
        let done = lock(&self.inner).and_then(|inner| {
            work(&FastConnection {
                conn: &inner.conn,
                schema: &inner.schema,
                write: false,
            })
        });
        Box::pin(std::future::ready(done))
    }

    fn write(&self, work: Work) -> Done<'_> {
        // The lock is held throughout, so writes run one at a time.
        let done = lock(&self.inner).and_then(|inner| {
            inner.conn.execute_batch("BEGIN").map_err(failed)?;
            let worked = work(&FastConnection {
                conn: &inner.conn,
                schema: &inner.schema,
                write: true,
            });
            match worked {
                Ok(()) => inner.conn.execute_batch("COMMIT").map_err(failed),
                Err(e) => {
                    inner.conn.execute_batch("ROLLBACK").map_err(failed)?;
                    Err(e)
                }
            }
        });
        Box::pin(std::future::ready(done))
    }
}

struct FastConnection<'a> {
    conn: &'a rusqlite::Connection,
    schema: &'a Schema,
    write: bool,
}

/// SQL being written, with its parameters in order.
#[derive(Default)]
struct Sql {
    params: Vec<SqlValue>,
    aliases: usize,
}

impl Sql {
    fn alias(&mut self) -> String {
        self.aliases += 1;
        format!("a{}", self.aliases)
    }

    fn bind(&mut self, value: &Value) -> &'static str {
        self.params.push(param(value));
        "?"
    }
}

fn refused(table: &Table, column: Option<&str>, kind: ConstraintKind) -> DbError {
    DbError::Constraint {
        table: table.name.to_string(),
        column: column.map(str::to_string),
        kind,
    }
}

impl FastConnection<'_> {
    fn table(&self, name: &str) -> Result<&Table, DbError> {
        self.schema.require(name)
    }

    fn writable(&self) -> Result<(), DbError> {
        if self.write {
            Ok(())
        } else {
            Err(DbError::ReadOnly)
        }
    }

    /// Runs `change`, then checks every rule it could have broken, and keeps
    /// it only if both succeed.
    fn change<T>(
        &self,
        table: &str,
        change: impl FnOnce() -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        self.writable()?;
        self.conn
            .execute_batch("SAVEPOINT fast_change")
            .map_err(failed)?;
        let done = change().and_then(|value| {
            check(self.conn, self.schema, Some(table))?;
            Ok(value)
        });
        let end = match done {
            Ok(_) => "RELEASE fast_change",
            Err(_) => "ROLLBACK TO fast_change; RELEASE fast_change",
        };
        self.conn.execute_batch(end).map_err(failed)?;
        done
    }

    fn operand(
        &self,
        sql: &mut Sql,
        table: &Table,
        alias: &str,
        operand: &Operand,
    ) -> Result<String, DbError> {
        match operand {
            Operand::Column(name) => {
                table.require(name)?;
                Ok(format!("{alias}.{}", quote(name)))
            }
            Operand::Through { reference, column } => {
                let declared = table.require(reference)?;
                let Some(target) = &declared.references else {
                    return Err(DbError::Schema(format!("{reference} is not a reference")));
                };
                let to = sql.alias();
                // The first row of the referenced table holding the value,
                // as MemoryPlace finds it.
                let found = format!(
                    "(SELECT {to}.{} FROM {} AS {to} WHERE {to}.{} = {alias}.{} ORDER BY {to}.rowid LIMIT 1)",
                    quote(column),
                    quote(target.table),
                    quote(target.column),
                    quote(reference),
                );
                Ok(match &target.except {
                    Some(except) => {
                        let mark = sql.bind(except);
                        format!(
                            "(CASE WHEN {alias}.{} IS {mark} THEN NULL ELSE {found} END)",
                            quote(reference)
                        )
                    }
                    None => found,
                })
            }
        }
    }

    /// The related rows, as a FROM and WHERE over `alias` of `related.table`.
    fn related(
        &self,
        sql: &mut Sql,
        table: &Table,
        alias: &str,
        related: &Related,
    ) -> Result<(String, String, &Table), DbError> {
        table.require(related.to)?;
        let other = self.table(related.table)?;
        let theirs = sql.alias();
        let filter = self.condition(sql, other, &theirs, &related.filter)?;
        Ok((
            format!(
                "FROM {} AS {theirs} WHERE {theirs}.{} = {alias}.{} AND {filter}",
                quote(related.table),
                quote(related.column),
                quote(related.to),
            ),
            theirs,
            other,
        ))
    }

    /// A condition as SQL that is always 1 or 0, never null, so that `NOT`
    /// gives the contract's two-valued logic.
    fn condition(
        &self,
        sql: &mut Sql,
        table: &Table,
        alias: &str,
        condition: &Condition,
    ) -> Result<String, DbError> {
        let compare = |sql: &mut Sql, o: &Operand, op: &str, v: &Value| {
            let o = self.operand(sql, table, alias, o)?;
            let mark = sql.bind(v);
            Ok::<_, DbError>(format!("COALESCE({o} {op} {mark}, 0)"))
        };
        Ok(match condition {
            Condition::True => "1".into(),
            Condition::Eq(o, v) => compare(sql, o, "=", v)?,
            Condition::Ne(o, v) => compare(sql, o, "<>", v)?,
            Condition::Lt(o, v) => compare(sql, o, "<", v)?,
            Condition::Le(o, v) => compare(sql, o, "<=", v)?,
            Condition::Gt(o, v) => compare(sql, o, ">", v)?,
            Condition::Ge(o, v) => compare(sql, o, ">=", v)?,
            Condition::IsNull(o) => format!("({} IS NULL)", self.operand(sql, table, alias, o)?),
            Condition::NotNull(o) => {
                format!("({} IS NOT NULL)", self.operand(sql, table, alias, o)?)
            }
            Condition::OneOf(o, values) => {
                if values.is_empty() {
                    return Ok("0".into());
                }
                let o = self.operand(sql, table, alias, o)?;
                // A list of any length is one parameter, a JSON array, so it
                // never meets SQLite's limit on parameters. Bytes have no
                // JSON form; a list holding any is bound value by value.
                match json(values) {
                    Some(array) => {
                        let mark = sql.bind(&Value::Text(array));
                        format!("COALESCE({o} IN (SELECT value FROM json_each({mark})), 0)")
                    }
                    None => {
                        let marks: Vec<&str> = values.iter().map(|v| sql.bind(v)).collect();
                        format!("COALESCE({o} IN ({}), 0)", marks.join(", "))
                    }
                }
            }
            Condition::In(o, inner) => {
                let o = self.operand(sql, table, alias, o)?;
                let inner = self.select_sql(sql, inner, false)?;
                format!("COALESCE({o} IN ({inner}), 0)")
            }
            Condition::EqIgnoringAsciiCase(o, wanted) => {
                let o = self.operand(sql, table, alias, o)?;
                let mark = sql.bind(&Value::Text(wanted.clone()));
                // NOCASE folds ASCII letters only.
                format!("COALESCE(typeof({o}) = 'text' AND {o} = {mark} COLLATE NOCASE, 0)")
            }
            Condition::ContainsIgnoringAsciiCase(o, wanted) => {
                let first = self.operand(sql, table, alias, o)?;
                let second = self.operand(sql, table, alias, o)?;
                let mark = sql.bind(&Value::Text(wanted.clone()));
                // SQLite's lower() folds ASCII letters only.
                format!(
                    "COALESCE(typeof({first}) = 'text' AND instr(lower({second}), lower({mark})) > 0, 0)"
                )
            }
            Condition::Exists(related) => {
                let (from, ..) = self.related(sql, table, alias, related)?;
                format!("EXISTS (SELECT 1 {from})")
            }
            Condition::All(all) => {
                if all.is_empty() {
                    return Ok("1".into());
                }
                let parts = all
                    .iter()
                    .map(|c| self.condition(sql, table, alias, c))
                    .collect::<Result<Vec<_>, _>>()?;
                format!("({})", parts.join(" AND "))
            }
            Condition::Any(any) => {
                if any.is_empty() {
                    return Ok("0".into());
                }
                let parts = any
                    .iter()
                    .map(|c| self.condition(sql, table, alias, c))
                    .collect::<Result<Vec<_>, _>>()?;
                format!("({})", parts.join(" OR "))
            }
            Condition::Not(c) => format!("(NOT {})", self.condition(sql, table, alias, c)?),
        })
    }

    /// A field as SQL, and the kind of value it gives.
    fn field(
        &self,
        sql: &mut Sql,
        table: &Table,
        alias: &str,
        field: &Field,
    ) -> Result<(String, ColumnType), DbError> {
        Ok(match field {
            Field::Value(o) => (
                self.operand(sql, table, alias, o)?,
                self.schema.operand_type(table, o)?,
            ),
            Field::Count(r) => {
                let (from, ..) = self.related(sql, table, alias, r)?;
                (format!("(SELECT COUNT(*) {from})"), ColumnType::Int)
            }
            Field::Exists(r) => {
                let (from, ..) = self.related(sql, table, alias, r)?;
                (format!("EXISTS (SELECT 1 {from})"), ColumnType::Bool)
            }
            Field::Max(r, column) => {
                let (from, theirs, other) = self.related(sql, table, alias, r)?;
                let ty = other.require(column)?.ty;
                (
                    format!("(SELECT MAX({theirs}.{}) {from})", quote(column)),
                    ty,
                )
            }
            Field::Newest {
                related,
                by,
                column,
            } => {
                let (from, theirs, other) = self.related(sql, table, alias, related)?;
                let ty = other.require(column)?.ty;
                // The largest `by`, nulls lowest; of equals, the last, as
                // MemoryPlace's `max_by` gives.
                (
                    format!(
                        "(SELECT {theirs}.{} {from} ORDER BY {theirs}.{} IS NULL, {theirs}.{} DESC, {theirs}.rowid DESC LIMIT 1)",
                        quote(column),
                        quote(by),
                        quote(by),
                    ),
                    ty,
                )
            }
        })
    }

    fn sort_key(
        &self,
        sql: &mut Sql,
        table: &Table,
        alias: &str,
        by: &OrderBy,
    ) -> Result<String, DbError> {
        Ok(match by {
            OrderBy::Operand(o) => self.operand(sql, table, alias, o)?,
            OrderBy::FirstOf(a, b) => format!(
                "COALESCE({}, {})",
                self.operand(sql, table, alias, a)?,
                self.operand(sql, table, alias, b)?
            ),
        })
    }

    /// The WHERE and ORDER BY of a select over `alias`: its filter, rows
    /// after its `after` in its order, and its order, ties in the order the
    /// rows were added.
    fn picking(
        &self,
        sql: &mut Sql,
        select: &Select,
        alias: &str,
    ) -> Result<(String, String), DbError> {
        let table = self.table(select.table)?;
        let mut filter = self.condition(sql, table, alias, &select.filter)?;
        if let Some(after) = &select.after {
            // Rows that come after `after` in the order: equal on every
            // earlier sort and later on this one. Nulls sort lowest, so
            // descending they come after every value.
            let mut ways = Vec::new();
            for (i, (sort, value)) in select.order.iter().zip(after).enumerate() {
                let mut parts = Vec::new();
                for (earlier, value) in select.order[..i].iter().zip(after) {
                    let (Sort::Asc(by) | Sort::Desc(by)) = earlier;
                    let key = self.sort_key(sql, table, alias, by)?;
                    let mark = sql.bind(value);
                    parts.push(format!("COALESCE({key} = {mark}, 0)"));
                }
                parts.push(match sort {
                    Sort::Asc(by) => {
                        let key = self.sort_key(sql, table, alias, by)?;
                        let mark = sql.bind(value);
                        format!("COALESCE({key} > {mark}, 0)")
                    }
                    Sort::Desc(by) => {
                        let key = self.sort_key(sql, table, alias, by)?;
                        let again = self.sort_key(sql, table, alias, by)?;
                        let mark = sql.bind(value);
                        format!("({key} IS NULL OR COALESCE({again} < {mark}, 0))")
                    }
                });
                ways.push(format!("({})", parts.join(" AND ")));
            }
            let later = if ways.is_empty() {
                "0".to_string()
            } else {
                ways.join(" OR ")
            };
            filter = format!("{filter} AND ({later})");
        }
        let mut order = Vec::new();
        for sort in &select.order {
            // SQLite puts nulls first ascending and last descending, as
            // the contract does.
            order.push(match sort {
                Sort::Asc(by) => format!("{} ASC", self.sort_key(sql, table, alias, by)?),
                Sort::Desc(by) => format!("{} DESC", self.sort_key(sql, table, alias, by)?),
            });
        }
        order.push(format!("{alias}.rowid ASC"));
        Ok((filter, order.join(", ")))
    }

    /// A select's SQL. With `typed`, it reads its fields' kinds too, for
    /// decoding; a select inside a condition needs only its SQL.
    fn select_sql(&self, sql: &mut Sql, select: &Select, limit: bool) -> Result<String, DbError> {
        let (text, _) = self.select_typed(sql, select, limit, &[])?;
        Ok(text)
    }

    fn select_typed(
        &self,
        sql: &mut Sql,
        select: &Select,
        limit: bool,
        extra: &[&str],
    ) -> Result<(String, Vec<ColumnType>), DbError> {
        let table = self.table(select.table)?;
        let alias = sql.alias();
        let mut fields = Vec::new();
        let mut types = Vec::new();
        for field in &select.fields {
            let (text, ty) = self.field(sql, table, &alias, field)?;
            fields.push(text);
            types.push(ty);
        }
        for column in extra {
            fields.push(format!("{alias}.{}", quote(column)));
        }
        if fields.is_empty() {
            fields.push("1".into());
        }
        let (filter, order) = self.picking(sql, select, &alias)?;
        let mut text = format!(
            "SELECT {} FROM {} AS {alias} WHERE {filter} ORDER BY {order}",
            fields.join(", "),
            quote(select.table)
        );
        if let (true, Some(n)) = (limit, select.limit) {
            text.push_str(&format!(" LIMIT {}", i64::try_from(n).unwrap_or(i64::MAX)));
        }
        Ok((text, types))
    }

    fn rows(
        &self,
        text: &str,
        sql: &Sql,
        types: &[ColumnType],
    ) -> Result<Vec<Vec<Value>>, DbError> {
        let mut statement = self.conn.prepare(text).map_err(failed)?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(sql.params.iter()))
            .map_err(failed)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            let mut values = Vec::with_capacity(types.len());
            for (i, ty) in types.iter().enumerate() {
                values.push(decode(row.get_ref(i).map_err(failed)?, *ty)?);
            }
            out.push(values);
        }
        Ok(out)
    }

    fn execute(&self, text: &str, params: Vec<SqlValue>) -> Result<usize, DbError> {
        self.conn
            .execute(text, rusqlite::params_from_iter(params.iter()))
            .map_err(failed)
    }

    /// The rows of `table` that meet `filter`, each by its rowid, oldest
    /// first, with the values of `columns`.
    fn hits(
        &self,
        table: &Table,
        filter: &Condition,
        columns: &[&str],
    ) -> Result<Vec<(i64, Vec<Value>)>, DbError> {
        let mut sql = Sql::default();
        let alias = sql.alias();
        let filter = self.condition(&mut sql, table, &alias, filter)?;
        let mut fields = vec![format!("{alias}.rowid")];
        let mut types = vec![ColumnType::Int];
        for column in columns {
            fields.push(format!("{alias}.{}", quote(column)));
            types.push(table.require(column)?.ty);
        }
        let text = format!(
            "SELECT {} FROM {} AS {alias} WHERE {filter} ORDER BY {alias}.rowid",
            fields.join(", "),
            quote(table.name)
        );
        Ok(self
            .rows(&text, &sql, &types)?
            .into_iter()
            .map(|mut values| {
                let rowid = match values.remove(0) {
                    Value::Int(i) => i,
                    _ => 0,
                };
                (rowid, values)
            })
            .collect())
    }

    fn insert_row(
        &self,
        insert: &Insert,
        existing: Existing,
    ) -> Result<(bool, Option<i64>), DbError> {
        self.schema.check_insert(insert)?;
        let table = self.table(insert.table)?;
        self.change(table.name, || {
            let mut row = Vec::new();
            for column in &table.columns {
                let given = insert.values.iter().find(|(n, _)| *n == column.name);
                row.push(match given {
                    Some((_, v)) => v.clone(),
                    None => column.default.clone().unwrap_or(Value::Null),
                });
            }
            let at = |name: &str| table.columns.iter().position(|c| c.name == name);
            // A table keyed by its increasing key cannot clash: the key is
            // new. Any other primary key can.
            let present = match table.key() {
                Some(key) if table.primary_key == [key.name] => None,
                _ => {
                    let same = Condition::All(
                        table
                            .primary_key
                            .iter()
                            .map(|c| match at(c).map(|i| &row[i]) {
                                Some(Value::Null) | None => Condition::IsNull(crate::col(c)),
                                Some(v) => Condition::Eq(crate::col(c), v.clone()),
                            })
                            .collect(),
                    );
                    let given: Vec<&str> = insert.values.iter().map(|(n, _)| *n).collect();
                    self.hits(table, &same, &given)?.into_iter().next()
                }
            };
            if let Some((rowid, old)) = present {
                return match existing {
                    Existing::Refuse => Err(refused(table, None, ConstraintKind::PrimaryKey)),
                    Existing::Keep => Ok((false, None)),
                    Existing::Replace => {
                        if table.constraints.contains(&Constraint::Immutable) {
                            return Err(refused(table, None, ConstraintKind::Immutable));
                        }
                        let mut sets = Vec::new();
                        let mut params = Vec::new();
                        for ((name, value), old) in insert.values.iter().zip(&old) {
                            fixed(table, name, old, value)?;
                            sets.push(format!("{} = ?", quote(name)));
                            params.push(param(value));
                        }
                        if !sets.is_empty() {
                            params.push(SqlValue::Integer(rowid));
                            self.execute(
                                &format!(
                                    "UPDATE {} SET {} WHERE rowid = ?",
                                    quote(table.name),
                                    sets.join(", ")
                                ),
                                params,
                            )?;
                        }
                        Ok((true, None))
                    }
                };
            }
            let key = match table.key() {
                Some(key) => {
                    let last: i64 = self
                        .conn
                        .query_row(
                            &format!(
                                "UPDATE \"{KEYS}\" SET last = last + 1 WHERE name = ?1 RETURNING last"
                            ),
                            [table.name],
                            |r| r.get(0),
                        )
                        .map_err(failed)?;
                    if let Some(i) = at(key.name) {
                        row[i] = Value::Int(last);
                    }
                    Some(last)
                }
                None => None,
            };
            let names: Vec<String> = table.columns.iter().map(|c| quote(c.name)).collect();
            let marks = vec!["?"; names.len()];
            self.execute(
                &format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    quote(table.name),
                    names.join(", "),
                    marks.join(", ")
                ),
                row.iter().map(param).collect(),
            )?;
            Ok((true, key))
        })
    }
}

/// What an insert does when a row with its primary key exists.
#[derive(Clone, Copy)]
enum Existing {
    Refuse,
    Keep,
    Replace,
}

/// Refuses changing a column the table keeps fixed.
fn fixed(table: &Table, column: &str, old: &Value, new: &Value) -> Result<(), DbError> {
    let is_fixed = table
        .constraints
        .iter()
        .any(|c| matches!(c, Constraint::Fixed(f) if *f == column));
    if is_fixed && old != new {
        return Err(refused(table, Some(column), ConstraintKind::Fixed));
    }
    Ok(())
}

/// Checks the rules a change to `changed` could have broken, or with `None`
/// every rule of every table: for each table in the schema's order, each
/// row oldest first, each column in order, null where none may be, a
/// positive column, and a reference to no row; then its primary key, then
/// each unique column. A table other than `changed` can only have broken a
/// reference into `changed`, so only those are checked there.
fn check(
    conn: &rusqlite::Connection,
    schema: &Schema,
    changed: Option<&str>,
) -> Result<(), DbError> {
    for table in schema.tables() {
        let whole = changed.is_none_or(|c| c == table.name);
        let mut cases = Vec::new();
        let mut kinds = Vec::new();
        let mut params = Vec::new();
        for column in &table.columns {
            let c = format!("t.{}", quote(column.name));
            if whole && !column.nullable {
                cases.push(format!("{c} IS NULL"));
                kinds.push((column.name, ConstraintKind::NotNull));
            }
            let positive = table
                .constraints
                .iter()
                .any(|k| matches!(k, Constraint::Positive(p) if *p == column.name));
            if whole && positive {
                cases.push(format!("(typeof({c}) = 'integer' AND {c} <= 0)"));
                kinds.push((column.name, ConstraintKind::Positive));
            }
            let Some(reference) = &column.references else {
                continue;
            };
            if !whole && changed != Some(reference.table) {
                continue;
            }
            let except = match &reference.except {
                Some(v) => {
                    params.push(param(v));
                    format!(" AND {c} IS NOT ?")
                }
                None => String::new(),
            };
            cases.push(format!(
                "({c} IS NOT NULL{except} AND NOT EXISTS (SELECT 1 FROM {} AS r WHERE r.{} = {c}))",
                quote(reference.table),
                quote(reference.column)
            ));
            kinds.push((column.name, ConstraintKind::Reference));
        }
        if !cases.is_empty() {
            let whens: Vec<String> = cases
                .iter()
                .enumerate()
                .map(|(i, case)| format!("WHEN {case} THEN {i}"))
                .collect();
            let text = format!(
                "SELECT broken FROM (SELECT t.rowid AS at, CASE {} END AS broken FROM {} AS t) WHERE broken IS NOT NULL ORDER BY at LIMIT 1",
                whens.join(" "),
                quote(table.name)
            );
            let broken: Option<i64> = conn
                .query_row(&text, rusqlite::params_from_iter(params.iter()), |r| {
                    r.get(0)
                })
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(failed(e)),
                })?;
            if let Some(i) = broken {
                let (column, kind) = usize::try_from(i)
                    .ok()
                    .and_then(|i| kinds.get(i))
                    .copied()
                    .ok_or_else(|| DbError::Backend("a check named no rule".into()))?;
                return Err(refused(table, Some(column), kind));
            }
        }
        if !whole {
            continue;
        }
        let key: Vec<String> = table.primary_key.iter().map(|c| quote(c)).collect();
        if !key.is_empty() && repeats(conn, table.name, &key.join(", "), "1")? {
            return Err(refused(table, None, ConstraintKind::PrimaryKey));
        }
        for column in table.columns.iter().filter(|c| c.unique) {
            let name = quote(column.name);
            if repeats(conn, table.name, &name, &format!("{name} IS NOT NULL"))? {
                return Err(refused(table, Some(column.name), ConstraintKind::Unique));
            }
        }
    }
    Ok(())
}

/// Whether two rows of `table` that meet `filter` share the values of
/// `columns`.
fn repeats(
    conn: &rusqlite::Connection,
    table: &str,
    columns: &str,
    filter: &str,
) -> Result<bool, DbError> {
    let text = format!(
        "SELECT EXISTS (SELECT 1 FROM {} WHERE {filter} GROUP BY {columns} HAVING COUNT(*) > 1)",
        quote(table)
    );
    conn.query_row(&text, [], |r| r.get(0)).map_err(failed)
}

impl Connection for FastConnection<'_> {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError> {
        self.schema.check_select(select)?;
        let mut sql = Sql::default();
        let (text, types) = self.select_typed(&mut sql, select, true, &[])?;
        Ok(self
            .rows(&text, &sql, &types)?
            .into_iter()
            .map(Row::new)
            .collect())
    }

    fn count(&self, count: &Count) -> Result<i64, DbError> {
        self.schema.check_count(count)?;
        let table = self.table(count.table)?;
        let mut sql = Sql::default();
        let alias = sql.alias();
        let filter = self.condition(&mut sql, table, &alias, &count.filter)?;
        let text = format!(
            "SELECT COUNT(*) FROM {} AS {alias} WHERE {filter}",
            quote(count.table)
        );
        match self
            .rows(&text, &sql, &[ColumnType::Int])?
            .first()
            .and_then(|r| r.first())
        {
            Some(Value::Int(n)) => Ok(*n),
            _ => Err(DbError::Backend("a count gave no number".into())),
        }
    }

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError> {
        self.schema.check_group(group)?;
        let table = self.table(group.table)?;
        let mut sql = Sql::default();
        let alias = sql.alias();
        let mut fields = vec![self.operand(&mut sql, table, &alias, &group.by)?];
        let mut types = vec![self.schema.operand_type(table, &group.by)?];
        for also in &group.also {
            fields.push(self.operand(&mut sql, table, &alias, also)?);
            types.push(self.schema.operand_type(table, also)?);
        }
        let max = format!("MAX({alias}.{})", quote(group.max));
        fields.push("COUNT(*)".into());
        types.push(ColumnType::Int);
        fields.push(max.clone());
        types.push(table.require(group.max)?.ty);
        let filter = self.condition(&mut sql, table, &alias, &group.filter)?;
        // The `by` expression again, for GROUP BY, with its own parameters
        // after the filter's.
        let by = self.operand(&mut sql, table, &alias, &group.by)?;
        // Largest first, nulls last; of equals, the group first seen first.
        let text = format!(
            "SELECT {} FROM {} AS {alias} WHERE {filter} GROUP BY {by} ORDER BY {max} DESC, MIN({alias}.rowid)",
            fields.join(", "),
            quote(group.table)
        );
        Ok(self
            .rows(&text, &sql, &types)?
            .into_iter()
            .map(Row::new)
            .collect())
    }

    fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError> {
        self.schema.check_search(search)?;
        let select = &search.select;
        let table = self.table(select.table)?;
        let Some(column) = table.search else {
            return Err(DbError::Schema(format!(
                "table {} is not searchable",
                table.name
            )));
        };
        let terms = kkt_search::terms(&search.terms);
        // Every row the select picks, without its limit; the words are
        // matched here, as every backend matches them (kkt-search), and
        // the limit applied to the rows that match.
        let mut sql = Sql::default();
        let (text, mut types) = self.select_typed(&mut sql, select, false, &[column])?;
        types.push(ColumnType::Text);
        let limit = select
            .limit
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX));
        let mut hits = Vec::new();
        for mut values in self.rows(&text, &sql, &types)? {
            if limit.is_some_and(|n| hits.len() >= n) {
                break;
            }
            let words = match values.pop() {
                Some(Value::Text(t)) => t,
                _ => String::new(),
            };
            if !kkt_search::matches(&words, &terms) {
                continue;
            }
            hits.push(Hit {
                row: Row::new(values),
                excerpt: kkt_search::excerpt(
                    &words,
                    &terms,
                    search.excerpt.open,
                    search.excerpt.close,
                    search.excerpt.ellipsis,
                    search.excerpt.words,
                ),
            });
        }
        Ok(hits)
    }

    fn insert(&self, insert: &Insert) -> Result<Option<i64>, DbError> {
        Ok(self.insert_row(insert, Existing::Refuse)?.1)
    }

    fn insert_unless_present(&self, insert: &Insert) -> Result<bool, DbError> {
        Ok(self.insert_row(insert, Existing::Keep)?.0)
    }

    fn save(&self, insert: &Insert) -> Result<(), DbError> {
        self.insert_row(insert, Existing::Replace).map(drop)
    }

    fn update(&self, update: &Update) -> Result<u64, DbError> {
        self.schema.check_update(update)?;
        let table = self.table(update.table)?;
        self.change(table.name, || {
            let columns: Vec<&str> = update.set.iter().map(|(n, _)| *n).collect();
            let hits = self.hits(table, &update.filter, &columns)?;
            if !hits.is_empty() && table.constraints.contains(&Constraint::Immutable) {
                return Err(refused(table, None, ConstraintKind::Immutable));
            }
            for (rowid, mut old) in hits.iter().cloned() {
                let mut sets = Vec::new();
                let mut params = Vec::new();
                for (i, (name, assign)) in update.set.iter().enumerate() {
                    // A column set twice sees its first new value.
                    let before = update.set[..i]
                        .iter()
                        .rposition(|(n, _)| n == name)
                        .map_or_else(|| old[i].clone(), |j| old[j].clone());
                    let new = match assign {
                        Assign::To(v) => v.clone(),
                        Assign::IfNull(v) if before.is_null() => v.clone(),
                        Assign::IfNull(_) => before.clone(),
                    };
                    fixed(table, name, &before, &new)?;
                    old[i] = new.clone();
                    sets.push(format!("{} = ?", quote(name)));
                    params.push(param(&new));
                }
                if !sets.is_empty() {
                    params.push(SqlValue::Integer(rowid));
                    self.execute(
                        &format!(
                            "UPDATE {} SET {} WHERE rowid = ?",
                            quote(table.name),
                            sets.join(", ")
                        ),
                        params,
                    )?;
                }
            }
            Ok(hits.len() as u64)
        })
    }

    fn delete(&self, delete: &Delete) -> Result<u64, DbError> {
        self.schema.check_delete(delete)?;
        let table = self.table(delete.table)?;
        self.change(table.name, || {
            let hits = self.hits(table, &delete.filter, &[])?;
            if !hits.is_empty() {
                for (constraint, kind) in [
                    (Constraint::Immutable, ConstraintKind::Immutable),
                    (Constraint::NoDelete, ConstraintKind::NoDelete),
                ] {
                    if table.constraints.contains(&constraint) {
                        return Err(refused(table, None, kind));
                    }
                }
            }
            for chunk in hits.chunks(500) {
                let marks = vec!["?"; chunk.len()];
                self.execute(
                    &format!(
                        "DELETE FROM {} WHERE rowid IN ({})",
                        quote(table.name),
                        marks.join(", ")
                    ),
                    chunk
                        .iter()
                        .map(|(rowid, _)| SqlValue::Integer(*rowid))
                        .collect(),
                )?;
            }
            Ok(hits.len() as u64)
        })
    }
}
