//! The conformance suite on both places, and search against FTS5 itself.

use crate::{MemoryPlace, Options, SqlitePlace, copy_database};
use kkt_db::search::{self, Excerpt};
use kkt_db::testing::{Place, conformance};
use kkt_db::{Database, DbError, Set, Step, Upgrade};
use kkt_scratch as scratch;
use rusqlite::Connection;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn options() -> Options {
    Options {
        readers: 2,
        busy_timeout: Duration::from_secs(5),
    }
}

/// A database file in its own scratch folder, removed with it.
struct FilePlace {
    _dir: scratch::Dir,
    place: SqlitePlace,
}

impl Place for FilePlace {
    fn open(&self, sets: &[Set]) -> Result<Database, DbError> {
        self.place.open(sets)
    }

    fn pending(&self, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
        self.place.pending(sets)
    }
}

impl Place for MemoryPlace {
    fn open(&self, sets: &[Set]) -> Result<Database, DbError> {
        MemoryPlace::open(self, sets)
    }

    fn pending(&self, sets: &[Set]) -> Result<Vec<Upgrade>, DbError> {
        MemoryPlace::pending(self, sets)
    }
}

static PLACES: AtomicU32 = AtomicU32::new(0);

fn file_place() -> FilePlace {
    let n = PLACES.fetch_add(1, Ordering::Relaxed);
    let dir = scratch::Dir::new(&format!("kkt-db-sqlite-{n}"));
    let place = SqlitePlace::new(&dir.join("kkt.db"), options());
    FilePlace { _dir: dir, place }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_file_keeps_the_contract() {
    conformance(|| Box::new(file_place())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_in_memory_keeps_the_contract() {
    conformance(|| Box::new(MemoryPlace::new().expect("an in-memory database"))).await;
}

#[test]
fn a_database_from_before_kkt_db_is_refused_and_left_alone() {
    let place = file_place();
    {
        let conn = Connection::open(&place.place.path).expect("make a database");
        conn.execute_batch("CREATE TABLE agents (name TEXT); PRAGMA user_version = 21;")
            .expect("an older kkt's database");
    }
    let refused = place.place.open(&[]);
    assert!(
        matches!(refused, Err(DbError::Backend(ref m)) if m.contains("schema 21")),
        "{:?}",
        refused.err()
    );
    assert!(
        place.place.pending(&[]).is_err(),
        "pending on an older database"
    );
    let conn = Connection::open(&place.place.path).expect("reopen");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("user_version");
    assert_eq!(version, 21, "the refused database was changed");
}

/// A deleted row leaves nothing in the search index. (Its words could not
/// match anyway: an insert clears its rowid's entry first. This keeps the
/// index from growing with the dead.)
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_row_leaves_nothing_in_the_search_index() {
    let place = file_place();
    let set = Set {
        name: "notes",
        migrations: vec![vec![
            Step::AddTable(kkt_db::Table::new(
                "notes",
                vec![kkt_db::Column::text("name"), kkt_db::Column::text("text")],
                vec!["name"],
            )),
            Step::AddSearch {
                table: "notes",
                column: "text",
            },
        ]],
    };
    let db = place.place.open(&[set]).expect("open");
    db.write(|c| {
        for name in ["a", "b"] {
            c.insert(
                &kkt_db::Insert::new("notes")
                    .value("name", name)
                    .value("text", "some words"),
            )?;
        }
        c.delete(&kkt_db::Delete::new(
            "notes",
            kkt_db::Condition::Eq(kkt_db::col("name"), "a".into()),
        ))
    })
    .await
    .map_err(|e: DbError| e)
    .expect("two notes, one deleted");
    let conn = Connection::open(&place.place.path).expect("open the file");
    let indexed: i64 = conn
        .query_row("SELECT count(*) FROM kkt_search_notes", [], |r| r.get(0))
        .expect("count the index");
    assert_eq!(indexed, 1, "the deleted row is still in the search index");
}

#[test]
fn a_file_with_tables_kkt_did_not_make_is_refused_and_left_alone() {
    let place = file_place();
    {
        let conn = Connection::open(&place.place.path).expect("make a database");
        conn.execute_batch("CREATE TABLE photos (id INTEGER);")
            .expect("someone else's table");
    }
    let refused = place.place.open(&[]);
    assert!(
        matches!(refused, Err(DbError::Backend(ref m)) if m.contains("did not make")),
        "{:?}",
        refused.err()
    );
    let conn = Connection::open(&place.place.path).expect("reopen");
    let tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .expect("tables");
    assert_eq!(tables, 1, "the refused file was changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_is_one_consistent_file() {
    let place = file_place();
    let set = Set {
        name: "notes",
        migrations: vec![vec![Step::AddTable(kkt_db::Table::new(
            "notes",
            vec![kkt_db::Column::key("id"), kkt_db::Column::text("text")],
            vec!["id"],
        ))]],
    };
    let db = place.place.open(std::slice::from_ref(&set)).expect("open");
    db.write(|c| {
        c.insert(&kkt_db::Insert::new("notes").value("text", "kept"))
            .map(drop)
    })
    .await
    .map_err(|e: DbError| e)
    .expect("a note");
    let copy = place._dir.join("copy.db");
    copy_database(&place.place.path, &copy, Duration::from_secs(5)).expect("copy");
    assert!(
        !place._dir.join("copy.db-wal").exists(),
        "the copy has a log beside it"
    );
    let copied = SqlitePlace::new(&copy, options())
        .open(&[set])
        .expect("open the copy");
    let n = copied
        .read(|c| {
            c.count(&kkt_db::Count {
                table: "notes",
                filter: kkt_db::Condition::True,
            })
        })
        .await
        .map_err(|e: DbError| e)
        .expect("count");
    assert_eq!(n, 1);
    assert!(
        copy_database(&place.place.path, &copy, Duration::from_secs(5)).is_err(),
        "a copy over a file"
    );
}

/// FTS5's own tokenizer, `unicode61` with its defaults, for comparing.
struct Fts5 {
    conn: Connection,
}

impl Fts5 {
    fn new() -> Fts5 {
        let conn = Connection::open_in_memory().expect("in memory");
        conn.execute_batch(
            "CREATE VIRTUAL TABLE t USING fts5(b);
             CREATE VIRTUAL TABLE v USING fts5vocab(t, instance);",
        )
        .expect("fts5 tables");
        Fts5 { conn }
    }

    /// The words FTS5 makes of each text, in order.
    fn words(&self, texts: &[String]) -> Vec<Vec<String>> {
        self.conn
            .execute_batch("BEGIN; DELETE FROM t;")
            .expect("clear");
        {
            let mut insert = self
                .conn
                .prepare("INSERT INTO t (rowid, b) VALUES (?1, ?2)")
                .expect("insert");
            for (i, text) in texts.iter().enumerate() {
                insert
                    .execute(rusqlite::params![i as i64 + 1, text])
                    .expect("insert");
            }
        }
        self.conn.execute_batch("COMMIT").expect("commit");
        let mut words = vec![Vec::new(); texts.len()];
        let mut query = self
            .conn
            .prepare("SELECT doc, term FROM v ORDER BY doc, offset")
            .expect("vocabulary");
        let rows = query
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .expect("vocabulary");
        for row in rows {
            let (doc, term) = row.expect("a word");
            words[doc as usize - 1].push(term);
        }
        words
    }
}

/// How FTS5 classes a character: a separator, a dropped diacritic, or a
/// word character folded to another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Separator,
    Drop,
    Keep(u32),
}

fn classes(words: &[String]) -> Class {
    match words {
        [a, b] if a == "q" && b == "z" => Class::Separator,
        [w] if w == "qz" => Class::Drop,
        [w] => {
            let mid: Vec<char> = w.chars().collect();
            assert_eq!(
                mid.len(),
                3,
                "a word character that folds to more than one: {w:?}"
            );
            Class::Keep(mid[1] as u32)
        }
        other => panic!("an unexpected split: {other:?}"),
    }
}

/// Every character's class, by FTS5 and by kkt-db, side by side.
fn every_character() -> Vec<(u32, Class, Class)> {
    let fts5 = Fts5::new();
    let characters: Vec<char> = (0..=0x10FFFF).filter_map(char::from_u32).collect();
    let mut out = Vec::with_capacity(characters.len());
    for chunk in characters.chunks(4096) {
        let texts: Vec<String> = chunk.iter().map(|c| format!("q{c}z")).collect();
        let theirs = fts5.words(&texts);
        for ((c, text), words) in chunk.iter().zip(&texts).zip(theirs) {
            let ours: Vec<String> = search::words(text).into_iter().map(|w| w.text).collect();
            out.push((*c as u32, classes(&words), classes(&ours)));
        }
    }
    out
}

/// kkt-db's word table, written from FTS5's classes.
fn table_source(classes: &[(u32, Class)]) -> String {
    let mut runs: Vec<(u32, u32, Class, i64)> = Vec::new();
    for &(cp, class) in classes {
        let offset = match class {
            Class::Keep(to) => i64::from(to) - i64::from(cp),
            _ => 0,
        };
        let kind = match class {
            Class::Keep(_) => Class::Keep(0),
            other => other,
        };
        match runs.last_mut() {
            Some(run) if run.1 + 1 == cp && run.2 == kind && run.3 == offset => run.1 = cp,
            _ => runs.push((cp, cp, kind, offset)),
        }
    }
    let mut out = String::from(
        "//! Which characters make up words, and how each is folded: a table of\n\
         //! ranges, generated from SQLite FTS5's `unicode61` tokenizer with its\n\
         //! defaults (Unicode 6.1; case folded; diacritics removed from Latin\n\
         //! letters), so kkt's search splits words exactly as it did when FTS5\n\
         //! split them. Each entry is `(first, last, class)`; a character in no\n\
         //! range separates words. kkt-db-sqlite's tests check every character\n\
         //! against FTS5 and can regenerate this file.\n\
         \n\
         use super::Class::{self, Drop, Keep};\n\
         \n\
         #[rustfmt::skip]\n\
         pub(super) const RANGES: &[(u32, u32, Class)] = &[\n",
    );
    for (first, last, kind, offset) in runs {
        let class = match kind {
            Class::Separator => continue,
            Class::Drop => "Drop".to_string(),
            Class::Keep(_) => format!("Keep({offset})"),
        };
        out.push_str(&format!("    (0x{first:04X}, 0x{last:04X}, {class}),\n"));
    }
    out.push_str("];\n");
    out
}

/// Every character splits and folds as FTS5's `unicode61` does. With
/// `UPDATE_WORDS=1`, rewrites kkt-db's table from FTS5 instead.
#[test]
fn every_character_makes_words_as_fts5_does() {
    let all = every_character();
    if std::env::var_os("UPDATE_WORDS").is_some() {
        let source = table_source(
            &all.iter()
                .map(|&(cp, theirs, _)| (cp, theirs))
                .collect::<Vec<_>>(),
        );
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../kkt-db/src/search/table.rs");
        std::fs::write(&path, source).expect("write the word table");
        return;
    }
    let different: Vec<_> = all
        .iter()
        .filter(|(_, theirs, ours)| theirs != ours)
        .take(20)
        .collect();
    assert!(
        different.is_empty(),
        "characters kkt-db classes otherwise than FTS5: {different:x?}"
    );
}

/// A small generator of texts: the same sequence on every run.
struct Texts(u64);

impl Texts {
    fn next(&mut self) -> u64 {
        // Knuth's MMIX linear congruential generator.
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.next() as usize % items.len()]
    }
}

const VOCABULARY: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "Delta",
    "long",
    "poll",
    "long-poll",
    "PDF.js",
    "parser",
    "café",
    "naïve",
    "x",
    "y",
    "z",
    "kkt",
    "search",
    "the",
    "a",
    "of",
    "🚀",
    "--",
    "中文",
    "Straße",
];
const GLUE: &[&str] = &[
    " ", " ", " ", " ", ", ", ". ", ": ", "\n", " — ", "; ", "! ", "  ",
];

/// Search's matches and excerpts agree with FTS5's `MATCH` and `snippet`
/// over generated texts and terms, single words and phrases.
#[test]
fn search_matches_and_excerpts_as_fts5_does() {
    let conn = Connection::open_in_memory().expect("in memory");
    conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(b)")
        .expect("fts5");
    let style = Excerpt {
        open: "**",
        close: "**",
        ellipsis: "…",
        words: 12,
    };
    let mut texts = Texts(7);
    let mut checked = 0;
    for _ in 0..4000 {
        let length = 1 + texts.next() as usize % 40;
        let mut text = String::new();
        let mut used = Vec::new();
        for i in 0..length {
            if i > 0 {
                text.push_str(texts.pick(GLUE));
            }
            let word = texts.pick(VOCABULARY);
            used.push(word);
            text.push_str(word);
        }
        // Most terms are runs of the text's own words, so most searches
        // match and their excerpts can be compared; the rest are random.
        let term_count = 1 + texts.next() as usize % 3;
        let raw: Vec<String> = (0..term_count)
            .map(|_| {
                let words = 1 + texts.next() as usize % 3;
                if texts.next().is_multiple_of(5) {
                    (0..words)
                        .map(|_| texts.pick(VOCABULARY))
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    let start = texts.next() as usize % used.len();
                    used[start..(start + words).min(used.len())].join(" ")
                }
            })
            .collect();
        let terms = search::terms(&raw);
        conn.execute("DELETE FROM t", []).expect("clear");
        conn.execute("INSERT INTO t (rowid, b) VALUES (1, ?1)", [&text])
            .expect("insert");
        let query: Vec<String> = raw
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect();
        let theirs: Option<String> = conn
            .query_row(
                "SELECT snippet(t, 0, '**', '**', '…', 12) FROM t WHERE t MATCH ?1",
                [query.join(" ")],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
            .expect("fts5 search");
        let ours = search::matches(&text, &terms).then(|| search::excerpt(&text, &terms, &style));
        assert_eq!(ours, theirs, "text {text:?}, terms {raw:?}");
        checked += usize::from(theirs.is_some());
    }
    assert!(
        checked > 1000,
        "too few matches to compare excerpts: {checked}"
    );
}

/// What a migration makes depends only on its steps: two databases brought
/// to the same version, one step at a time or all at once, have the same
/// tables, indexes and triggers, byte for byte.
#[tokio::test(flavor = "multi_thread")]
async fn the_schema_is_the_same_however_a_database_got_there() {
    fn library(versions: usize) -> Set {
        let mut migrations = vec![
            vec![Step::AddTable(kkt_db::Table::new(
                "people",
                vec![
                    kkt_db::Column::key("id"),
                    kkt_db::Column::text("name").unique(),
                ],
                vec!["id"],
            ))],
            vec![
                Step::AddColumn {
                    table: "people",
                    column: kkt_db::Column::text("city").nullable(),
                },
                Step::AddSearch {
                    table: "people",
                    column: "name",
                },
            ],
        ];
        migrations.truncate(versions);
        Set {
            name: "library",
            migrations,
        }
    }
    let master = |place: &FilePlace| -> Vec<(String, String, Option<String>)> {
        let conn = Connection::open(&place.place.path).expect("open");
        let mut statement = conn
            .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
            .expect("sqlite_master");
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("sqlite_master")
            .collect::<rusqlite::Result<_>>()
            .expect("sqlite_master")
    };
    let stepwise = file_place();
    stepwise.place.open(&[library(1)]).expect("version 1");
    stepwise.place.open(&[library(2)]).expect("version 2");
    let at_once = file_place();
    at_once
        .place
        .open(&[library(2)])
        .expect("version 2 at once");
    assert_eq!(master(&stepwise), master(&at_once));
}
