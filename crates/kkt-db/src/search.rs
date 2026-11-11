//! What a caller asks a search for, and what it finds. How text splits
//! into words, how terms match and how excerpts are drawn is kkt-search's,
//! the same for every backend: a backend indexes kkt-search's words of the
//! searchable column and never splits text itself.

use crate::{Row, Select};

/// How an excerpt is drawn: the marks around each match, the mark where
/// the window cuts the text, and how many words the window holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    pub open: &'static str,
    pub close: &'static str,
    pub ellipsis: &'static str,
    pub words: usize,
}

/// Rows of a table with a search index ([`crate::Step::AddSearch`]) that
/// match every term, with an excerpt of the searchable column for each.
/// The [`Select`] gives the table, the fields, and the filter, order and
/// limit, which apply as they do to a select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub select: Select,
    /// Raw terms, as a person typed them: each is split into words by the
    /// backend, with kkt-search.
    pub terms: Vec<String>,
    pub excerpt: Excerpt,
}

/// One row a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: Row,
    pub excerpt: String,
}
