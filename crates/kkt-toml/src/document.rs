//! A TOML text open for editing, with a few operations: find what is at a
//! key path, open a table or a list there, and set,
//! remove or add text. Whatever is not changed keeps its comments and
//! layout.
//!
//! "A table" means a `[section]` or an inline `{ ... }` table alike.

use crate::error::ParseError;
use toml_edit::{Array, DocumentMut, Item, TableLike};

/// A TOML text open for editing.
#[derive(Clone, Debug)]
pub struct Document(DocumentMut);

/// What a document holds at a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A table, a section or inline.
    Table,
    /// Text in quotes, without its quotes.
    Text(String),
    /// Any other value.
    Other,
}

/// What opening a table does when a key on its path is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfMissing {
    /// Add an empty table there.
    Add,
    /// Refuse, with [`EditError::Missing`].
    Refuse,
}

/// Why a document could not be changed at a key. `path` is the keys from
/// the top of the document to the one at fault, that one included.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("{} is missing", .path.join("."))]
    Missing { path: Vec<String> },
    #[error("{} is not a table", .path.join("."))]
    NotATable { path: Vec<String> },
    #[error("{} is not a list", .path.join("."))]
    NotAList { path: Vec<String> },
}

/// A table in a document, to change.
pub struct TableMut<'a> {
    table: &'a mut dyn TableLike,
    path: Vec<String>,
}

/// A list in a document, to change.
pub struct ListMut<'a>(&'a mut Array);

impl Document {
    /// `text`, open for editing.
    pub fn parse(text: &str) -> Result<Document, ParseError> {
        text.parse::<DocumentMut>()
            .map(Document)
            .map_err(|e| ParseError::new(text, e.message(), e.span(), e.to_string()))
    }

    /// The document's text, with every change made and everything else as
    /// it was.
    pub fn text(&self) -> String {
        self.0.to_string()
    }

    /// What is at `path`, a key path from the top of the document, looked
    /// up through tables. `None` when a key on the way is missing or not a
    /// table.
    pub fn find(&self, path: &[&str]) -> Option<Found> {
        let (last, tables) = path.split_last()?;
        let mut table: &dyn TableLike = self.0.as_table();
        for key in tables {
            table = table.get(key)?.as_table_like()?;
        }
        table.get(last).map(found)
    }

    /// The table at `path`, from the top of the document; the empty path is
    /// the document's top. A missing key on the way is added or refused, as
    /// `if_missing` says; a key that holds something other than a table is
    /// refused.
    pub fn table_mut(
        &mut self,
        path: &[&str],
        if_missing: IfMissing,
    ) -> Result<TableMut<'_>, EditError> {
        TableMut {
            table: self.0.as_table_mut(),
            path: Vec::new(),
        }
        .into_table(path, if_missing)
    }

    /// The list at `key`, at the top of the document, added empty if it is
    /// missing.
    pub fn list_mut(&mut self, key: &str) -> Result<ListMut<'_>, EditError> {
        self.0
            .entry(key)
            .or_insert_with(|| toml_edit::value(Array::new()))
            .as_array_mut()
            .map(ListMut)
            .ok_or_else(|| EditError::NotAList {
                path: vec![key.to_string()],
            })
    }
}

impl<'a> TableMut<'a> {
    /// What is at `key` in this table.
    pub fn find(&self, key: &str) -> Option<Found> {
        self.table.get(key).map(found)
    }

    /// The value at `key` as the text writes it, with the spaces and any
    /// comment that follow it on its line. `None` when the key is missing
    /// or holds a `[section]`, which is not a value.
    pub fn written(&self, key: &str) -> Option<String> {
        self.table
            .get(key)
            .and_then(Item::as_value)
            .map(|value| value.to_string())
    }

    /// The keys in this table, in the order the text has them.
    pub fn keys(&self) -> Vec<String> {
        self.table.iter().map(|(key, _)| key.to_string()).collect()
    }

    /// Sets `key` to `text`, a text value, adding the key if it is missing.
    pub fn set_text(&mut self, key: &str, text: &str) {
        self.table.insert(key, toml_edit::value(text));
    }

    /// Removes `key`, if the table has it.
    pub fn remove(&mut self, key: &str) {
        self.table.remove(key);
    }

    /// The table at `path` inside this one, as [`Document::table_mut`]
    /// opens one.
    pub fn table_mut(
        &mut self,
        path: &[&str],
        if_missing: IfMissing,
    ) -> Result<TableMut<'_>, EditError> {
        TableMut {
            table: &mut *self.table,
            path: self.path.clone(),
        }
        .into_table(path, if_missing)
    }

    fn into_table(self, path: &[&str], if_missing: IfMissing) -> Result<TableMut<'a>, EditError> {
        let TableMut {
            mut table,
            path: mut here,
        } = self;
        for key in path {
            here.push((*key).to_string());
            let item = match if_missing {
                IfMissing::Add => table
                    .entry(key)
                    .or_insert(Item::Table(toml_edit::Table::new())),
                IfMissing::Refuse => match table.get_mut(key) {
                    Some(item) => item,
                    None => return Err(EditError::Missing { path: here }),
                },
            };
            table = match item.as_table_like_mut() {
                Some(next) => next,
                None => return Err(EditError::NotATable { path: here }),
            };
        }
        Ok(TableMut { table, path: here })
    }
}

impl ListMut<'_> {
    /// Whether the list holds `text` as one of its values.
    pub fn contains_text(&self, text: &str) -> bool {
        self.0.iter().any(|value| value.as_str() == Some(text))
    }

    /// Adds `text` at the end of the list.
    pub fn push_text(&mut self, text: &str) {
        self.0.push(text);
    }
}

fn found(item: &Item) -> Found {
    if let Some(text) = item.as_str() {
        Found::Text(text.to_string())
    } else if item.is_table_like() {
        Found::Table
    } else {
        Found::Other
    }
}
