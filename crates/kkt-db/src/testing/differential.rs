//! [`super::FastPlace`] answers as [`super::MemoryPlace`] does: the same
//! seeded random writes and questions, over the conformance checks'
//! library, run on both, and every answer and every refusal compared. The
//! conformance checks hold each backend to examples; this holds the fast
//! stand-in's own SQL translation to the plain one across many more
//! combinations. Values come from small pools, so names clash, references
//! dangle and rules are broken often.

use super::conformance::library;
use super::{FastPlace, MemoryPlace};
use crate::search::{Excerpt, Search};
use crate::{
    Backend, BackendExt, Condition, Count, DbError, Delete, Field, Group, Insert, OrderBy, Related,
    Select, Sort, Update, Value, col, through,
};
use std::sync::Arc;

/// A small seeded generator (xorshift64*), so a failing seed can be rerun.
struct Draw(u64);

impl Draw {
    fn new(seed: u64) -> Draw {
        Draw(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % n.max(1)
    }

    fn chance(&mut self, in_ten: usize) -> bool {
        self.below(10) < in_ten
    }

    fn pick<T: Clone>(&mut self, from: &[T]) -> T {
        from[self.below(from.len())].clone()
    }
}

const NAMES: &[&str] = &["ann", "Ann", "bob", "cy", "dee", "library"];
const SHELVES: &[&str] = &["a", "b", "c", "d"];
const TITLES: &[&str] = &[
    "red fox",
    "Red Fox",
    "the blue fox",
    "fox and hound",
    "naïve café",
    "NAÏVE CAFÉ",
    "",
    "o'brien \"quoted\" \\ line\nbreak",
    // Text with a NUL inside: compared, folded and searched whole.
    "A\0x",
    "a\0y",
];
const INTS: &[i64] = &[-1, 0, 1, 2, 3, 5, 8];

fn text(draw: &mut Draw, from: &[&str]) -> Value {
    Value::Text(draw.pick(from).to_string())
}

fn int(draw: &mut Draw) -> Value {
    Value::Int(draw.pick(INTS))
}

fn book_id(draw: &mut Draw) -> Value {
    Value::Int(draw.below(8) as i64 + 1)
}

/// A condition on books, the table with every kind of column.
fn book_condition(draw: &mut Draw, depth: usize) -> Condition {
    let choice = if depth > 2 {
        draw.below(9)
    } else {
        draw.below(13)
    };
    match choice {
        0 => Condition::Eq(col("shelf"), text(draw, SHELVES)),
        1 => Condition::Ne(col("title"), text(draw, TITLES)),
        2 => Condition::Lt(col("pages"), int(draw)),
        3 => Condition::Ge(col("rating"), int(draw)),
        4 => Condition::IsNull(col("pages")),
        5 => Condition::OneOf(
            col("id"),
            (0..draw.below(4)).map(|_| book_id(draw)).collect(),
        ),
        6 => Condition::EqIgnoringAsciiCase(col("title"), draw.pick(TITLES).to_string()),
        7 => Condition::ContainsIgnoringAsciiCase(
            col("title"),
            draw.pick(&["fox", "FOX", "é", "", "ï", "\0", "a\0"])
                .to_string(),
        ),
        8 => Condition::Eq(through("shelf", "owner"), text(draw, NAMES)),
        9 => Condition::exists(
            Related::new("loans", "book", "id")
                .filter(Condition::Eq(col("person"), text(draw, NAMES))),
        ),
        10 => Condition::not(book_condition(draw, depth + 1)),
        11 => Condition::All(
            (0..draw.below(3))
                .map(|_| book_condition(draw, depth + 1))
                .collect(),
        ),
        _ => Condition::Any(
            (0..draw.below(3))
                .map(|_| book_condition(draw, depth + 1))
                .collect(),
        ),
    }
}

fn person_condition(draw: &mut Draw) -> Condition {
    match draw.below(5) {
        0 => Condition::Eq(col("name"), text(draw, NAMES)),
        1 => Condition::IsNull(col("nick")),
        2 => Condition::Eq(col("active"), Value::Bool(draw.chance(5))),
        3 => Condition::EqIgnoringAsciiCase(col("name"), draw.pick(NAMES).to_uppercase()),
        _ => Condition::True,
    }
}

/// One change, often against a rule.
fn change(draw: &mut Draw) -> Change {
    match draw.below(14) {
        0 | 1 => {
            let mut insert = Insert::new("people")
                .value("name", text(draw, NAMES))
                .value("joined", Value::Text("2026".into()));
            if draw.chance(4) {
                insert = insert.value("nick", text(draw, NAMES));
            }
            if draw.chance(3) {
                insert = insert.value("active", Value::Bool(draw.chance(5)));
            }
            if draw.chance(2) {
                insert = insert.value("photo", Value::Bytes(vec![draw.below(3) as u8]));
            }
            Change::Insert(insert)
        }
        2 => {
            let mut insert = Insert::new("shelves").value("name", text(draw, SHELVES));
            if draw.chance(6) {
                insert = insert.value("owner", text(draw, NAMES));
            }
            if draw.chance(5) {
                Change::Insert(insert)
            } else {
                Change::Save(insert)
            }
        }
        3 | 4 => {
            let mut insert = Insert::new("books")
                .value("shelf", text(draw, SHELVES))
                .value("title", text(draw, TITLES));
            if draw.chance(6) {
                insert = insert.value("pages", int(draw));
            }
            Change::Insert(insert)
        }
        5 => Change::Insert(
            Insert::new("editions")
                .value("book", book_id(draw))
                .value("number", int(draw))
                .value("text", text(draw, TITLES)),
        ),
        6 => Change::Insert(
            Insert::new("holds")
                .value("book", book_id(draw))
                .value("person", text(draw, NAMES)),
        ),
        7 => {
            let insert = Insert::new("loans")
                .value("book", book_id(draw))
                .value("person", text(draw, NAMES))
                .value("at", text(draw, &["1", "2", "3"]));
            match draw.below(3) {
                0 => Change::Insert(insert),
                1 => Change::Keep(insert),
                _ => Change::Save(insert),
            }
        }
        8 => {
            let mut update = Update::new("books", book_condition(draw, 0));
            match draw.below(4) {
                0 => update = update.set("pages", int(draw)),
                1 => update = update.set_if_null("pages", int(draw)),
                2 => update = update.set("title", text(draw, TITLES)),
                _ => update = update.set("shelf", text(draw, SHELVES)),
            }
            Change::Update(update)
        }
        9 => Change::Update(
            Update::new("people", person_condition(draw))
                .set_if_null("nick", text(draw, NAMES))
                .set("active", Value::Bool(draw.chance(5))),
        ),
        10 => Change::Update(
            Update::new("editions", Condition::Eq(col("book"), book_id(draw)))
                .set("text", text(draw, TITLES)),
        ),
        11 => {
            let table = draw.pick(&["loans", "holds"]);
            let filter = if draw.chance(5) {
                Condition::Eq(col("person"), text(draw, NAMES))
            } else {
                Condition::Eq(col("book"), book_id(draw))
            };
            Change::Delete(Delete::new(table, filter))
        }
        12 => {
            // A book others hold, lend or print breaks their references;
            // an edition is never deleted.
            if draw.chance(7) {
                Change::Delete(Delete::new(
                    "books",
                    Condition::Eq(col("id"), book_id(draw)),
                ))
            } else {
                Change::Delete(Delete::new(
                    "editions",
                    Condition::Eq(col("book"), book_id(draw)),
                ))
            }
        }
        _ => Change::Delete(Delete::new("people", person_condition(draw))),
    }
}

/// One question, over books mostly.
fn question(draw: &mut Draw) -> Question {
    match draw.below(8) {
        // A shelf's owner is a reference with an exception value
        // (`library`): read through it, compared ignoring ASCII case.
        6 => Question::Select(
            Select::columns("shelves", &["name"])
                .filter(Condition::EqIgnoringAsciiCase(
                    through("owner", "nick"),
                    draw.pick(NAMES).to_uppercase(),
                ))
                .order(Sort::asc(col("name"))),
        ),
        // A question inside a condition keeps its own order and limit.
        7 => {
            let inner = Select::columns("loans", &["book"])
                .order(Sort::desc(col("book")))
                .limit(draw.below(3) as u64);
            Question::Select(
                Select::columns("books", &["id"])
                    .filter(Condition::In(col("id"), Box::new(inner)))
                    .order(Sort::asc(col("id"))),
            )
        }
        0 | 1 => {
            let mut fields = vec![Field::Value(col("id")), Field::Value(col("title"))];
            if draw.chance(5) {
                fields.push(Field::Value(through("shelf", "owner")));
            }
            if draw.chance(5) {
                fields.push(Field::Count(Related::new("loans", "book", "id")));
            }
            if draw.chance(4) {
                fields.push(Field::Max(Related::new("loans", "book", "id"), "at"));
            }
            if draw.chance(4) {
                fields.push(Field::Newest {
                    related: Related::new("holds", "book", "id"),
                    by: "seq",
                    column: "person",
                });
            }
            if draw.chance(4) {
                fields.push(Field::Exists(Related::new("editions", "book", "id")));
            }
            let mut select = Select::new("books", fields).filter(book_condition(draw, 0));
            let sorts = draw.below(3);
            let mut after = Vec::new();
            for _ in 0..sorts {
                let (by, value) = match draw.below(4) {
                    0 => (OrderBy::Operand(col("title")), text(draw, TITLES)),
                    1 => (OrderBy::Operand(col("pages")), int(draw)),
                    2 => (OrderBy::FirstOf(col("pages"), col("rating")), int(draw)),
                    _ => (OrderBy::Operand(col("id")), book_id(draw)),
                };
                select = select.order(if draw.chance(5) {
                    Sort::Asc(by)
                } else {
                    Sort::Desc(by)
                });
                after.push(value);
            }
            // Every row ends up unique in its order: the id breaks ties.
            select = select.order(Sort::asc(col("id")));
            after.push(book_id(draw));
            if sorts > 0 && draw.chance(4) {
                select = select.after(after);
            }
            if draw.chance(4) {
                select = select.limit(draw.below(4) as u64);
            }
            Question::Select(select)
        }
        2 => Question::Count(Count {
            table: "books",
            filter: book_condition(draw, 0),
        }),
        3 => Question::Group(Group {
            table: "loans",
            by: col(draw.pick(&["person", "book"])),
            also: Vec::new(),
            max: "at",
            filter: Condition::True,
        }),
        4 => Question::Select(
            Select::columns("people", &["name", "nick", "active", "photo", "city"])
                .filter(person_condition(draw))
                .order(Sort::asc(col("id"))),
        ),
        _ => Question::Search(Search {
            select: Select::columns("books", &["id"])
                .filter(book_condition(draw, 0))
                .order(Sort::desc(col("id")))
                .limit(draw.below(4) as u64 + 1),
            terms: vec![
                draw.pick(&["fox", "FOX", "café", "naïve", "blue fox", "hound"])
                    .to_string(),
            ],
            excerpt: Excerpt {
                open: "[",
                close: "]",
                ellipsis: "…",
                words: 3,
            },
        }),
    }
}

#[derive(Clone)]
enum Change {
    Insert(Insert),
    Keep(Insert),
    Save(Insert),
    Update(Update),
    Delete(Delete),
}

#[derive(Clone)]
enum Question {
    Select(Select),
    Count(Count),
    Group(Group),
    Search(Search),
}

/// One write of `changes`, all or nothing, as an answer to compare.
async fn write(db: &Arc<dyn Backend>, changes: Vec<Change>) -> String {
    let done = BackendExt::write(db.as_ref(), move |c| {
        let mut said = Vec::new();
        for change in &changes {
            said.push(match change {
                Change::Insert(i) => format!("{:?}", c.insert(i)?),
                Change::Keep(i) => format!("{:?}", c.insert_unless_present(i)?),
                Change::Save(i) => format!("{:?}", c.save(i)?),
                Change::Update(u) => format!("{:?}", c.update(u)?),
                Change::Delete(d) => format!("{:?}", c.delete(d)?),
            });
        }
        Ok::<_, DbError>(said)
    })
    .await;
    format!("{done:?}")
}

async fn read(db: &Arc<dyn Backend>, question: Question) -> String {
    let done = BackendExt::read(db.as_ref(), move |c| {
        Ok::<_, DbError>(match &question {
            Question::Select(s) => format!("{:?}", c.select(s)?),
            Question::Count(n) => format!("{:?}", c.count(n)?),
            Question::Group(g) => format!("{:?}", c.group(g)?),
            Question::Search(s) => format!("{:?}", c.search(s)?),
        })
    })
    .await;
    format!("{done:?}")
}

#[tokio::test]
async fn the_fast_database_answers_as_the_memory_database_does() {
    for seed in 1..=40 {
        let mut draw = Draw::new(seed);
        // Each seed also declares an index whose name may be one the
        // stand-in could use for its own: the key's, a unique column's, a
        // declared one's.
        let (table, column) =
            draw.pick(&[("people", "name"), ("books", "title"), ("loans", "person")]);
        let extras = crate::Set {
            name: "extras",
            migrations: vec![vec![crate::Step::AddIndex {
                table,
                index: crate::Index {
                    name: draw.pick(&["pk", "key", "name", "unique_name", "index_x", "by_x"]),
                    columns: vec![column],
                },
            }]],
        };
        let sets = [library(), extras];
        let plain = MemoryPlace::new().open(&sets);
        let fast = FastPlace::new().and_then(|p| p.open(&sets));
        let (plain, fast) = match (plain, fast) {
            (Ok(plain), Ok(fast)) => (plain, fast),
            (plain, fast) => panic!(
                "seed {seed}: opening: {:?} and {:?}",
                plain.map(drop),
                fast.map(drop)
            ),
        };
        for step in 0..150 {
            let (said, also) = if draw.chance(5) {
                let changes: Vec<Change> =
                    (0..draw.below(3) + 1).map(|_| change(&mut draw)).collect();
                (
                    write(&plain, changes.clone()).await,
                    write(&fast, changes).await,
                )
            } else {
                let q = question(&mut draw);
                (read(&plain, q.clone()).await, read(&fast, q).await)
            };
            assert_eq!(said, also, "seed {seed}, step {step}");
        }
    }
}

/// Two tables may declare indexes whose names, joined to their tables',
/// read the same: `a` with `index_x` and `a_index` with `x`.
#[test]
fn indexes_of_two_tables_never_clash() {
    use crate::{Column, Set, Step, Table};
    let set = Set {
        name: "indexes",
        migrations: vec![vec![
            Step::AddTable(
                Table::new("a", vec![Column::text("id")], vec!["id"]).index("index_x", vec!["id"]),
            ),
            Step::AddTable(
                Table::new("a_index", vec![Column::text("id")], vec!["id"]).index("x", vec!["id"]),
            ),
            Step::AddTable(
                Table::new("a_key", vec![Column::text("id").unique()], vec!["id"])
                    .index("unique_id", vec!["id"]),
            ),
        ]],
    };
    assert!(MemoryPlace::new().open(std::slice::from_ref(&set)).is_ok());
    let fast = FastPlace::new().and_then(|p| p.open(&[set]));
    assert!(fast.is_ok(), "{:?}", fast.err());
}

/// A table may declare an index of any name: it never clashes with the
/// stand-in's own indexes, for the key, unique columns and declared ones.
#[test]
fn an_index_of_any_name_opens() {
    use crate::{Column, Set, Step, Table};
    for name in [
        "pk",
        "key",
        "key_",
        "unique_id",
        "index_x",
        "_fast_items_key_",
    ] {
        let set = Set {
            name: "indexes",
            migrations: vec![vec![Step::AddTable(
                Table::new("items", vec![Column::text("id").unique()], vec!["id"])
                    .index(name, vec!["id"]),
            )]],
        };
        assert!(
            MemoryPlace::new().open(std::slice::from_ref(&set)).is_ok(),
            "{name}"
        );
        let fast = FastPlace::new().and_then(|p| p.open(&[set]));
        assert!(fast.is_ok(), "{name}: {:?}", fast.err());
    }
}
