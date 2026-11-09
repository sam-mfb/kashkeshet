//! Values in columns, and rows as a query returns them.

use crate::DbError;

/// One value in a column. Each column type has its own kind of value, and
/// any column declared nullable may hold [`Value::Null`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Null,
    /// A 64-bit integer, in an `Int` or `Key` column.
    Int(i64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The name of the value's kind, for errors.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Int(_) => "integer",
            Value::Text(_) => "text",
            Value::Bool(_) => "bool",
            Value::Bytes(_) => "bytes",
        }
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Value {
        Value::Int(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Value {
        Value::Text(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Value {
        Value::Text(v)
    }
}

impl From<&String> for Value {
    fn from(v: &String) -> Value {
        Value::Text(v.clone())
    }
}

impl From<&[u8]> for Value {
    fn from(v: &[u8]) -> Value {
        Value::Bytes(v.to_vec())
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Value {
        Value::Bytes(v)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        v.map_or(Value::Null, Into::into)
    }
}

/// A Rust type a [`Value`] can be read as.
pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Result<Self, DbError>;
}

fn wrong(expected: &'static str, found: &Value) -> DbError {
    DbError::Type {
        expected,
        found: found.kind(),
    }
}

impl FromValue for i64 {
    fn from_value(value: &Value) -> Result<i64, DbError> {
        match value {
            Value::Int(v) => Ok(*v),
            other => Err(wrong("integer", other)),
        }
    }
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<bool, DbError> {
        match value {
            Value::Bool(v) => Ok(*v),
            other => Err(wrong("bool", other)),
        }
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<String, DbError> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            other => Err(wrong("text", other)),
        }
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> Result<Vec<u8>, DbError> {
        match value {
            Value::Bytes(v) => Ok(v.clone()),
            other => Err(wrong("bytes", other)),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Option<T>, DbError> {
        match value {
            Value::Null => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }
}

/// One row of a query's answer: a value for each field the query asked
/// for, in the order it asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row(Vec<Value>);

impl Row {
    pub fn new(values: Vec<Value>) -> Row {
        Row(values)
    }

    /// The `i`th value, read as `T`. Fails if there is no `i`th value or it
    /// is of another kind; a null reads only as an `Option`.
    pub fn get<T: FromValue>(&self, i: usize) -> Result<T, DbError> {
        match self.0.get(i) {
            Some(value) => T::from_value(value),
            None => Err(DbError::Schema(format!(
                "the row has {} values, not {}",
                self.0.len(),
                i + 1
            ))),
        }
    }

    pub fn values(&self) -> &[Value] {
        &self.0
    }

    pub fn into_values(self) -> Vec<Value> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_reads_each_kind_and_refuses_the_wrong_one() {
        let row = Row::new(vec![
            Value::Int(7),
            Value::from("seven"),
            Value::Bool(true),
            Value::Bytes(vec![7]),
            Value::Null,
        ]);
        assert_eq!(row.get::<i64>(0).unwrap(), 7);
        assert_eq!(row.get::<String>(1).unwrap(), "seven");
        assert!(row.get::<bool>(2).unwrap());
        assert_eq!(row.get::<Vec<u8>>(3).unwrap(), vec![7]);
        assert_eq!(row.get::<Option<i64>>(4).unwrap(), None);
        assert_eq!(row.get::<Option<i64>>(0).unwrap(), Some(7));
        assert!(matches!(
            row.get::<String>(0),
            Err(DbError::Type {
                expected: "text",
                found: "integer"
            })
        ));
        assert!(matches!(row.get::<i64>(4), Err(DbError::Type { .. })));
        assert!(matches!(row.get::<i64>(5), Err(DbError::Schema(_))));
    }
}
