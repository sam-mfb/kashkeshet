//! The checks every backend must pass. A backend's tests call
//! [`conformance`] with a way to make an empty place for a database; each
//! check makes its own, so they do not depend on each other.
//!
//! The checks use a small library: people, shelves, books, the editions of
//! a book and loans. Its first migration declares every kind of column,
//! key, reference and constraint; its second adds columns and fills one.

use super::MemoryPlace;
use crate::{
    Column, Condition, Connection, Constraint, ConstraintKind, Count, Database, DbError, Delete,
    Evolve, Field, Group, Index, Insert, OrderBy, Related, Row, Select, Set, Sort, Step, Table,
    Update, Value, col, pending, through,
};

/// Where a database lives, which can be opened more than once: the second
/// open finds what the first wrote, as a server restart does.
pub trait Place {
    /// Brings the database here up to date with `sets` and opens it.
    fn open(&self, sets: &[Set]) -> Result<Database, DbError>;

    /// What opening it with `sets` would upgrade, without changing it.
    fn pending(&self, sets: &[Set]) -> Result<Vec<crate::Upgrade>, DbError>;
}

impl Place for MemoryPlace {
    fn open(&self, sets: &[Set]) -> Result<Database, DbError> {
        MemoryPlace::open(self, sets)
    }

    fn pending(&self, sets: &[Set]) -> Result<Vec<crate::Upgrade>, DbError> {
        pending(self as &dyn Evolve, sets)
    }
}

fn first() -> Vec<Step> {
    vec![
        Step::AddTable(Table::new(
            "people",
            vec![
                Column::key("id"),
                Column::text("name").unique(),
                Column::text("nick").nullable(),
                Column::text("joined"),
                Column::bool("active").default(true),
                Column::bytes("photo").nullable(),
            ],
            vec!["id"],
        )),
        Step::AddTable(
            Table::new(
                "shelves",
                vec![
                    Column::text("name"),
                    Column::text("owner")
                        .nullable()
                        .references("people", "name")
                        .except("library"),
                ],
                vec!["name"],
            )
            .constraint(Constraint::NoDelete),
        ),
        Step::AddTable(
            Table::new(
                "books",
                vec![
                    Column::key("id"),
                    Column::text("shelf").references("shelves", "name"),
                    Column::text("title"),
                    Column::int("pages").nullable(),
                ],
                vec!["id"],
            )
            .index("books_by_shelf", vec!["shelf", "id"])
            .constraint(Constraint::Fixed("shelf"))
            .constraint(Constraint::Positive("pages")),
        ),
        Step::AddTable(
            Table::new(
                "editions",
                vec![
                    Column::int("book").references("books", "id"),
                    Column::int("number"),
                    Column::text("text"),
                ],
                vec!["book", "number"],
            )
            .constraint(Constraint::Immutable)
            .constraint(Constraint::Positive("number")),
        ),
        Step::AddTable(Table::new(
            "loans",
            vec![
                Column::int("book").references("books", "id"),
                Column::text("person").references("people", "name"),
                Column::text("at"),
            ],
            vec!["book", "person"],
        )),
    ]
}

fn second() -> Vec<Step> {
    vec![
        Step::AddColumn {
            table: "people",
            column: Column::text("city").nullable(),
        },
        Step::AddColumn {
            table: "books",
            column: Column::int("rating").default(3),
        },
        Step::AddIndex {
            table: "loans",
            index: Index {
                name: "loans_by_person",
                columns: vec!["person"],
            },
        },
        Step::Fill {
            name: "nicks from names",
            run: fill_nicks,
        },
    ]
}

/// Gives everyone without a nickname their name as one.
fn fill_nicks(c: &dyn Connection) -> Result<(), DbError> {
    let rows = c.select(
        &Select::columns("people", &["id", "name"]).filter(Condition::IsNull(col("nick"))),
    )?;
    for row in rows {
        let (id, name): (i64, String) = (row.get(0)?, row.get(1)?);
        c.update(&Update::new("people", Condition::Eq(col("id"), id.into())).set("nick", name))?;
    }
    Ok(())
}

fn library() -> Set {
    Set {
        name: "library",
        migrations: vec![first(), second()],
    }
}

fn library_at_one() -> Set {
    Set {
        name: "library",
        migrations: vec![first()],
    }
}

/// Runs every check against fresh places from `new_place`. Panics, naming
/// the check, at the first that fails.
pub async fn conformance(new_place: impl Fn() -> Box<dyn Place>) {
    let open = || {
        new_place()
            .open(&[library()])
            .expect("open a new database with the library's tables")
    };
    values_keep_their_kind(open()).await;
    keys_only_go_up(open()).await;
    columns_left_out_get_defaults(open()).await;
    constraints_refuse_and_change_nothing(open()).await;
    references_hold_through_changes_and_deletes(open()).await;
    conditions_answer_as_the_contract_says(open()).await;
    text_compares_by_code_point_and_folds_only_ascii(open()).await;
    references_are_followed(open()).await;
    related_rows_give_counts_maxima_newest_and_existence(open()).await;
    rows_come_in_order_with_paging_and_limits(open()).await;
    counts_and_groups(open()).await;
    inserts_saves_updates_and_deletes_say_what_they_did(open()).await;
    a_failed_write_keeps_nothing(open()).await;
    reads_cannot_change_anything(open()).await;
    writes_run_as_if_alone(open()).await;
    questions_that_do_not_fit_the_schema_are_refused(open()).await;
    a_change_can_be_made_to_fail(open()).await;
    migrations_apply_once_in_order_and_fill(&new_place).await;
    a_failing_migration_changes_nothing(&new_place).await;
    a_database_newer_than_its_sets_is_refused(&new_place).await;
    sets_are_recorded_apart(&new_place).await;
}

