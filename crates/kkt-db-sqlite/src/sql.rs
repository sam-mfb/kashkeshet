//! kkt-db's questions and changes as SQL.
//!
//! Every value is a numbered parameter (`?1`, `?2`, …), so the text can be
//! put together in any order. Names come only from the schema, quoted.
//!
//! Logic is two-valued, as the contract says: each condition is written
//! so it is 0 or 1, never null. A comparison is `(x IS NOT NULL AND x = ?)`,
//! which SQLite still answers from an index on `x`.

use kkt_db::search::Search;
use kkt_db::{
    Assign, ColumnType, Condition, Count, DbError, Delete, Field, Group, Insert, Operand, OrderBy,
    Related, Schema, Select, Sort, Table, Update, Value, check,
};
use rusqlite::types::Value as SqlValue;

/// A name, quoted for SQL.
pub(crate) fn q(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The search index of `table`.
pub(crate) fn search_table(table: &str) -> String {
    format!("kkt_search_{table}")
}

pub(crate) fn sql_value(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Int(v) => SqlValue::Integer(*v),
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::Text(v) => SqlValue::Text(v.clone()),
        Value::Bytes(v) => SqlValue::Blob(v.clone()),
    }
}

/// A statement being written: its text grows piece by piece, and its
/// parameters are numbered as they are added.
pub(crate) struct Statement<'s> {
    schema: &'s Schema,
    pub(crate) params: Vec<SqlValue>,
    aliases: usize,
}

/// A finished statement, and the type of each value it returns.
pub(crate) struct Sql {
    pub(crate) text: String,
    pub(crate) params: Vec<SqlValue>,
    pub(crate) types: Vec<ColumnType>,
}

