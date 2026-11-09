//! A database held in memory that keeps the whole contract: the stand-in
//! for tests, and the plainest statement of what each operation means.
//! It is slow on purpose: every change works on a copy of the whole
//! database, which replaces it only if the change keeps every constraint.

use crate::search::{self, Hit, Search};
use crate::{
    Assign, Backend, Column, Condition, Connection, Constraint, ConstraintKind, Count, Database,
    DbError, Delete, Evolve, Field, Group, Insert, Operand, OrderBy, Related, Row, Schema, Select,
    Set, Sort, Step, Table, Update, Value, Work, check, migrate,
};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug, Default)]
struct State {
    schema: Schema,
    /// Each table's rows, each with a value per column in the table's
    /// column order, oldest first.
    data: BTreeMap<&'static str, Data>,
    versions: BTreeMap<&'static str, u32>,
}

#[derive(Clone, Debug, Default)]
struct Data {
    rows: Vec<Vec<Value>>,
    /// The largest increasing key ever assigned.
    last_key: i64,
}

/// Where an in-memory database lives. Opening the same place again, as a
/// program does on restart, finds what was written there.
#[derive(Clone, Default)]
pub struct MemoryPlace {
    state: Arc<Mutex<State>>,
}

impl MemoryPlace {
    pub fn new() -> MemoryPlace {
        MemoryPlace::default()
    }

    /// Brings the database here up to date with `sets` and opens it.
    pub fn open(&self, sets: &[Set]) -> Result<Database, DbError> {
        migrate(self, sets)?;
        Ok(Database::new(MemoryBackend {
            state: self.state.clone(),
        }))
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, DbError> {
        lock(&self.state)
    }
}

fn lock(state: &Mutex<State>) -> Result<MutexGuard<'_, State>, DbError> {
    state
        .lock()
        .map_err(|_| DbError::Backend("the in-memory database's lock is poisoned".into()))
}

impl Evolve for MemoryPlace {
    fn version(&self, set: &'static str) -> Result<u32, DbError> {
        Ok(self.lock()?.versions.get(set).copied().unwrap_or(0))
    }

    fn apply(
        &self,
        set: &'static str,
        version: u32,
        before: &Schema,
        after: &Schema,
        steps: &[Step],
    ) -> Result<(), DbError> {
        let mut guard = self.lock()?;
        let mut state = guard.clone();
        if &state.schema != before {
            return Err(DbError::Backend(
                "the database's tables are not what the migrations say they were".into(),
            ));
        }
        for step in steps {
            let mut schema = state.schema.clone();
            step.apply_to(&mut schema)?;
            match step {
                Step::AddTable(table) => {
                    state.data.insert(table.name, Data::default());
                }
                Step::AddColumn { table, column } => {
                    let fill = column.default.clone().unwrap_or(Value::Null);
                    if let Some(data) = state.data.get_mut(table) {
                        for row in &mut data.rows {
                            row.push(fill.clone());
                        }
                    }
                }
                Step::AddIndex { .. }
                | Step::AddConstraint { .. }
                | Step::AddSearch { .. }
                | Step::Fill { .. } => {}
            }
            state.schema = schema;
            if let Step::Fill { run, .. } = step {
                let connection = MemoryConnection {
                    state: RefCell::new(state),
                    write: true,
                };
                run(&connection)?;
                state = connection.state.into_inner();
            }
            check_all(&state)?;
        }
        if &state.schema != after {
            return Err(DbError::Backend(
                "the migration's steps do not add up to its schema".into(),
            ));
        }
        state.versions.insert(set, version);
        *guard = state;
        Ok(())
    }
}

struct MemoryBackend {
    state: Arc<Mutex<State>>,
}

impl Backend for MemoryBackend {
    fn read(&self, work: Work<'_>) -> Result<(), DbError> {
        // A copy taken under the lock: one consistent state.
        let state = lock(&self.state)?.clone();
        work(&MemoryConnection {
            state: RefCell::new(state),
            write: false,
        })
    }

    fn write(&self, work: Work<'_>) -> Result<(), DbError> {
        // The lock is held throughout, so writes run one at a time.
        let mut guard = lock(&self.state)?;
        let connection = MemoryConnection {
            state: RefCell::new(guard.clone()),
            write: true,
        };
        work(&connection)?;
        *guard = connection.state.into_inner();
        Ok(())
    }
}

struct MemoryConnection {
    state: RefCell<State>,
    write: bool,
}

fn refused(table: &Table, column: Option<&str>, kind: ConstraintKind) -> DbError {
    DbError::Constraint {
        table: table.name.to_string(),
        column: column.map(str::to_string),
        kind,
    }
}