async fn write<T: Send + 'static>(
    db: &Database,
    f: impl FnOnce(&dyn Connection) -> Result<T, DbError> + Send + 'static,
) -> Result<T, DbError> {
    db.write(f).await
}

async fn read<T: Send + 'static>(
    db: &Database,
    f: impl FnOnce(&dyn Connection) -> Result<T, DbError> + Send + 'static,
) -> Result<T, DbError> {
    db.read(f).await
}

fn person(name: &str) -> Insert {
    Insert::new("people")
        .value("name", name)
        .value("joined", "2026-11-01T00:00:00Z")
}

fn values(rows: Vec<Row>) -> Vec<Vec<Value>> {
    rows.into_iter().map(Row::into_values).collect()
}

fn texts(rows: Vec<Row>) -> Vec<String> {
    rows.into_iter()
        .map(|r| r.get::<String>(0).expect("text"))
        .collect()
}

/// A shelf `s1` owned by the library, and in it three books.
async fn seed(db: &Database) {
    write(db, |c| {
        for name in ["amber", "brave", "calm"] {
            c.insert(&person(name))?;
        }
        c.insert(
            &Insert::new("shelves")
                .value("name", "s1")
                .value("owner", "library"),
        )?;
        c.insert(
            &Insert::new("shelves")
                .value("name", "s2")
                .value("owner", "amber"),
        )?;
        c.insert(&Insert::new("shelves").value("name", "s3"))?;
        for (title, pages) in [("Alpha", Some(10)), ("Beta", None), ("Gamma", Some(30))] {
            c.insert(
                &Insert::new("books")
                    .value("shelf", "s1")
                    .value("title", title)
                    .value("pages", pages.map(i64::from)),
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed the library");
}

async fn values_keep_their_kind(db: Database) {
    let photo: Vec<u8> = vec![0, 1, 254, 255];
    let p = photo.clone();
    write(&db, move |c| {
        c.insert(
            &person("ünïcode 中文 🚀")
                .value("nick", Value::Null)
                .value("active", false)
                .value("photo", p),
        )?;
        c.insert(&Insert::new("shelves").value("name", "s"))?;
        c.insert(
            &Insert::new("books")
                .value("shelf", "s")
                .value("title", "big")
                .value("pages", i64::MAX),
        )
    })
    .await
    .expect("values_keep_their_kind: insert");
    let rows = read(&db, |c| {
        let mut rows = c.select(&Select::columns(
            "people",
            &["name", "nick", "active", "photo"],
        ))?;
        rows.extend(c.select(&Select::columns("books", &["pages"]))?);
        Ok(rows)
    })
    .await
    .expect("values_keep_their_kind: read");
    assert_eq!(
        values(rows),
        vec![
            vec![
                Value::from("ünïcode 中文 🚀"),
                Value::Null,
                Value::Bool(false),
                Value::Bytes(photo),
            ],
            vec![Value::Int(i64::MAX)],
        ],
        "values_keep_their_kind"
    );
}

async fn keys_only_go_up(db: Database) {
    let keys = write(&db, |c| {
        let mut keys = Vec::new();
        for name in ["a", "b", "c"] {
            keys.push(c.insert(&person(name))?);
        }
        // Deleting the newest row does not free its key.
        c.delete(&Delete::new(
            "people",
            Condition::Eq(col("name"), "c".into()),
        ))?;
        keys.push(c.insert(&person("d"))?);
        Ok(keys)
    })
    .await
    .expect("keys_only_go_up: insert");
    let later = write(&db, |c| {
        c.delete(&Delete::new(
            "people",
            Condition::Eq(col("name"), "d".into()),
        ))?;
        c.insert(&person("e"))
    })
    .await
    .expect("keys_only_go_up: insert in a later write");
    assert_eq!(
        keys,
        vec![Some(1), Some(2), Some(3), Some(4)],
        "keys_only_go_up"
    );
    assert_eq!(
        later,
        Some(5),
        "keys_only_go_up: after a delete in an earlier write"
    );
    let none = write(&db, |c| {
        c.insert(&Insert::new("shelves").value("name", "s"))
    })
    .await
    .expect("keys_only_go_up: a table without a key");
    assert_eq!(none, None, "keys_only_go_up: no key, none returned");
}

async fn columns_left_out_get_defaults(db: Database) {
    seed(&db).await;
    let rows = read(&db, |c| {
        let mut rows =
            c.select(&Select::columns("people", &["active", "nick", "city"]).limit(1))?;
        rows.extend(c.select(&Select::columns("books", &["rating"]).limit(1))?);
        Ok(rows)
    })
    .await
    .expect("columns_left_out_get_defaults: read");
    assert_eq!(
        values(rows),
        vec![
            vec![Value::Bool(true), Value::Null, Value::Null],
            vec![Value::Int(3)]
        ],
        "columns_left_out_get_defaults"
    );
}

/// Runs `change` in a write and requires it to be refused for `kind`, with
/// nothing changed.
async fn refuses(
    db: &Database,
    what: &str,
    kind: ConstraintKind,
    change: impl FnOnce(&dyn Connection) -> Result<(), DbError> + Send + 'static,
) {
    let before = snapshot(db).await;
    let result = write(db, move |c| {
        // The refused change is caught, so the write itself commits:
        // nothing the refused change touched may be kept.
        let refused = change(c);
        Ok(refused)
    })
    .await
    .expect("the write around a refused change");
    match result {
        Err(DbError::Constraint { kind: found, .. }) if found == kind => {}
        other => panic!("{what}: expected a {kind:?} refusal, got {other:?}"),
    }
    assert_eq!(
        snapshot(db).await,
        before,
        "{what}: a refused change changed something"
    );
}

/// Every row of every table in the library, for comparing before and after.
async fn snapshot(db: &Database) -> Vec<Vec<Value>> {
    read(db, |c| {
        let mut all = Vec::new();
        for (table, columns) in [
            (
                "people",
                &["id", "name", "nick", "joined", "active", "photo", "city"][..],
            ),
            ("shelves", &["name", "owner"][..]),
            ("books", &["id", "shelf", "title", "pages", "rating"][..]),
            ("editions", &["book", "number", "text"][..]),
            ("loans", &["book", "person", "at"][..]),
        ] {
            let mut select = Select::columns(table, columns);
            for c in columns {
                select = select.order(Sort::asc(col(c)));
            }
            all.extend(values(c.select(&select)?));
        }
        Ok(all)
    })
    .await
    .expect("snapshot")
}

async fn constraints_refuse_and_change_nothing(db: Database) {
    use ConstraintKind::*;
    seed(&db).await;
    write(&db, |c| {
        c.insert(
            &Insert::new("editions")
                .value("book", 1)
                .value("number", 1)
                .value("text", "one"),
        )
    })
    .await
    .expect("constraints: add an edition");
    refuses(&db, "a second shelf named s1", PrimaryKey, |c| {
        c.insert(&Insert::new("shelves").value("name", "s1"))
            .map(drop)
    })
    .await;
    refuses(&db, "a second person named amber", Unique, |c| {
        c.insert(&person("amber")).map(drop)
    })
    .await;
    refuses(&db, "a book with no title", NotNull, |c| {
        c.insert(&Insert::new("books").value("shelf", "s1"))
            .map(drop)
    })
    .await;
    refuses(&db, "a title set to null", NotNull, |c| {
        c.update(&Update::new("books", Condition::True).set("title", Value::Null))
            .map(drop)
    })
    .await;
    refuses(&db, "a book of no pages", Positive, |c| {
        c.insert(
            &Insert::new("books")
                .value("shelf", "s1")
                .value("title", "x")
                .value("pages", 0),
        )
        .map(drop)
    })
    .await;
    refuses(&db, "an edition changed", Immutable, |c| {
        c.update(&Update::new("editions", Condition::True).set("text", "two"))
            .map(drop)
    })
    .await;
    refuses(&db, "an edition replaced", Immutable, |c| {
        c.save(
            &Insert::new("editions")
                .value("book", 1)
                .value("number", 1)
                .value("text", "two"),
        )
    })
    .await;
    refuses(&db, "an edition deleted", Immutable, |c| {
        c.delete(&Delete::new("editions", Condition::True))
            .map(drop)
    })
    .await;
    refuses(&db, "a second edition 1", PrimaryKey, |c| {
        c.insert(
            &Insert::new("editions")
                .value("book", 1)
                .value("number", 1)
                .value("text", "x"),
        )
        .map(drop)
    })
    .await;
    refuses(&db, "a shelf deleted", NoDelete, |c| {
        c.delete(&Delete::new(
            "shelves",
            Condition::Eq(col("name"), "s3".into()),
        ))
        .map(drop)
    })
    .await;
    refuses(&db, "a book moved to another shelf", Fixed, |c| {
        c.update(&Update::new("books", Condition::Eq(col("id"), 1.into())).set("shelf", "s2"))
            .map(drop)
    })
    .await;
    // Setting a fixed column to the value it has is no change.
    let same = write(&db, |c| {
        c.update(&Update::new("books", Condition::Eq(col("id"), 1.into())).set("shelf", "s1"))
    })
    .await;
    assert_eq!(
        same,
        Ok(1),
        "constraints: a fixed column set to its own value"
    );
    // An update refused for one row changes no row.
    refuses(
        &db,
        "pages set to zero where there were none",
        Positive,
        |c| {
            c.update(&Update::new("books", Condition::True).set_if_null("pages", 0))
                .map(drop)
        },
    )
    .await;
}

async fn references_hold_through_changes_and_deletes(db: Database) {
    use ConstraintKind::Reference;
    seed(&db).await;
    write(&db, |c| {
        c.insert(
            &Insert::new("loans")
                .value("book", 1)
                .value("person", "brave")
                .value("at", "t"),
        )
    })
    .await
    .expect("references: a loan");
    refuses(&db, "a book on no shelf", Reference, |c| {
        c.insert(
            &Insert::new("books")
                .value("shelf", "nowhere")
                .value("title", "x"),
        )
        .map(drop)
    })
    .await;
    refuses(&db, "a shelf owned by nobody known", Reference, |c| {
        c.insert(
            &Insert::new("shelves")
                .value("name", "s9")
                .value("owner", "zed"),
        )
        .map(drop)
    })
    .await;
    refuses(&db, "a loan to nobody known", Reference, |c| {
        c.update(&Update::new("loans", Condition::True).set("person", "zed"))
            .map(drop)
    })
    .await;
    refuses(&db, "a person with a loan deleted", Reference, |c| {
        c.delete(&Delete::new(
            "people",
            Condition::Eq(col("name"), "brave".into()),
        ))
        .map(drop)
    })
    .await;
    refuses(&db, "a person with a shelf renamed", Reference, |c| {
        c.update(
            &Update::new("people", Condition::Eq(col("name"), "amber".into())).set("name", "ann"),
        )
        .map(drop)
    })
    .await;
    // The exception value and null name no row and need none.
    let ok = write(&db, |c| {
        c.insert(
            &Insert::new("shelves")
                .value("name", "s8")
                .value("owner", "library"),
        )?;
        c.update(
            &Update::new("shelves", Condition::Eq(col("name"), "s8".into()))
                .set("owner", Value::Null),
        )
    })
    .await;
    assert_eq!(ok, Ok(1), "references: the exception value and null");
}

async fn conditions_answer_as_the_contract_says(db: Database) {
    seed(&db).await;
    let titles = |filter: Condition| {
        let db = db.clone();
        async move {
            read(&db, move |c| {
                c.select(
                    &Select::columns("books", &["title"])
                        .filter(filter)
                        .order(Sort::asc(col("title"))),
                )
            })
            .await
            .map(texts)
            .expect("conditions: select")
        }
    };
    let pages = || col("pages");
    assert_eq!(
        titles(Condition::True).await,
        ["Alpha", "Beta", "Gamma"],
        "True"
    );
    assert_eq!(
        titles(Condition::Eq(pages(), 10.into())).await,
        ["Alpha"],
        "Eq"
    );
    // Two-valued logic: Beta's pages are null, so every comparison is
    // false and its negation true.
    assert_eq!(
        titles(Condition::Ne(pages(), 10.into())).await,
        ["Gamma"],
        "Ne"
    );
    assert_eq!(
        titles(Condition::not(Condition::Eq(pages(), 10.into()))).await,
        ["Beta", "Gamma"],
        "Not Eq holds where the column is null"
    );
    assert_eq!(
        titles(Condition::Lt(pages(), 30.into())).await,
        ["Alpha"],
        "Lt"
    );
    assert_eq!(
        titles(Condition::Le(pages(), 30.into())).await,
        ["Alpha", "Gamma"],
        "Le"
    );
    assert_eq!(
        titles(Condition::Gt(pages(), 10.into())).await,
        ["Gamma"],
        "Gt"
    );
    assert_eq!(
        titles(Condition::Ge(pages(), 10.into())).await,
        ["Alpha", "Gamma"],
        "Ge"
    );
    assert_eq!(titles(Condition::IsNull(pages())).await, ["Beta"], "IsNull");
    assert_eq!(
        titles(Condition::NotNull(pages())).await,
        ["Alpha", "Gamma"],
        "NotNull"
    );
    assert_eq!(
        titles(Condition::OneOf(
            col("id"),
            vec![1.into(), 3.into(), 99.into()]
        ))
        .await,
        ["Alpha", "Gamma"],
        "OneOf"
    );
    assert!(
        titles(Condition::OneOf(col("id"), vec![])).await.is_empty(),
        "OneOf none"
    );
    assert_eq!(
        titles(Condition::Any(vec![
            Condition::Eq(col("title"), "Beta".into()),
            Condition::Eq(pages(), 30.into()),
        ]))
        .await,
        ["Beta", "Gamma"],
        "Any"
    );
    assert!(
        titles(Condition::Any(vec![])).await.is_empty(),
        "Any of none"
    );
    assert_eq!(
        titles(Condition::All(vec![
            Condition::NotNull(pages()),
            Condition::Lt(pages(), 20.into()),
        ]))
        .await,
        ["Alpha"],
        "All"
    );
    assert_eq!(titles(Condition::All(vec![])).await.len(), 3, "All of none");
}

async fn text_compares_by_code_point_and_folds_only_ascii(db: Database) {
    let names = ["b", "B", "a_c", "a%c", "abc", "é", "É", "z", "中"];
    write(&db, move |c| {
        for name in names {
            c.insert(&person(name))?;
        }
        Ok(())
    })
    .await
    .expect("text: insert");
    let names = |filter: Condition| {
        let db = db.clone();
        async move {
            read(&db, move |c| {
                c.select(
                    &Select::columns("people", &["name"])
                        .filter(filter)
                        .order(Sort::asc(col("name"))),
                )
            })
            .await
            .map(texts)
            .expect("text: select")
        }
    };
    assert_eq!(
        names(Condition::True).await,
        ["B", "a%c", "a_c", "abc", "b", "z", "É", "é", "中"],
        "text orders by code point"
    );
    assert_eq!(
        names(Condition::Gt(col("name"), "z".into())).await,
        ["É", "é", "中"],
        "text compares by code point"
    );
    assert_eq!(
        names(Condition::EqIgnoringAsciiCase(col("name"), "b".into())).await,
        ["B", "b"],
        "ASCII letters fold"
    );
    assert_eq!(
        names(Condition::EqIgnoringAsciiCase(col("name"), "é".into())).await,
        ["é"],
        "other letters do not fold"
    );
    assert_eq!(
        names(Condition::ContainsIgnoringAsciiCase(
            col("name"),
            "_".into()
        ))
        .await,
        ["a_c"],
        "_ is no wildcard"
    );
    assert_eq!(
        names(Condition::ContainsIgnoringAsciiCase(
            col("name"),
            "%".into()
        ))
        .await,
        ["a%c"],
        "% is no wildcard"
    );
    assert_eq!(
        names(Condition::ContainsIgnoringAsciiCase(
            col("name"),
            "BC".into()
        ))
        .await,
        ["abc"],
        "contains folds ASCII"
    );
    assert_eq!(
        names(Condition::ContainsIgnoringAsciiCase(
            col("name"),
            "É".into()
        ))
        .await,
        ["É"],
        "contains does not fold other letters"
    );
}

async fn references_are_followed(db: Database) {
    seed(&db).await;
    write(&db, |c| {
        c.update(
            &Update::new("people", Condition::Eq(col("name"), "amber".into())).set("city", "Haifa"),
        )
    })
    .await
    .expect("through: set a city");
    let rows = read(&db, |c| {
        c.select(
            &Select::new(
                "shelves",
                vec![
                    Field::Value(col("name")),
                    Field::Value(through("owner", "city")),
                ],
            )
            .order(Sort::asc(col("name"))),
        )
    })
    .await
    .expect("through: select");
    assert_eq!(
        values(rows),
        vec![
            // The exception value names no row.
            vec![Value::from("s1"), Value::Null],
            vec![Value::from("s2"), Value::from("Haifa")],
            // Neither does null.
            vec![Value::from("s3"), Value::Null],
        ],
        "through: fields"
    );
    let rows = read(&db, |c| {
        c.select(
            &Select::columns("shelves", &["name"])
                .filter(Condition::Eq(through("owner", "city"), "Haifa".into())),
        )
    })
    .await
    .expect("through: filter");
    assert_eq!(texts(rows), ["s2"], "through: filter");
}

async fn related_rows_give_counts_maxima_newest_and_existence(db: Database) {
    seed(&db).await;
    write(&db, |c| {
        c.insert(
            &Insert::new("loans")
                .value("book", 1)
                .value("person", "amber")
                .value("at", "2"),
        )?;
        c.insert(
            &Insert::new("loans")
                .value("book", 1)
                .value("person", "brave")
                .value("at", "1"),
        )?;
        c.insert(
            &Insert::new("loans")
                .value("book", 3)
                .value("person", "brave")
                .value("at", "3"),
        )?;
        Ok(())
    })
    .await
    .expect("related: loans");
    let loans = || Related::new("loans", "book", "id");
    let rows = read(&db, move |c| {
        c.select(
            &Select::new(
                "books",
                vec![
                    Field::Value(col("title")),
                    Field::Count(loans()),
                    Field::Count(loans().filter(Condition::Eq(col("person"), "brave".into()))),
                    Field::Max(loans(), "at"),
                    Field::Newest {
                        related: loans(),
                        by: "at",
                        column: "person",
                    },
                    Field::Exists(loans().filter(Condition::Eq(col("person"), "amber".into()))),
                ],
            )
            .order(Sort::asc(col("id"))),
        )
    })
    .await
    .expect("related: select");
    assert_eq!(
        values(rows),
        vec![
            vec![
                "Alpha".into(),
                2.into(),
                1.into(),
                "2".into(),
                "amber".into(),
                true.into()
            ],
            vec![
                "Beta".into(),
                0.into(),
                0.into(),
                Value::Null,
                Value::Null,
                false.into()
            ],
            vec![
                "Gamma".into(),
                1.into(),
                1.into(),
                "3".into(),
                "brave".into(),
                false.into()
            ],
        ],
        "related: fields"
    );
    let rows = read(&db, move |c| {
        c.select(
            &Select::columns("books", &["title"])
                .filter(Condition::exists(
                    loans().filter(Condition::Eq(col("person"), "brave".into())),
                ))
                .filter(Condition::not(Condition::exists(
                    loans().filter(Condition::Eq(col("person"), "amber".into())),
                )))
                .order(Sort::asc(col("title"))),
        )
    })
    .await
    .expect("related: exists");
    assert_eq!(texts(rows), ["Gamma"], "related: exists and not exists");
    // A related row through a reference that names no row: shelves owned
    // by the library have no owner to relate.
    let rows = read(&db, |c| {
        c.select(
            &Select::new(
                "shelves",
                vec![Field::Count(Related::new("people", "name", "owner"))],
            )
            .order(Sort::asc(col("name"))),
        )
    })
    .await
    .expect("related: through an exception value");
    assert_eq!(
        values(rows),
        vec![vec![0.into()], vec![1.into()], vec![0.into()]],
        "related: a null or excepted link relates nothing"
    );
}

async fn rows_come_in_order_with_paging_and_limits(db: Database) {
    write(&db, |c| {
        for (name, nick) in [("d", None), ("a", Some("z")), ("c", None), ("b", Some("y"))] {
            c.insert(&person(name).value("nick", nick))?;
        }
        Ok(())
    })
    .await
    .expect("order: insert");
    let names = |select: Select| {
        let db = db.clone();
        async move {
            read(&db, move |c| c.select(&select))
                .await
                .map(texts)
                .expect("order: select")
        }
    };
    let people = || Select::columns("people", &["name"]);
    assert_eq!(
        names(people().order(Sort::desc(col("name")))).await,
        ["d", "c", "b", "a"],
        "desc"
    );
    assert_eq!(
        names(
            people()
                .order(Sort::asc(col("nick")))
                .order(Sort::asc(col("name")))
        )
        .await,
        ["c", "d", "b", "a"],
        "nulls first ascending"
    );
    assert_eq!(
        names(
            people()
                .order(Sort::desc(col("nick")))
                .order(Sort::asc(col("name")))
        )
        .await,
        ["a", "b", "c", "d"],
        "nulls last descending"
    );
    assert_eq!(
        names(people().order(Sort::Asc(OrderBy::FirstOf(col("nick"), col("name"))))).await,
        ["c", "d", "b", "a"],
        "first of nick and name"
    );
    assert_eq!(
        names(
            people()
                .order(Sort::asc(col("name")))
                .after(vec!["b".into()])
                .limit(1)
        )
        .await,
        ["c"],
        "after and limit"
    );
    assert_eq!(
        names(
            people()
                .order(Sort::desc(col("name")))
                .after(vec!["b".into()])
        )
        .await,
        ["a"],
        "after, descending"
    );
    assert_eq!(
        names(
            people()
                .order(Sort::asc(col("active")))
                .order(Sort::asc(col("name")))
                .after(vec![true.into(), "a".into()])
        )
        .await,
        ["b", "c", "d"],
        "after over two sorts"
    );
    assert!(names(people().limit(0)).await.is_empty(), "limit 0");
}

async fn counts_and_groups(db: Database) {
    seed(&db).await;
    write(&db, |c| {
        c.insert(
            &Insert::new("books")
                .value("shelf", "s2")
                .value("title", "Delta"),
        )?;
        c.insert(
            &Insert::new("books")
                .value("shelf", "s3")
                .value("title", "Epsilon"),
        )?;
        Ok(())
    })
    .await
    .expect("groups: more books");
    let (count, groups) = read(&db, |c| {
        let count = c.count(&Count {
            table: "books",
            filter: Condition::NotNull(col("pages")),
        })?;
        let groups = c.group(&Group {
            table: "books",
            by: col("shelf"),
            also: vec![through("shelf", "owner")],
            max: "id",
            filter: Condition::not(Condition::Eq(col("title"), "Gamma".into())),
        })?;
        Ok((count, groups))
    })
    .await
    .expect("groups: read");
    assert_eq!(count, 2, "count");
    assert_eq!(
        values(groups),
        vec![
            vec!["s3".into(), Value::Null, 1.into(), 5.into()],
            vec!["s2".into(), "amber".into(), 1.into(), 4.into()],
            vec!["s1".into(), "library".into(), 2.into(), 2.into()],
        ],
        "groups, largest key first"
    );
}

async fn inserts_saves_updates_and_deletes_say_what_they_did(db: Database) {
    seed(&db).await;
    let said = write(&db, |c| {
        let loan = || {
            Insert::new("loans")
                .value("book", 1)
                .value("person", "amber")
        };
        let first = c.insert_unless_present(&loan().value("at", "1"))?;
        let again = c.insert_unless_present(&loan().value("at", "2"))?;
        c.save(&loan().value("at", "3"))?;
        c.save(
            &Insert::new("loans")
                .value("book", 2)
                .value("person", "amber")
                .value("at", "4"),
        )?;
        let none = c.update(
            &Update::new("people", Condition::Eq(col("name"), "zed".into())).set("city", "x"),
        )?;
        let all = c.update(&Update::new("people", Condition::True).set_if_null("city", "Acre"))?;
        let again_all = c.update(
            &Update::new("people", Condition::Eq(col("name"), "amber".into()))
                .set_if_null("city", "Eilat"),
        )?;
        let deleted = c.delete(&Delete::new("loans", Condition::Eq(col("book"), 2.into())))?;
        let deleted_none = c.delete(&Delete::new("loans", Condition::Eq(col("book"), 2.into())))?;
        Ok((first, again, none, all, again_all, deleted, deleted_none))
    })
    .await
    .expect("changes: write");
    assert_eq!(
        said,
        (true, false, 0, 3, 1, 1, 0),
        "what each change says it did"
    );
    let rows = read(&db, |c| {
        let mut rows = c.select(&Select::columns("loans", &["book", "person", "at"]))?;
        rows.extend(
            c.select(
                &Select::columns("people", &["city"])
                    .filter(Condition::Eq(col("name"), "amber".into())),
            )?,
        );
        Ok(rows)
    })
    .await
    .expect("changes: read");
    assert_eq!(
        values(rows),
        vec![
            vec![1.into(), "amber".into(), "3".into()],
            vec!["Acre".into()]
        ],
        "save replaced the loan; set-if-null kept the first city"
    );
}

async fn a_failed_write_keeps_nothing(db: Database) {
    seed(&db).await;
    let before = snapshot(&db).await;
    let failed: Result<(), DbError> = write(&db, |c| {
        c.insert(&person("dana"))?;
        c.update(&Update::new("books", Condition::True).set("title", "same"))?;
        Err(DbError::Backend("the caller gave up".into()))
    })
    .await;
    assert_eq!(
        failed,
        Err(DbError::Backend("the caller gave up".into())),
        "the caller's error comes back"
    );
    assert_eq!(snapshot(&db).await, before, "a failed write kept a change");
}

async fn reads_cannot_change_anything(db: Database) {
    seed(&db).await;
    let before = snapshot(&db).await;
    let refused = read(&db, |c| {
        Ok([
            c.insert(&person("dana")).map(drop),
            c.insert_unless_present(&person("erin")).map(drop),
            c.save(&person("fay")),
            c.update(&Update::new("books", Condition::True).set("title", "x"))
                .map(drop),
            c.delete(&Delete::new("loans", Condition::True)).map(drop),
        ])
    })
    .await
    .expect("read-only: read");
    for result in refused {
        assert_eq!(result, Err(DbError::ReadOnly), "a change inside a read");
    }
    assert_eq!(snapshot(&db).await, before, "a read changed something");
}

async fn writes_run_as_if_alone(db: Database) {
    // Each write counts the people and adds one named for the count: if
    // two writes saw the same count, the unique name would refuse one.
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            write(&db, |c| {
                let n = c.count(&Count {
                    table: "people",
                    filter: Condition::True,
                })?;
                c.insert(&person(&format!("p{n}"))).map(drop)
            })
            .await
        }));
    }
    for task in tasks {
        task.await
            .expect("isolation: task")
            .expect("isolation: two writes saw the same count");
    }
    let n = read(&db, |c| {
        c.count(&Count {
            table: "people",
            filter: Condition::True,
        })
    })
    .await
    .expect("isolation: count");
    assert_eq!(n, 8, "isolation: every write added its row");
}

