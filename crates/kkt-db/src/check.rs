//! Checks that a question or change fits the schema: its tables and
//! columns exist, references are followed only through reference
//! columns, and values are of their columns' types. Every backend runs
//! these first, so a mistake in a store is the same error everywhere.

use crate::{
    Assign, ColumnType, Condition, Count, DbError, Delete, Field, Group, Insert, Operand, OrderBy,
    Related, Schema, Select, Sort, Table, Update, Value,
};

fn fail<T>(what: String) -> Result<T, DbError> {
    Err(DbError::Schema(what))
}

/// The type of `operand` read from a row of `table`.
pub fn operand_type(
    schema: &Schema,
    table: &Table,
    operand: &Operand,
) -> Result<ColumnType, DbError> {
    match operand {
        Operand::Column(name) => Ok(table.require(name)?.ty),
        Operand::Through { reference, column } => {
            let Some(target) = &table.require(reference)?.references else {
                return fail(format!("{}.{reference} is not a reference", table.name));
            };
            let target = schema.require(target.table)?;
            Ok(target.require(column)?.ty)
        }
    }
}

fn same(a: ColumnType, b: ColumnType) -> bool {
    let int = |t| matches!(t, ColumnType::Int | ColumnType::Key);
    a == b || (int(a) && int(b))
}

fn check_value(ty: ColumnType, value: &Value, what: &str) -> Result<(), DbError> {
    if value.is_null() {
        return fail(format!(
            "{what} is compared with null, which is never equal: use IsNull"
        ));
    }
    if !ty.admits(value) {
        return fail(format!("{what} is a {ty:?}, not a {}", value.kind()));
    }
    Ok(())
}

fn related<'s>(schema: &'s Schema, table: &Table, related: &Related) -> Result<&'s Table, DbError> {
    let other = schema.require(related.table)?;
    let theirs = other.require(related.column)?.ty;
    let ours = table.require(related.to)?.ty;
    if !same(theirs, ours) {
        return fail(format!(
            "{}.{} and {}.{} are of different types",
            other.name, related.column, table.name, related.to
        ));
    }
    condition(schema, other, &related.filter)?;
    Ok(other)
}

/// Checks a condition on rows of `table`.
pub fn condition(schema: &Schema, table: &Table, condition: &Condition) -> Result<(), DbError> {
    let typed = |operand: &Operand| operand_type(schema, table, operand);
    let text = |operand: &Operand| match typed(operand)? {
        ColumnType::Text => Ok(()),
        other => fail(format!("{operand:?} is a {other:?}, not text")),
    };
    match condition {
        Condition::True => Ok(()),
        Condition::Eq(o, v)
        | Condition::Ne(o, v)
        | Condition::Lt(o, v)
        | Condition::Le(o, v)
        | Condition::Gt(o, v)
        | Condition::Ge(o, v) => check_value(typed(o)?, v, &format!("{o:?}")),
        Condition::IsNull(o) | Condition::NotNull(o) => typed(o).map(drop),
        Condition::OneOf(o, values) => {
            let ty = typed(o)?;
            values
                .iter()
                .try_for_each(|v| check_value(ty, v, &format!("{o:?}")))
        }
        Condition::In(o, inner) => {
            let ty = typed(o)?;
            select(schema, inner)?;
            let [field] = inner.fields.as_slice() else {
                return fail(format!(
                    "{o:?} is compared with a select of {} fields",
                    inner.fields.len()
                ));
            };
            let other = schema.require(inner.table)?;
            let theirs = match field {
                Field::Value(operand) => operand_type(schema, other, operand)?,
                _ => return fail("a select compared with must return a column".into()),
            };
            if !same(ty, theirs) {
                return fail(format!(
                    "{o:?} is a {ty:?}, and the select returns {theirs:?}"
                ));
            }
            Ok(())
        }
        Condition::EqIgnoringAsciiCase(o, _) | Condition::ContainsIgnoringAsciiCase(o, _) => {
            text(o)
        }
        Condition::Exists(r) => related(schema, table, r).map(drop),
        Condition::All(all) | Condition::Any(all) => all
            .iter()
            .try_for_each(|c| self::condition(schema, table, c)),
        Condition::Not(c) => self::condition(schema, table, c),
    }
}

