//! Search: words, terms, matching and excerpts, the same for every
//! backend.
//!
//! **Words.** A word is a run of word characters, folded: [`table`]
//! says which characters are word characters and how each folds. It is
//! SQLite FTS5's `unicode61` rule with its defaults, written down as ours:
//! letters, digits and private-use characters (as Unicode 6.1 classed
//! them) make words; case is folded; diacritics are removed from Latin
//! letters; everything else separates words. So `PDF.js` is the words
//! `pdf` and `js`.
//!
//! **Terms.** A search gives terms; each is split into words, and a
//! message matches a term where those words appear together and in order.
//! A row matches a search when it matches every term. A term with no words
//! (`--`, an emoji) is dropped, and a search left with no terms matches
//! nothing.
//!
//! **Excerpts.** [`excerpt`] chooses a window of the text around the
//! matches and marks them, as FTS5's `snippet` does.
//!
//! A backend indexes [`words`] of the searchable column and matches terms
//! against them; it never splits text itself.

mod table;

use crate::{Row, Select};

/// How a character takes part in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// A word character that adds nothing to the word: a combining
    /// diacritic, removed.
    Drop,
    /// A word character, folded to the character this far from it.
    Keep(i32),
}

fn class(c: char) -> Option<Class> {
    let cp = c as u32;
    let i = table::RANGES.partition_point(|&(_, last, _)| last < cp);
    match table::RANGES.get(i) {
        Some(&(first, _, class)) if first <= cp => Some(class),
        _ => None,
    }
}

/// One word of a text: its folded form, and where it stands in the text
/// (byte offsets, `start..end`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// The words of `text`, in order.
pub fn words(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current: Option<Word> = None;
    for (at, c) in text.char_indices() {
        match class(c) {
            Some(class) => {
                let word = current.get_or_insert_with(|| Word {
                    text: String::new(),
                    start: at,
                    end: at,
                });
                if let Class::Keep(by) = class {
                    let folded = (c as u32).checked_add_signed(by).and_then(char::from_u32);
                    word.text.push(folded.unwrap_or(c));
                }
                word.end = at + c.len_utf8();
            }
            None => {
                if let Some(word) = current.take().filter(|w| !w.text.is_empty()) {
                    words.push(word);
                }
            }
        }
    }
    if let Some(word) = current.filter(|w| !w.text.is_empty()) {
        words.push(word);
    }
    words
}

/// The terms of a search, each as its words: terms with no words dropped.
pub fn terms<S: AsRef<str>>(terms: &[S]) -> Vec<Vec<String>> {
    terms
        .iter()
        .map(|t| {
            words(t.as_ref())
                .into_iter()
                .map(|w| w.text)
                .collect::<Vec<_>>()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// Where `term` appears in `words`: the position of its first word, each
/// time.
fn places(words: &[String], term: &[String]) -> Vec<usize> {
    if term.is_empty() || term.len() > words.len() {
        return Vec::new();
    }
    (0..=words.len() - term.len())
        .filter(|&i| words[i..i + term.len()] == *term)
        .collect()
}

/// Whether `text` matches every one of `terms` (as [`terms`] gives them).
/// No terms match nothing.
pub fn matches(text: &str, terms: &[Vec<String>]) -> bool {
    let words: Vec<String> = words(text).into_iter().map(|w| w.text).collect();
    !terms.is_empty() && terms.iter().all(|t| !places(&words, t).is_empty())
}

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
    /// Raw terms, as a person typed them: each is split into words here.
    pub terms: Vec<String>,
    pub excerpt: Excerpt,
}

/// One row a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: Row,
    pub excerpt: String,
}

/// One appearance of a term: which term, and the position of its first
/// word.
#[derive(Clone, Copy, Debug)]
struct Place {
    term: usize,
    at: usize,
}

/// How well the window starting at `start` covers the places, as FTS5
/// scores it: 1000 for each term's first place in the window and 1 for
/// each later one. Also where the window would start if centred on the
/// places it covers, kept within the text.
fn score(
    places: &[Place],
    terms: &[Vec<String>],
    start: usize,
    size: usize,
    words: usize,
) -> (u64, usize) {
    let mut seen = vec![false; terms.len()];
    let mut score = 0;
    let (mut first, mut last): (Option<usize>, usize) = (None, 0);
    for p in places
        .iter()
        .filter(|p| p.at >= start && p.at < start + size)
    {
        score += if seen[p.term] { 1 } else { 1000 };
        seen[p.term] = true;
        first.get_or_insert(p.at);
        last = p.at + terms[p.term].len();
    }
    let first = first.unwrap_or(0) as i64;
    let size = size as i64;
    let mut centred = first - (size - (last as i64 - first)) / 2;
    if centred + size > words as i64 {
        centred = words as i64 - size;
    }
    (score, centred.max(0) as usize)
}

/// Positions of words that begin a sentence: the first word, and each word
/// after a `.` or `:` followed by white space.
fn sentence_starts(text: &str, words: &[Word]) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut starts = vec![0];
    for (i, word) in words.iter().enumerate().skip(1) {
        let mut at = word.start;
        while at > 0 && matches!(bytes[at - 1], b' ' | b'\t' | b'\n' | b'\r') {
            at -= 1;
        }
        if at < word.start && at > 0 && matches!(bytes[at - 1], b'.' | b':') {
            starts.push(i);
        }
    }
    starts
}