async fn questions_that_do_not_fit_the_schema_are_refused(db: Database) {
    type Check = Box<dyn FnOnce(&dyn Connection) -> Result<(), DbError> + Send>;
    let checks: Vec<(&str, Check)> = vec![
        (
            "no such table",
            Box::new(|c| c.select(&Select::columns("nowhere", &["x"])).map(drop)),
        ),
        (
            "no such column",
            Box::new(|c| c.select(&Select::columns("people", &["age"])).map(drop)),
        ),
        (
            "through a column that is no reference",
            Box::new(|c| {
                c.select(&Select::new(
                    "books",
                    vec![Field::Value(through("title", "x"))],
                ))
                .map(drop)
            }),
        ),
        (
            "a value of the wrong type",
            Box::new(|c| {
                c.select(
                    &Select::columns("people", &["name"])
                        .filter(Condition::Eq(col("name"), 1.into())),
                )
                .map(drop)
            }),
        ),
        (
            "a comparison with null",
            Box::new(|c| {
                c.select(
                    &Select::columns("people", &["name"])
                        .filter(Condition::Eq(col("nick"), Value::Null)),
                )
                .map(drop)
            }),
        ),
        (
            "a key given on insert",
            Box::new(|c| c.insert(&person("x").value("id", 9)).map(drop)),
        ),
        (
            "after without a sort for each value",
            Box::new(|c| {
                c.select(&Select::columns("people", &["name"]).after(vec!["a".into()]))
                    .map(drop)
            }),
        ),
    ];
    for (what, check) in checks {
        let result = write(&db, move |c| Ok(check(c)))
            .await
            .expect("schema: write");
        assert!(
            matches!(result, Err(DbError::Schema(_))),
            "{what}: expected a schema error, got {result:?}"
        );
    }
}