fn order_type(schema: &Schema, table: &Table, by: &OrderBy) -> Result<ColumnType, DbError> {
    match by {
        OrderBy::Operand(o) => operand_type(schema, table, o),
        OrderBy::FirstOf(a, b) => {
            let (ta, tb) = (
                operand_type(schema, table, a)?,
                operand_type(schema, table, b)?,
            );
            if !same(ta, tb) {
                return fail(format!("{a:?} and {b:?} are of different types"));
            }
            Ok(ta)
        }
    }
}

pub fn select(schema: &Schema, select: &Select) -> Result<(), DbError> {
    let table = schema.require(select.table)?;
    for field in &select.fields {
        match field {
            Field::Value(o) => {
                operand_type(schema, table, o)?;
            }
            Field::Count(r) | Field::Exists(r) => {
                related(schema, table, r)?;
            }
            Field::Max(r, column) => {
                related(schema, table, r)?.require(column)?;
            }
            Field::Newest {
                related: r,
                by,
                column,
            } => {
                let other = related(schema, table, r)?;
                other.require(by)?;
                other.require(column)?;
            }
        }
    }
    condition(schema, table, &select.filter)?;
    let mut types = Vec::new();
    for sort in &select.order {
        let (Sort::Asc(by) | Sort::Desc(by)) = sort;
        types.push(order_type(schema, table, by)?);
    }
    if let Some(after) = &select.after {
        if after.len() != select.order.len() {
            return fail(format!(
                "`after` has {} values for {} sorts",
                after.len(),
                select.order.len()
            ));
        }
        for (ty, value) in types.iter().zip(after) {
            check_value(*ty, value, "an `after` value")?;
        }
    }
    Ok(())
}

pub fn search(schema: &Schema, search: &crate::search::Search) -> Result<(), DbError> {
    select(schema, &search.select)?;
    if schema.require(search.select.table)?.search.is_none() {
        return fail(format!("table {} is not searchable", search.select.table));
    }
    if search.excerpt.words == 0 {
        return fail("an excerpt of no words".into());
    }
    Ok(())
}

pub fn count(schema: &Schema, count: &Count) -> Result<(), DbError> {
    condition(schema, schema.require(count.table)?, &count.filter)
}

pub fn group(schema: &Schema, group: &Group) -> Result<(), DbError> {
    let table = schema.require(group.table)?;
    operand_type(schema, table, &group.by)?;
    for o in &group.also {
        operand_type(schema, table, o)?;
    }
    table.require(group.max)?;
    condition(schema, table, &group.filter)
}

fn values(table: &Table, values: &[(&'static str, Value)]) -> Result<(), DbError> {
    for (i, (name, value)) in values.iter().enumerate() {
        if values[..i].iter().any(|(n, _)| n == name) {
            return fail(format!("{}.{name} is given twice", table.name));
        }
        let column = table.require(name)?;
        if column.ty == ColumnType::Key {
            return fail(format!("{}.{name} is assigned by the database", table.name));
        }
        if !column.ty.admits(value) {
            return fail(format!(
                "{}.{name} is a {:?}, not a {}",
                table.name,
                column.ty,
                value.kind()
            ));
        }
    }
    Ok(())
}

pub fn insert(schema: &Schema, insert: &Insert) -> Result<(), DbError> {
    values(schema.require(insert.table)?, &insert.values)
}

pub fn update(schema: &Schema, update: &Update) -> Result<(), DbError> {
    let table = schema.require(update.table)?;
    let set: Vec<_> = update
        .set
        .iter()
        .map(|(name, set)| match set {
            Assign::To(v) | Assign::IfNull(v) => (*name, v.clone()),
        })
        .collect();
    values(table, &set)?;
    condition(schema, table, &update.filter)
}

pub fn delete(schema: &Schema, delete: &Delete) -> Result<(), DbError> {
    condition(schema, schema.require(delete.table)?, &delete.filter)
}
