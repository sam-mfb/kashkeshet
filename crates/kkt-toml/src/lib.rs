//! TOML text, in kkt's terms. The one crate that uses the `toml` and
//! `toml_edit` libraries; no type of theirs is in its public items.
//!
//! It offers three things:
//! - [`parse`]: a text's tree of values ([`Table`], [`Value`]), its keys in
//!   name order;
//! - [`read`] and [`write`]: a typed record read from text, or written as
//!   text;
//! - [`Document`]: a text open for editing, given back with its comments and
//!   layout kept.
//!
//! A text that cannot be read gives a [`ParseError`], with the parser's
//! reason and, when it knows it, the [`Place`] in the text. Each caller
//! words it for its own users. It is pure logic: callers depend on it
//! directly.

mod document;
mod error;
mod value;

pub use document::{Document, EditError, Found, IfMissing, ListMut, TableMut};
pub use error::{ParseError, Place, WriteError};
pub use value::{Table, Value};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The tree of values `text` holds, its keys in name order.
pub fn parse(text: &str) -> Result<Table, ParseError> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        ParseError::new(text, e.message(), e.span(), e.to_string())
    })?;
    Ok(value::table_from_library(table))
}

/// The record of type `T` that `text` holds.
pub fn read<T: DeserializeOwned>(text: &str) -> Result<T, ParseError> {
    toml::from_str(text)
        .map_err(|e: toml::de::Error| ParseError::new(text, e.message(), e.span(), e.to_string()))
}

/// `record` written as TOML text.
pub fn write<T: Serialize>(record: &T) -> Result<String, WriteError> {
    toml::to_string(record).map_err(|e| WriteError::new(e.to_string()))
}

#[cfg(test)]
mod tests;