async fn a_change_can_be_made_to_fail(db: Database) {
    seed(&db).await;
    db.fail_next_change_to("loans");
    let loan = || {
        Insert::new("loans")
            .value("book", 1)
            .value("person", "amber")
            .value("at", "1")
    };
    let first = write(&db, move |c| c.insert(&loan()).map(drop)).await;
    assert!(
        matches!(first, Err(DbError::Backend(_))),
        "the next change fails: {first:?}"
    );
    let second = write(&db, move |c| c.insert(&loan()).map(drop)).await;
    assert_eq!(second, Ok(()), "only the next change fails");
}

async fn migrations_apply_once_in_order_and_fill(new_place: &impl Fn() -> Box<dyn Place>) {
    let place = new_place();
    assert_eq!(
        place
            .pending(&[library()])
            .expect("migrations: pending on an empty place"),
        vec![crate::Upgrade {
            set: "library",
            from: 0,
            to: 2
        }],
        "an empty place upgrades from 0"
    );
    let db = place
        .open(&[library_at_one()])
        .expect("migrations: open at version 1");
    write(&db, |c| {
        c.insert(&person("amber"))?;
        c.insert(&person("brave").value("nick", "b"))?;
        c.insert(&Insert::new("shelves").value("name", "s1"))?;
        c.insert(
            &Insert::new("books")
                .value("shelf", "s1")
                .value("title", "Alpha"),
        )
    })
    .await
    .expect("migrations: rows at version 1");
    assert_eq!(
        place
            .pending(&[library()])
            .expect("migrations: pending at version 1"),
        vec![crate::Upgrade {
            set: "library",
            from: 1,
            to: 2
        }],
        "pending at version 1"
    );
    let db = place
        .open(&[library()])
        .expect("migrations: open at version 2");
    let rows = read(&db, |c| {
        let mut rows = c.select(
            &Select::columns("people", &["name", "nick", "city"]).order(Sort::asc(col("name"))),
        )?;
        rows.extend(c.select(&Select::columns("books", &["title", "rating"]))?);
        Ok(rows)
    })
    .await
    .expect("migrations: read at version 2");
    assert_eq!(
        values(rows),
        vec![
            vec!["amber".into(), "amber".into(), Value::Null],
            vec!["brave".into(), "b".into(), Value::Null],
            vec!["Alpha".into(), 3.into()],
        ],
        "added columns get their defaults, and the fill ran"
    );
    assert!(
        place
            .pending(&[library()])
            .expect("migrations: pending")
            .is_empty(),
        "up to date"
    );
    // Opening again applies nothing: the fill would otherwise have run on
    // a nickname set since.
    write(&db, |c| c.insert(&person("calm")).map(drop))
        .await
        .expect("migrations: a person with no nickname");
    let db = place.open(&[library()]).expect("migrations: open again");
    let nick = read(&db, |c| {
        c.first(
            &Select::columns("people", &["nick"]).filter(Condition::Eq(col("name"), "calm".into())),
        )
    })
    .await
    .expect("migrations: read again");
    assert_eq!(
        nick.map(Row::into_values),
        Some(vec![Value::Null]),
        "a migration ran twice"
    );
}