impl<'s> Statement<'s> {
    pub(crate) fn new(schema: &'s Schema) -> Statement<'s> {
        Statement {
            schema,
            params: Vec::new(),
            aliases: 0,
        }
    }

    fn alias(&mut self) -> String {
        self.aliases += 1;
        format!("t{}", self.aliases)
    }

    fn value(&mut self, value: &Value) -> String {
        self.params.push(sql_value(value));
        format!("?{}", self.params.len())
    }

    fn text_param(&mut self, text: String) -> String {
        self.params.push(SqlValue::Text(text));
        format!("?{}", self.params.len())
    }

    fn operand(
        &mut self,
        table: &Table,
        alias: &str,
        operand: &Operand,
    ) -> Result<String, DbError> {
        match operand {
            Operand::Column(name) => {
                table.require(name)?;
                Ok(format!("{alias}.{}", q(name)))
            }
            Operand::Through { reference, column } => {
                let Some(target) = table.require(reference)?.references.clone() else {
                    return Err(DbError::Schema(format!("{reference} is not a reference")));
                };
                let other = self.schema.require(target.table)?;
                other.require(column)?;
                let a = self.alias();
                let found = format!(
                    "(SELECT {a}.{} FROM {} {a} WHERE {a}.{} = {alias}.{})",
                    q(column),
                    q(target.table),
                    q(target.column),
                    q(reference)
                );
                Ok(match &target.except {
                    Some(except) => {
                        let p = self.value(except);
                        format!(
                            "(CASE WHEN {alias}.{} = {p} THEN NULL ELSE {found} END)",
                            q(reference)
                        )
                    }
                    None => found,
                })
            }
        }
    }

    /// `FROM … WHERE …` over the rows related to the row `alias` of
    /// `table`, with the alias the related rows have.
    fn related(
        &mut self,
        table: &Table,
        alias: &str,
        related: &Related,
    ) -> Result<(String, String), DbError> {
        let other = self.schema.require(related.table)?;
        other.require(related.column)?;
        table.require(related.to)?;
        let a = self.alias();
        let filter = self.condition(other, &a, &related.filter)?;
        Ok((
            a.clone(),
            format!(
                "FROM {} {a} WHERE {a}.{} = {alias}.{} AND {filter}",
                q(related.table),
                q(related.column),
                q(related.to)
            ),
        ))
    }

    pub(crate) fn condition(
        &mut self,
        table: &Table,
        alias: &str,
        condition: &Condition,
    ) -> Result<String, DbError> {
        let compare = |s: &mut Self, o: &Operand, op: &str, v: &Value| -> Result<String, DbError> {
            let x = s.operand(table, alias, o)?;
            let p = s.value(v);
            Ok(format!("({x} IS NOT NULL AND {x} {op} {p})"))
        };
        Ok(match condition {
            Condition::True => "1".into(),
            Condition::Eq(o, v) => compare(self, o, "=", v)?,
            Condition::Ne(o, v) => compare(self, o, "<>", v)?,
            Condition::Lt(o, v) => compare(self, o, "<", v)?,
            Condition::Le(o, v) => compare(self, o, "<=", v)?,
            Condition::Gt(o, v) => compare(self, o, ">", v)?,
            Condition::Ge(o, v) => compare(self, o, ">=", v)?,
            Condition::IsNull(o) => format!("({} IS NULL)", self.operand(table, alias, o)?),
            Condition::NotNull(o) => format!("({} IS NOT NULL)", self.operand(table, alias, o)?),
            Condition::OneOf(o, values) => {
                if values.is_empty() {
                    "0".into()
                } else {
                    let x = self.operand(table, alias, o)?;
                    match json_array(values) {
                        // A long list is one parameter, which SQLite reads
                        // as a table: no limit on how many.
                        Some(json) if values.len() > LONG_LIST => {
                            let p = self.text_param(json);
                            format!(
                                "({x} IS NOT NULL AND {x} IN (SELECT value FROM json_each({p})))"
                            )
                        }
                        _ => {
                            let list: Vec<String> = values.iter().map(|v| self.value(v)).collect();
                            format!("({x} IS NOT NULL AND {x} IN ({}))", list.join(", "))
                        }
                    }
                }
            }
            Condition::In(o, inner) => {
                let x = self.operand(table, alias, o)?;
                let (text, _) = self.select_text(inner)?;
                format!("({x} IS NOT NULL AND {x} IN ({text}))")
            }
            Condition::EqIgnoringAsciiCase(o, text) => {
                let x = self.operand(table, alias, o)?;
                let p = self.text_param(text.clone());
                format!("({x} IS NOT NULL AND {x} = {p} COLLATE NOCASE)")
            }
            Condition::ContainsIgnoringAsciiCase(o, text) => {
                // LIKE folds ASCII letters only; the pattern's own `%`, `_`
                // and `\` are escaped so they match themselves.
                let x = self.operand(table, alias, o)?;
                let p = self.text_param(like_pattern(text));
                format!("({x} IS NOT NULL AND {x} LIKE {p} ESCAPE '\\')")
            }
            Condition::Exists(related) => {
                let (_, from) = self.related(table, alias, related)?;
                format!("EXISTS (SELECT 1 {from})")
            }
            Condition::All(all) => {
                if all.is_empty() {
                    "1".into()
                } else {
                    let parts = all
                        .iter()
                        .map(|c| self.condition(table, alias, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    format!("({})", parts.join(" AND "))
                }
            }
            Condition::Any(any) => {
                if any.is_empty() {
                    "0".into()
                } else {
                    let parts = any
                        .iter()
                        .map(|c| self.condition(table, alias, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    format!("({})", parts.join(" OR "))
                }
            }
            Condition::Not(c) => format!("(NOT {})", self.condition(table, alias, c)?),
        })
    }

    fn field(
        &mut self,
        table: &Table,
        alias: &str,
        field: &Field,
    ) -> Result<(String, ColumnType), DbError> {
        Ok(match field {
            Field::Value(o) => (
                self.operand(table, alias, o)?,
                check::operand_type(self.schema, table, o)?,
            ),
            Field::Count(r) => {
                let (_, from) = self.related(table, alias, r)?;
                (format!("(SELECT count(*) {from})"), ColumnType::Int)
            }
            Field::Exists(r) => {
                let (_, from) = self.related(table, alias, r)?;
                (format!("EXISTS (SELECT 1 {from})"), ColumnType::Bool)
            }
            Field::Max(r, column) => {
                let ty = self.schema.require(r.table)?.require(column)?.ty;
                let (a, from) = self.related(table, alias, r)?;
                (format!("(SELECT max({a}.{}) {from})", q(column)), ty)
            }
            Field::Newest {
                related,
                by,
                column,
            } => {
                let other = self.schema.require(related.table)?;
                other.require(by)?;
                let ty = other.require(column)?.ty;
                let (a, from) = self.related(table, alias, related)?;
                (
                    format!(
                        "(SELECT {a}.{} {from} ORDER BY {a}.{} DESC LIMIT 1)",
                        q(column),
                        q(by)
                    ),
                    ty,
                )
            }
        })
    }

    fn order_key(&mut self, table: &Table, alias: &str, by: &OrderBy) -> Result<String, DbError> {
        match by {
            OrderBy::Operand(o) => self.operand(table, alias, o),
            OrderBy::FirstOf(a, b) => Ok(format!(
                "coalesce({}, {})",
                self.operand(table, alias, a)?,
                self.operand(table, alias, b)?
            )),
        }
    }

    /// `WHERE`, `ORDER BY` and `LIMIT` of a select, with `more` added to the
    /// conditions.
    fn tail(
        &mut self,
        table: &Table,
        alias: &str,
        select: &Select,
        more: Option<String>,
    ) -> Result<String, DbError> {
        let mut conditions = vec![self.condition(table, alias, &select.filter)?];
        conditions.extend(more);
        let mut keys = Vec::new();
        for sort in &select.order {
            let (Sort::Asc(by) | Sort::Desc(by)) = sort;
            keys.push((
                self.order_key(table, alias, by)?,
                matches!(sort, Sort::Desc(_)),
            ));
        }
        if let Some(after) = &select.after {
            // Rows after the values in the order: for some sort, equal on
            // every sort before it and past the value on it. Nulls come
            // first ascending, so a null is never past a value then; and
            // last descending, so it always is.
            let mut any = Vec::new();
            for i in 0..keys.len() {
                let mut all = Vec::new();
                for (j, (key, _)) in keys[..i].iter().enumerate() {
                    let p = self.value(&after[j]);
                    all.push(format!("({key} IS NOT NULL AND {key} = {p})"));
                }
                let (key, desc) = &keys[i];
                let p = self.value(&after[i]);
                all.push(if *desc {
                    format!("({key} IS NULL OR {key} < {p})")
                } else {
                    format!("({key} IS NOT NULL AND {key} > {p})")
                });
                any.push(format!("({})", all.join(" AND ")));
            }
            conditions.push(format!("({})", any.join(" OR ")));
        }
        let mut text = format!(" WHERE {}", conditions.join(" AND "));
        if !keys.is_empty() {
            let order: Vec<String> = keys
                .iter()
                .map(|(key, desc)| format!("{key} {}", if *desc { "DESC" } else { "ASC" }))
                .collect();
            text.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
        if let Some(limit) = select.limit {
            let p = self.value(&Value::Int(i64::try_from(limit).unwrap_or(i64::MAX)));
            text.push_str(&format!(" LIMIT {p}"));
        }
        Ok(text)
    }

    /// A select's text and the types of its fields, its parameters added
    /// to this statement's.
    fn select_text(&mut self, select: &Select) -> Result<(String, Vec<ColumnType>), DbError> {
        let table = self.schema.require(select.table)?;
        let alias = self.alias();
        let mut fields = Vec::new();
        let mut types = Vec::new();
        for field in &select.fields {
            let (text, ty) = self.field(table, &alias, field)?;
            fields.push(text);
            types.push(ty);
        }
        let tail = self.tail(table, &alias, select, None)?;
        Ok((
            format!(
                "SELECT {} FROM {} {alias}{tail}",
                fields.join(", "),
                q(table.name)
            ),
            types,
        ))
    }

    pub(crate) fn select(mut self, select: &Select) -> Result<Sql, DbError> {
        check::select(self.schema, select)?;
        let (text, types) = self.select_text(select)?;
        Ok(Sql {
            text,
            params: self.params,
            types,
        })
    }

    /// A search's select: its fields, then the searchable column, over the
    /// rows whose words match `query` (an FTS5 query of quoted words).
    pub(crate) fn search(mut self, search: &Search, query: String) -> Result<Sql, DbError> {
        check::search(self.schema, search)?;
        let select = &search.select;
        let table = self.schema.require(select.table)?;
        let Some(column) = table.search else {
            return Err(DbError::Schema(format!(
                "table {} is not searchable",
                table.name
            )));
        };
        let alias = self.alias();
        let mut fields = Vec::new();
        let mut types = Vec::new();
        for field in &select.fields {
            let (text, ty) = self.field(table, &alias, field)?;
            fields.push(text);
            types.push(ty);
        }
        fields.push(format!("{alias}.{}", q(column)));
        types.push(ColumnType::Text);
        let index = q(&search_table(table.name));
        let p = self.text_param(query);
        let matching =
            format!("{alias}.rowid IN (SELECT rowid FROM {index} WHERE {index} MATCH {p})");
        let tail = self.tail(table, &alias, select, Some(matching))?;
        Ok(Sql {
            text: format!(
                "SELECT {} FROM {} {alias}{tail}",
                fields.join(", "),
                q(table.name)
            ),
            params: self.params,
            types,
        })
    }

    pub(crate) fn count(mut self, count: &Count) -> Result<Sql, DbError> {
        check::count(self.schema, count)?;
        let table = self.schema.require(count.table)?;
        let alias = self.alias();
        let filter = self.condition(table, &alias, &count.filter)?;
        Ok(Sql {
            text: format!(
                "SELECT count(*) FROM {} {alias} WHERE {filter}",
                q(table.name)
            ),
            params: self.params,
            types: vec![ColumnType::Int],
        })
    }

    pub(crate) fn group(mut self, group: &Group) -> Result<Sql, DbError> {
        check::group(self.schema, group)?;
        let table = self.schema.require(group.table)?;
        let alias = self.alias();
        let by = self.operand(table, &alias, &group.by)?;
        let mut fields = vec![by.clone()];
        let mut types = vec![check::operand_type(self.schema, table, &group.by)?];
        for o in &group.also {
            fields.push(self.operand(table, &alias, o)?);
            types.push(check::operand_type(self.schema, table, o)?);
        }
        let max = format!("max({alias}.{})", q(group.max));
        fields.push("count(*)".into());
        types.push(ColumnType::Int);
        fields.push(max.clone());
        types.push(table.require(group.max)?.ty);
        let filter = self.condition(table, &alias, &group.filter)?;
        Ok(Sql {
            text: format!(
                "SELECT {} FROM {} {alias} WHERE {filter} GROUP BY {by} ORDER BY {max} DESC",
                fields.join(", "),
                q(table.name)
            ),
            params: self.params,
            types,
        })
    }

    /// `INSERT` with what to do when a row with the primary key exists:
    /// nothing (`ON CONFLICT … DO NOTHING`), or set the given columns.
    pub(crate) fn insert(
        mut self,
        insert: &Insert,
        on_conflict: OnConflict,
    ) -> Result<Sql, DbError> {
        check::insert(self.schema, insert)?;
        let table = self.schema.require(insert.table)?;
        let mut text = if insert.values.is_empty() {
            format!("INSERT INTO {} DEFAULT VALUES", q(table.name))
        } else {
            let columns: Vec<String> = insert.values.iter().map(|(c, _)| q(c)).collect();
            let values: Vec<String> = insert.values.iter().map(|(_, v)| self.value(v)).collect();
            format!(
                "INSERT INTO {} ({}) VALUES ({})",
                q(table.name),
                columns.join(", "),
                values.join(", ")
            )
        };
        let key: Vec<String> = table.primary_key.iter().map(|c| q(c)).collect();
        match on_conflict {
            OnConflict::Fail => {}
            OnConflict::Keep => {
                text.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", key.join(", ")))
            }
            OnConflict::Replace => {
                let set: Vec<String> = insert
                    .values
                    .iter()
                    .filter(|(c, _)| !table.primary_key.contains(c))
                    .map(|(c, _)| format!("{} = excluded.{}", q(c), q(c)))
                    .collect();
                if set.is_empty() {
                    text.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", key.join(", ")));
                } else {
                    text.push_str(&format!(
                        " ON CONFLICT ({}) DO UPDATE SET {}",
                        key.join(", "),
                        set.join(", ")
                    ));
                }
            }
        }
        Ok(Sql {
            text,
            params: self.params,
            types: Vec::new(),
        })
    }

    /// The rows an update or delete covers, by rowid.
    pub(crate) fn rowids(mut self, table: &str, filter: &Condition) -> Result<Sql, DbError> {
        let table = self.schema.require(table)?;
        let alias = q(table.name);
        let condition = self.condition(table, &alias, filter)?;
        Ok(Sql {
            text: format!("SELECT rowid FROM {alias} WHERE {condition}"),
            params: self.params,
            types: vec![ColumnType::Int],
        })
    }

    pub(crate) fn update(mut self, update: &Update) -> Result<Sql, DbError> {
        check::update(self.schema, update)?;
        let table = self.schema.require(update.table)?;
        // SQLite's UPDATE takes no alias: the table's own name serves.
        let alias = q(table.name);
        let set: Vec<String> = update
            .set
            .iter()
            .map(|(c, assign)| match assign {
                Assign::To(v) => format!("{} = {}", q(c), self.value(v)),
                Assign::IfNull(v) => format!("{} = coalesce({}, {})", q(c), q(c), self.value(v)),
            })
            .collect();
        let filter = self.condition(table, &alias, &update.filter)?;
        Ok(Sql {
            text: format!("UPDATE {alias} SET {} WHERE {filter}", set.join(", ")),
            params: self.params,
            types: Vec::new(),
        })
    }

    pub(crate) fn delete(mut self, delete: &Delete) -> Result<Sql, DbError> {
        check::delete(self.schema, delete)?;
        let table = self.schema.require(delete.table)?;
        let alias = q(table.name);
        let filter = self.condition(table, &alias, &delete.filter)?;
        Ok(Sql {
            text: format!("DELETE FROM {alias} WHERE {filter}"),
            params: self.params,
            types: Vec::new(),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum OnConflict {
    Fail,
    Keep,
    Replace,
}

/// How many values a `OneOf` binds one by one; a longer list is one JSON
/// parameter.
const LONG_LIST: usize = 100;

/// The values as a JSON array for `json_each`, which gives back integers
/// and text as SQLite stores them; `None` if any is of another kind.
fn json_array(values: &[Value]) -> Option<String> {
    let mut out = String::from("[");
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match value {
            Value::Int(v) => out.push_str(&v.to_string()),
            Value::Bool(v) => out.push_str(if *v { "1" } else { "0" }),
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
            Value::Null | Value::Bytes(_) => return None,
        }
    }
    out.push(']');
    Some(out)
}

/// `%text%`, with `%`, `_` and `\` escaped for `LIKE … ESCAPE '\'`.
fn like_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('%');
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// An FTS5 query for the terms' words: each term a quoted phrase. Words
/// hold only word characters, never a quote.
pub(crate) fn match_query(terms: &[Vec<String>]) -> String {
    terms
        .iter()
        .map(|words| format!("\"{}\"", words.join(" ")))
        .collect::<Vec<_>>()
        .join(" ")
}
