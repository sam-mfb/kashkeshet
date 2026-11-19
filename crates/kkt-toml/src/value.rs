//! A TOML text's tree of values.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

/// A TOML table: its keys in name order, each with its value.
pub type Table = BTreeMap<String, Value>;

/// One value in a TOML text. It prints as TOML writes it: text in quotes,
/// a list in brackets, a table inline. Turned into JSON (it is
/// serializable), each value is the JSON value of the same kind, and a
/// date-time is its text.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    /// A date, a time or both, as TOML writes it.
    DateTime(String),
    List(Vec<Value>),
    Table(Table),
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }
}

/// Written by the library from the same value, so that it reads exactly
/// as the library writes it.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        to_library(self).fmt(f)
    }
}

pub(crate) fn table_from_library(table: toml::Table) -> Table {
    table
        .into_iter()
        .map(|(key, value)| (key, from_library(value)))
        .collect()
}

fn from_library(value: toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::Text(text),
        toml::Value::Integer(n) => Value::Integer(n),
        toml::Value::Float(x) => Value::Float(x),
        toml::Value::Boolean(b) => Value::Boolean(b),
        toml::Value::Datetime(when) => Value::DateTime(when.to_string()),
        toml::Value::Array(items) => Value::List(items.into_iter().map(from_library).collect()),
        toml::Value::Table(table) => Value::Table(table_from_library(table)),
    }
}

fn to_library(value: &Value) -> toml::Value {
    match value {
        Value::Text(text) => toml::Value::String(text.clone()),
        Value::Integer(n) => toml::Value::Integer(*n),
        Value::Float(x) => toml::Value::Float(*x),
        Value::Boolean(b) => toml::Value::Boolean(*b),
        // A DateTime is made only from the library's own date-time, written
        // as it writes it, so it reads back. Should one not, its text is
        // shown as text.
        Value::DateTime(text) => text
            .parse()
            .map_or_else(|_| toml::Value::String(text.clone()), toml::Value::Datetime),
        Value::List(items) => toml::Value::Array(items.iter().map(to_library).collect()),
        Value::Table(table) => toml::Value::Table(
            table
                .iter()
                .map(|(key, value)| (key.clone(), to_library(value)))
                .collect(),
        ),
    }
}