fn failing_fill(_: &dyn Connection) -> Result<(), DbError> {
    Err(DbError::Backend("the fill failed".into()))
}

async fn a_failing_migration_changes_nothing(new_place: &impl Fn() -> Box<dyn Place>) {
    let place = new_place();
    let db = place
        .open(&[library_at_one()])
        .expect("failing migration: open at 1");
    seed_at_one(&db).await;
    let mut broken = library();
    broken.migrations[1].push(Step::Fill {
        name: "fails",
        run: failing_fill,
    });
    let result = place.open(&[broken]);
    assert!(
        matches!(result, Err(DbError::Backend(_))),
        "failing migration: {:?}",
        result.err()
    );
    assert_eq!(
        place
            .pending(&[library()])
            .expect("failing migration: pending"),
        vec![crate::Upgrade {
            set: "library",
            from: 1,
            to: 2
        }],
        "a failed migration was recorded"
    );
    // The columns it added are gone with it: version 2 applies cleanly.
    let db = place
        .open(&[library()])
        .expect("failing migration: then the good one");
    let nicks = read(&db, |c| c.select(&Select::columns("people", &["nick"]))).await;
    assert_eq!(
        nicks.map(values),
        Ok(vec![vec!["amber".into()]]),
        "the good migration's fill"
    );
}