/// An excerpt of `text` around the places `terms` (as [`terms`] gives
/// them) appear: FTS5's `snippet`, rewritten. The window of
/// `style.words` words is chosen by [`score`], preferring the start of a
/// sentence; matches inside it are marked, overlapping matches as one; and
/// the ellipsis marks where it cuts the text.
pub fn excerpt(text: &str, terms: &[Vec<String>], style: &Excerpt) -> String {
    let words_of = words(text);
    let folded: Vec<String> = words_of.iter().map(|w| w.text.clone()).collect();
    let mut places: Vec<Place> = terms
        .iter()
        .enumerate()
        .flat_map(|(term, t)| {
            places(&folded, t)
                .into_iter()
                .map(move |at| Place { term, at })
        })
        .collect();
    places.sort_by_key(|p| (p.at, p.term));
    let size = style.words.max(1);
    let count = words_of.len();

    let sentences = sentence_starts(text, &words_of);
    let (mut best, mut start) = (0, 0);
    for place in &places {
        let (s, centred) = score(&places, terms, place.at, size, count);
        if s > best {
            best = s;
            start = centred;
        }
        if count > size {
            let j = sentences.iter().rposition(|&s| s <= place.at).unwrap_or(0);
            if sentences[j] < place.at {
                let (s, _) = score(&places, terms, sentences[j], size, count);
                let s = s + if sentences[j] == 0 { 120 } else { 100 };
                if s > best {
                    best = s;
                    start = sentences[j];
                }
            }
        }
    }
    let end = start + size - 1;

    // The matched spans, overlapping places merged, from the window's
    // start on.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for p in &places {
        let last = p.at + terms[p.term].len() - 1;
        match spans.last_mut() {
            Some(span) if p.at <= span.1 => span.1 = span.1.max(last),
            _ => spans.push((p.at, last)),
        }
    }
    let mut spans = spans.into_iter().filter(|s| s.0 >= start).peekable();

    let mut out = String::new();
    if start > 0 {
        out.push_str(style.ellipsis);
    }
    let mut copied = 0;
    let mut open = false;
    for (i, word) in words_of.iter().enumerate() {
        if i < start || i > end {
            continue;
        }
        if start > 0 && i == start {
            copied = word.start;
        }
        let span = spans.peek().copied();
        if open && span.is_none_or(|s| i <= s.0) && word.start > copied {
            out.push_str(style.close);
            open = false;
        }
        if span.is_some_and(|s| s.0 == i) && !open {
            out.push_str(&text[copied..word.start]);
            out.push_str(style.open);
            copied = word.start;
            open = true;
        }
        if span.is_some_and(|s| s.1 == i) {
            if !open {
                out.push_str(style.open);
                open = true;
            }
            out.push_str(&text[copied..word.end]);
            copied = word.end;
            spans.next();
        }
        if i == end {
            if open && spans.peek().is_some_and(|s| i >= s.0) {
                out.push_str(style.close);
                open = false;
            }
            out.push_str(&text[copied..word.end]);
            copied = word.end;
        }
    }
    if open {
        out.push_str(style.close);
    }
    if end + 1 >= count {
        out.push_str(&text[copied..]);
    } else {
        out.push_str(style.ellipsis);
    }
    out
}

#[cfg(test)]
mod tests;