fn position(table: &Table, column: &str) -> Result<usize, DbError> {
    table
        .columns
        .iter()
        .position(|c| c.name == column)
        .ok_or_else(|| DbError::Schema(format!("table {} has no column {column}", table.name)))
}

/// Values compared as the contract says: same kinds only, text by code
/// point; `None` when either is null.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Text(a), Value::Text(b)) => Some(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
        (Value::Bytes(a), Value::Bytes(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// The order of sorting: nulls before everything.
fn sort_compare(a: &Value, b: &Value) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => compare(a, b).unwrap_or(Ordering::Equal),
    }
}

/// Reading the database: every evaluation is over one state.
struct View<'a> {
    state: &'a State,
}

impl View<'_> {
    fn table(&self, name: &str) -> Result<&Table, DbError> {
        self.state.schema.require(name)
    }

    fn rows(&self, table: &str) -> &[Vec<Value>] {
        self.state
            .data
            .get(table)
            .map(|d| d.rows.as_slice())
            .unwrap_or(&[])
    }

    fn operand(&self, table: &Table, row: &[Value], operand: &Operand) -> Result<Value, DbError> {
        match operand {
            Operand::Column(name) => Ok(row[position(table, name)?].clone()),
            Operand::Through { reference, column } => {
                let declared = table.require(reference)?;
                let Some(target) = &declared.references else {
                    return Err(DbError::Schema(format!("{reference} is not a reference")));
                };
                let value = &row[position(table, reference)?];
                if value.is_null() || target.except.as_ref() == Some(value) {
                    return Ok(Value::Null);
                }
                let other = self.table(target.table)?;
                let (by, read) = (position(other, target.column)?, position(other, column)?);
                Ok(self
                    .rows(target.table)
                    .iter()
                    .find(|r| compare(&r[by], value) == Some(Ordering::Equal))
                    .map(|r| r[read].clone())
                    .unwrap_or(Value::Null))
            }
        }
    }

    fn related(
        &self,
        table: &Table,
        row: &[Value],
        related: &Related,
    ) -> Result<(&Table, Vec<&Vec<Value>>), DbError> {
        let other = self.table(related.table)?;
        let ours = &row[position(table, related.to)?];
        let theirs = position(other, related.column)?;
        let mut found = Vec::new();
        for candidate in self.rows(related.table) {
            if compare(&candidate[theirs], ours) == Some(Ordering::Equal)
                && self.condition(other, candidate, &related.filter)?
            {
                found.push(candidate);
            }
        }
        Ok((other, found))
    }

    fn condition(
        &self,
        table: &Table,
        row: &[Value],
        condition: &Condition,
    ) -> Result<bool, DbError> {
        let cmp = |o: &Operand, v: &Value| -> Result<Option<Ordering>, DbError> {
            Ok(compare(&self.operand(table, row, o)?, v))
        };
        let text = |o: &Operand| -> Result<Option<String>, DbError> {
            Ok(match self.operand(table, row, o)? {
                Value::Text(t) => Some(t),
                _ => None,
            })
        };
        Ok(match condition {
            Condition::True => true,
            Condition::Eq(o, v) => cmp(o, v)? == Some(Ordering::Equal),
            Condition::Ne(o, v) => matches!(cmp(o, v)?, Some(Ordering::Less | Ordering::Greater)),
            Condition::Lt(o, v) => cmp(o, v)? == Some(Ordering::Less),
            Condition::Le(o, v) => matches!(cmp(o, v)?, Some(Ordering::Less | Ordering::Equal)),
            Condition::Gt(o, v) => cmp(o, v)? == Some(Ordering::Greater),
            Condition::Ge(o, v) => matches!(cmp(o, v)?, Some(Ordering::Greater | Ordering::Equal)),
            Condition::IsNull(o) => self.operand(table, row, o)?.is_null(),
            Condition::NotNull(o) => !self.operand(table, row, o)?.is_null(),
            Condition::OneOf(o, values) => {
                let value = self.operand(table, row, o)?;
                values
                    .iter()
                    .any(|v| compare(&value, v) == Some(Ordering::Equal))
            }
            Condition::EqIgnoringAsciiCase(o, wanted) => {
                text(o)?.is_some_and(|t| t.eq_ignore_ascii_case(wanted))
            }
            Condition::ContainsIgnoringAsciiCase(o, wanted) => text(o)?.is_some_and(|t| {
                t.to_ascii_lowercase()
                    .contains(&wanted.to_ascii_lowercase())
            }),
            Condition::Exists(related) => !self.related(table, row, related)?.1.is_empty(),
            Condition::All(all) => {
                for c in all {
                    if !self.condition(table, row, c)? {
                        return Ok(false);
                    }
                }
                true
            }
            Condition::Any(any) => {
                for c in any {
                    if self.condition(table, row, c)? {
                        return Ok(true);
                    }
                }
                false
            }
            Condition::Not(c) => !self.condition(table, row, c)?,
        })
    }

    fn field(&self, table: &Table, row: &[Value], field: &Field) -> Result<Value, DbError> {
        Ok(match field {
            Field::Value(o) => self.operand(table, row, o)?,
            Field::Count(r) => Value::Int(self.related(table, row, r)?.1.len() as i64),
            Field::Exists(r) => Value::Bool(!self.related(table, row, r)?.1.is_empty()),
            Field::Max(r, column) => {
                let (other, rows) = self.related(table, row, r)?;
                let at = position(other, column)?;
                rows.iter()
                    .map(|r| &r[at])
                    .filter(|v| !v.is_null())
                    .max_by(|a, b| sort_compare(a, b))
                    .cloned()
                    .unwrap_or(Value::Null)
            }
            Field::Newest {
                related,
                by,
                column,
            } => {
                let (other, rows) = self.related(table, row, related)?;
                let (by, at) = (position(other, by)?, position(other, column)?);
                rows.iter()
                    .max_by(|a, b| sort_compare(&a[by], &b[by]))
                    .map(|r| r[at].clone())
                    .unwrap_or(Value::Null)
            }
        })
    }

    fn sort_key(&self, table: &Table, row: &[Value], by: &OrderBy) -> Result<Value, DbError> {
        match by {
            OrderBy::Operand(o) => self.operand(table, row, o),
            OrderBy::FirstOf(a, b) => {
                let first = self.operand(table, row, a)?;
                if first.is_null() {
                    self.operand(table, row, b)
                } else {
                    Ok(first)
                }
            }
        }
    }

    fn matching<'s>(
        &'s self,
        table: &Table,
        filter: &Condition,
    ) -> Result<Vec<&'s Vec<Value>>, DbError> {
        let mut rows = Vec::new();
        for row in self.rows(table.name) {
            if self.condition(table, row, filter)? {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError> {
        let table = self.table(select.table)?;
        self.selected(select, |_| true)?
            .into_iter()
            .map(|row| self.fields(table, select, row).map(Row::new))
            .collect()
    }

    fn fields(&self, table: &Table, select: &Select, row: &[Value]) -> Result<Vec<Value>, DbError> {
        select
            .fields
            .iter()
            .map(|f| self.field(table, row, f))
            .collect()
    }

    fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError> {
        let select = &search.select;
        let table = self.table(select.table)?;
        let terms = search::terms(&search.terms);
        let Some(column) = table.search else {
            return Err(DbError::Schema(format!(
                "table {} is not searchable",
                table.name
            )));
        };
        let at = position(table, column)?;
        let text = |row: &[Value]| match &row[at] {
            Value::Text(t) => t.clone(),
            _ => String::new(),
        };
        self.selected(select, |row| search::matches(&text(row), &terms))?
            .into_iter()
            .map(|row| {
                Ok(Hit {
                    row: Row::new(self.fields(table, select, row)?),
                    excerpt: search::excerpt(&text(row), &terms, &search.excerpt),
                })
            })
            .collect()
    }

    /// The rows a select picks, in its order, after its `after` and up to
    /// its limit, among those that also meet `keep`.
    fn selected<'s>(
        &'s self,
        select: &Select,
        keep: impl Fn(&[Value]) -> bool,
    ) -> Result<Vec<&'s Vec<Value>>, DbError> {
        let table = self.table(select.table)?;
        let mut keyed = Vec::new();
        for row in self.matching(table, &select.filter)? {
            if !keep(row) {
                continue;
            }
            let mut keys = Vec::new();
            for sort in &select.order {
                let (Sort::Asc(by) | Sort::Desc(by)) = sort;
                keys.push(self.sort_key(table, row, by)?);
            }
            keyed.push((keys, row));
        }
        let ordering = |a: &[Value], b: &[Value]| {
            for ((sort, x), y) in select.order.iter().zip(a).zip(b) {
                let o = sort_compare(x, y);
                let o = if matches!(sort, Sort::Desc(_)) {
                    o.reverse()
                } else {
                    o
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        };
        keyed.sort_by(|(a, _), (b, _)| ordering(a, b));
        if let Some(after) = &select.after {
            keyed.retain(|(keys, _)| ordering(keys, after) == Ordering::Greater);
        }
        if let Some(limit) = select.limit {
            keyed.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        Ok(keyed.into_iter().map(|(_, row)| row).collect())
    }

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError> {
        let table = self.table(group.table)?;
        let at = position(table, group.max)?;
        let mut groups: Vec<(Value, Vec<Value>, i64, Value)> = Vec::new();
        let mut index: HashMap<Value, usize> = HashMap::new();
        for row in self.matching(table, &group.filter)? {
            let by = self.operand(table, row, &group.by)?;
            let max = row[at].clone();
            match index.get(&by) {
                Some(&i) => {
                    let g = &mut groups[i];
                    g.2 += 1;
                    if sort_compare(&max, &g.3) == Ordering::Greater {
                        g.3 = max;
                    }
                }
                None => {
                    let also = group
                        .also
                        .iter()
                        .map(|o| self.operand(table, row, o))
                        .collect::<Result<Vec<_>, _>>()?;
                    index.insert(by.clone(), groups.len());
                    groups.push((by, also, 1, max));
                }
            }
        }
        groups.sort_by(|a, b| sort_compare(&b.3, &a.3));
        Ok(groups
            .into_iter()
            .map(|(by, also, count, max)| {
                let mut values = vec![by];
                values.extend(also);
                values.push(Value::Int(count));
                values.push(max);
                Row::new(values)
            })
            .collect())
    }
}

/// Checks every row against its table's columns and constraints, and every
/// reference: what must hold after any change.
fn check_all(state: &State) -> Result<(), DbError> {
    let view = View { state };
    for table in state.schema.tables() {
        let rows = view.rows(table.name);
        for row in rows {
            for (column, value) in table.columns.iter().zip(row) {
                check_cell(table, column, value)?;
                if let (Some(reference), false) = (&column.references, value.is_null()) {
                    if reference.except.as_ref() == Some(value) {
                        continue;
                    }
                    let other = view.table(reference.table)?;
                    let at = position(other, reference.column)?;
                    let found = view
                        .rows(reference.table)
                        .iter()
                        .any(|r| compare(&r[at], value) == Some(Ordering::Equal));
                    if !found {
                        return Err(refused(table, Some(column.name), ConstraintKind::Reference));
                    }
                }
            }
        }
        let key: Vec<usize> = table
            .primary_key
            .iter()
            .map(|c| position(table, c))
            .collect::<Result<_, _>>()?;
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            if !seen.insert(key.iter().map(|&i| row[i].clone()).collect::<Vec<_>>()) {
                return Err(refused(table, None, ConstraintKind::PrimaryKey));
            }
        }
        for (i, column) in table.columns.iter().enumerate() {
            if !column.unique {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            for row in rows {
                if !row[i].is_null() && !seen.insert(row[i].clone()) {
                    return Err(refused(table, Some(column.name), ConstraintKind::Unique));
                }
            }
        }
    }
    Ok(())
}

fn check_cell(table: &Table, column: &Column, value: &Value) -> Result<(), DbError> {
    if value.is_null() && !column.nullable {
        return Err(refused(table, Some(column.name), ConstraintKind::NotNull));
    }
    let positive = table
        .constraints
        .iter()
        .any(|c| matches!(c, Constraint::Positive(p) if *p == column.name));
    if positive && matches!(value, Value::Int(v) if *v <= 0) {
        return Err(refused(table, Some(column.name), ConstraintKind::Positive));
    }
    Ok(())
}

impl MemoryConnection {
    fn writable(&self) -> Result<(), DbError> {
        if self.write {
            Ok(())
        } else {
            Err(DbError::ReadOnly)
        }
    }

    /// Runs `change` on a copy of the state, and keeps the copy only if it
    /// succeeds and every constraint still holds.
    fn change<T>(
        &self,
        change: impl FnOnce(&mut State) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        self.writable()?;
        let mut state = self.state.borrow().clone();
        let value = change(&mut state)?;
        check_all(&state)?;
        *self.state.borrow_mut() = state;
        Ok(value)
    }

    fn insert_row(
        &self,
        insert: &Insert,
        existing: Existing,
    ) -> Result<(bool, Option<i64>), DbError> {
        check::insert(&self.state.borrow().schema, insert)?;
        self.change(|state| {
            let table = state.schema.require(insert.table)?.clone();
            let mut row = Vec::new();
            for column in &table.columns {
                let given = insert.values.iter().find(|(n, _)| *n == column.name);
                row.push(match given {
                    Some((_, v)) => v.clone(),
                    None => column.default.clone().unwrap_or(Value::Null),
                });
            }
            let data = state.data.entry(table.name).or_default();
            let key_at: Vec<usize> = table
                .primary_key
                .iter()
                .map(|c| position(&table, c))
                .collect::<Result<_, _>>()?;
            let same_key = |r: &Vec<Value>| key_at.iter().all(|&i| r[i] == row[i]);
            // A table keyed by its increasing key cannot clash: the key is
            // new. Any other primary key can.
            let present = match table.key() {
                Some(key) if table.primary_key == [key.name] => None,
                _ => data.rows.iter().position(same_key),
            };
            if let Some(at) = present {
                return match existing {
                    Existing::Refuse => Err(refused(&table, None, ConstraintKind::PrimaryKey)),
                    Existing::Keep => Ok((false, None)),
                    Existing::Replace => {
                        if table.constraints.contains(&Constraint::Immutable) {
                            return Err(refused(&table, None, ConstraintKind::Immutable));
                        }
                        let old = data.rows[at].clone();
                        for (name, value) in &insert.values {
                            let i = position(&table, name)?;
                            fixed(&table, name, &old[i], value)?;
                            data.rows[at][i] = value.clone();
                        }
                        Ok((true, None))
                    }
                };
            }
            let key = match table.key() {
                Some(key) => {
                    data.last_key += 1;
                    let i = position(&table, key.name)?;
                    row[i] = Value::Int(data.last_key);
                    Some(data.last_key)
                }
                None => None,
            };
            data.rows.push(row);
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

impl Connection for MemoryConnection {
    fn select(&self, select: &Select) -> Result<Vec<Row>, DbError> {
        let state = self.state.borrow();
        check::select(&state.schema, select)?;
        View { state: &state }.select(select)
    }

    fn count(&self, count: &Count) -> Result<i64, DbError> {
        let state = self.state.borrow();
        check::count(&state.schema, count)?;
        let view = View { state: &state };
        let table = view.table(count.table)?;
        Ok(view.matching(table, &count.filter)?.len() as i64)
    }

    fn group(&self, group: &Group) -> Result<Vec<Row>, DbError> {
        let state = self.state.borrow();
        check::group(&state.schema, group)?;
        View { state: &state }.group(group)
    }

    fn search(&self, search: &Search) -> Result<Vec<Hit>, DbError> {
        let state = self.state.borrow();
        check::search(&state.schema, search)?;
        View { state: &state }.search(search)
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
        check::update(&self.state.borrow().schema, update)?;
        self.change(|state| {
            let table = state.schema.require(update.table)?.clone();
            let hits: Vec<usize> = {
                let view = View { state };
                let mut hits = Vec::new();
                for (i, row) in view.rows(table.name).iter().enumerate() {
                    if view.condition(&table, row, &update.filter)? {
                        hits.push(i);
                    }
                }
                hits
            };
            if !hits.is_empty() && table.constraints.contains(&Constraint::Immutable) {
                return Err(refused(&table, None, ConstraintKind::Immutable));
            }
            let data = state.data.entry(table.name).or_default();
            for &i in &hits {
                for (name, assign) in &update.set {
                    let at = position(&table, name)?;
                    let old = data.rows[i][at].clone();
                    let new = match assign {
                        Assign::To(v) => v.clone(),
                        Assign::IfNull(v) if old.is_null() => v.clone(),
                        Assign::IfNull(_) => old.clone(),
                    };
                    fixed(&table, name, &old, &new)?;
                    data.rows[i][at] = new;
                }
            }
            Ok(hits.len() as u64)
        })
    }

    fn delete(&self, delete: &Delete) -> Result<u64, DbError> {
        check::delete(&self.state.borrow().schema, delete)?;
        self.change(|state| {
            let table = state.schema.require(delete.table)?.clone();
            let keep: Vec<bool> = {
                let view = View { state };
                view.rows(table.name)
                    .iter()
                    .map(|row| view.condition(&table, row, &delete.filter).map(|hit| !hit))
                    .collect::<Result<_, _>>()?
            };
            let removed = keep.iter().filter(|k| !**k).count();
            if removed > 0 {
                for (constraint, kind) in [
                    (Constraint::Immutable, ConstraintKind::Immutable),
                    (Constraint::NoDelete, ConstraintKind::NoDelete),
                ] {
                    if table.constraints.contains(&constraint) {
                        return Err(refused(&table, None, kind));
                    }
                }
            }
            let data = state.data.entry(table.name).or_default();
            let mut flags = keep.iter();
            data.rows.retain(|_| *flags.next().unwrap_or(&true));
            Ok(removed as u64)
        })
    }
}