async fn seed_at_one(db: &Database) {
    write(db, |c| c.insert(&person("amber")).map(drop))
        .await
        .expect("seed at version 1");
}

async fn a_database_newer_than_its_sets_is_refused(new_place: &impl Fn() -> Box<dyn Place>) {
    let place = new_place();
    place.open(&[library()]).expect("newer: open at 2");
    let result = place.open(&[library_at_one()]);
    assert!(
        matches!(
            result,
            Err(DbError::Newer {
                set: "library",
                found: 2,
                known: 1
            })
        ),
        "newer: {:?}",
        result.err()
    );
}

async fn sets_are_recorded_apart(new_place: &impl Fn() -> Box<dyn Place>) {
    let place = new_place();
    place.open(&[library_at_one()]).expect("sets: library at 1");
    let reviews = Set {
        name: "reviews",
        migrations: vec![vec![Step::AddTable(Table::new(
            "reviews",
            vec![
                Column::key("id"),
                Column::int("book").references("books", "id"),
                Column::text("text"),
            ],
            vec!["id"],
        ))]],
    };
    assert_eq!(
        place
            .pending(&[library(), reviews.clone()])
            .expect("sets: pending"),
        vec![
            crate::Upgrade {
                set: "library",
                from: 1,
                to: 2
            },
            crate::Upgrade {
                set: "reviews",
                from: 0,
                to: 1
            },
        ],
        "each set has its own version"
    );
    let db = place
        .open(&[library(), reviews.clone()])
        .expect("sets: open both");
    let id = write(&db, |c| {
        c.insert(&Insert::new("shelves").value("name", "s"))?;
        let book = c.insert(&Insert::new("books").value("shelf", "s").value("title", "t"))?;
        c.insert(
            &Insert::new("reviews")
                .value("book", book)
                .value("text", "good"),
        )
    })
    .await
    .expect("sets: a review of a book, across sets, in one write");
    assert_eq!(id, Some(1), "sets: the review's key");
    assert!(
        place
            .pending(&[library(), reviews])
            .expect("sets: pending")
            .is_empty(),
        "both up to date"
    );
}
