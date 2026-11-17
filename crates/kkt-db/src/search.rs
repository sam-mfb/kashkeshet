//! What a caller asks a search for, and what it finds; and the words a
//! backend indexes and matches. How text splits into words, how terms
//! match and how excerpts are drawn is kkt-search's, applied here by
//! kkt-db, the same for every backend. A backend is handed [`Words`] to
//! index for each row it writes, and [`Terms`] to match; it never splits
//! text. Both can only be made by kkt-db.

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
    /// Raw terms, as a person typed them. kkt-db splits each into words
    /// before the backend sees them.
    pub terms: Vec<String>,
    pub excerpt: Excerpt,
}

/// One row a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: Row,
    pub excerpt: String,
}

/// The words of one row's searchable text, in order: what a backend
/// indexes for the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words(Vec<String>);

impl Words {
    pub(crate) fn of(text: &str) -> Words {
        Words(
            kkt_search::words(text)
                .into_iter()
                .map(|w| w.text)
                .collect(),
        )
    }

    /// The words, in order.
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    /// Whether these words contain every one of `terms`: each term's words
    /// together and in order. No terms match nothing. A backend with no
    /// index of its own matches with this.
    pub fn matches(&self, terms: &Terms) -> bool {
        !terms.0.is_empty()
            && terms.0.iter().all(|term| {
                !term.is_empty() && self.0.windows(term.len()).any(|w| w == term.as_slice())
            })
    }
}

/// A search's terms, each as its words; terms with no words are left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terms(Vec<Vec<String>>);

impl Terms {
    pub(crate) fn of(raw: &[String]) -> Terms {
        Terms(kkt_search::terms(raw))
    }

    /// Each term's words, in order.
    pub fn as_slice(&self) -> &[Vec<String>] {
        &self.0
    }

    /// Whether there are no terms, which match nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// How a write changes the search index of the rows it writes, worked out
/// by kkt-db from the write's values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reindex {
    /// The write leaves the searchable column alone, or the table has
    /// none: rows already there keep their entries, and a row it adds has
    /// none (its searchable column is null).
    Keep,
    /// Every row the write adds or changes gets these words; `None` when
    /// the searchable column becomes null, which has no entry.
    Set(Option<Words>),
    /// Rows whose searchable column was null get these words, as the
    /// column gets its value only in those rows ([`crate::Assign::IfNull`]).
    SetIfNull(Words),
}

/// Turns stored text into [`Words`], for a backend building the index over
/// rows already in a table: when [`crate::Step::AddSearch`] makes a column
/// searchable, and when a backend adopts a database from before kkt-db.
/// There, and only there, a backend reads the searchable text itself; this
/// is how it gets that text's words.
#[derive(Clone, Copy, Debug, Default)]
pub struct Indexer(());

impl Indexer {
    pub fn new() -> Indexer {
        Indexer(())
    }

    pub fn words(&self, text: &str) -> Words {
        Words::of(text)
    }
}

/// One row a backend's search found: its fields, and its searchable text,
/// from which kkt-db cuts the excerpt (empty when the text is null).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub row: Row,
    pub text: String,
}

/// The excerpt of `text` around `terms`, drawn as `excerpt` says.
pub(crate) fn excerpt_of(text: &str, terms: &Terms, excerpt: &Excerpt) -> String {
    kkt_search::excerpt(
        text,
        &terms.0,
        excerpt.open,
        excerpt.close,
        excerpt.ellipsis,
        excerpt.words,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(raw: &[&str]) -> Terms {
        Terms::of(&raw.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn words_match_as_kkt_search_matches_text() {
        let texts = [
            "Hello, World! The quick brown fox.",
            "Ünïcode Straße and café au lait",
            "",
            "one two three two one",
        ];
        let searches: [&[&str]; 7] = [
            &["hello"],
            &["WORLD", "fox"],
            &["quick brown"],
            &["brown quick"],
            &["strasse", "cafe"],
            &["!!!"],
            &["two one", "three"],
        ];
        for text in texts {
            for raw in searches {
                let t = terms(raw);
                assert_eq!(
                    Words::of(text).matches(&t),
                    kkt_search::matches(text, t.as_slice()),
                    "{text:?} {raw:?}"
                );
            }
        }
    }

    #[test]
    fn no_terms_match_nothing() {
        assert!(terms(&["", "  ", "!"]).is_empty());
        assert!(!Words::of("anything").matches(&terms(&[])));
    }

    #[test]
    fn the_indexer_gives_the_words_kkt_search_gives() {
        let words: Vec<String> = kkt_search::words("Mixed CASE, accents: é")
            .into_iter()
            .map(|w| w.text)
            .collect();
        assert_eq!(
            Indexer::new().words("Mixed CASE, accents: é").as_slice(),
            words
        );
    }
}
