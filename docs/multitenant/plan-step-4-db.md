# Step 4: a database-neutral store

Status: design, 2026-11-09, by kkt-mt-step1 for Sam, asked through sage-cardinal in #kkt-dev (message 1172). Nothing here is implemented. It replaces the step 4 row of `plan-phase-1.md` as first written ("a new crate, `kkt-db`, owning `Db`, the write lock and reader pool, and the migration runner"). Code facts are from `next` at `94c4187`.

## Summary

Sam's shape (message 1172): kkt-store describes the core's data and asks for what it needs in kkt-db's terms, with no SQL; kkt-db is a contract that knows no product and no particular database; kkt-db-sqlite is SQLite's implementation of it, and the only crate that knows SQLite. A later kkt-db-postgres replaces it with no change to kkt-store, or to pro's store.

This document derives kkt-db's operations from what kkt-store does today: 47 reads, 26 writes, 13 indexes, 17 triggers and one full-text index. It covers search, constraints and migrations as kkt-db concepts. It describes the upgrade from 0.10.0 under Sam's baseline reset, lists what an agent could notice, and breaks the work into seven pull requests. The questions for Sam are at the end.

The short version:

- kkt-db's query language has about twenty operations. Each one is something a store method needs today; none is general SQL.
- Search tokenizes in kkt-db's own Rust code, so every backend indexes the same words and returns the same excerpts. SQLite's FTS5 only matches whole tokens that kkt-db has already made.
- The 17 triggers become six kinds of declared constraint, plus the search index.
- The schema starts again from a baseline equal to 0.10.0's tables. A 0.10.0 database is rebuilt into the baseline when it is first opened. A fresh install and an upgraded one end up with the same schema, byte for byte.

## 1. The three crates

| Crate | Owns | Must not | Features |
|---|---|---|---|
| `kkt-db` | The contract: values and column types; declaring tables, keys, references, indexes, constraints and a search index; the query and change operations; search (tokens, matching, excerpts); transactions; migrations and their record; `Database`, the handle a store is given; the conformance suite every backend must pass | Know a product's tables; name a database or import a database library | `testing`: `kkt_db::testing`, the conformance suite, and a fault hook that makes the next write to a named table fail |
| `kkt-db-sqlite` | SQLite's implementation: SQL generated from the operations; the writer connection and reader pool; constraints as generated triggers; the search index as FTS5 over kkt-db's tokens; the migration record; adopting an existing SQLite file; `pending_upgrade` and `copy_database` | Know a product's tables; let a rusqlite type out of the crate | `real`: open a file, `pending_upgrade`, `copy_database`. `testing`: an in-memory database |
| `kkt-store` | The core's data: its records; its schema declared in kkt-db's terms (tables, relations, protective constraints, the search index); the baseline and every migration after it; `Reads` and `Writes`, implemented with kkt-db's operations; `swarms_stay_apart` | Contain SQL; import rusqlite or kkt-db-sqlite | `testing`: `testing::swarms_stay_apart` and a store over an in-memory backend |

Who chooses the backend: kkt-server and the program. They open a `kkt_db_sqlite` database with kkt-store's schema and hand the resulting `kkt_db::Database` to `kkt_store::Db::new`. kkt-store depends on kkt-db only. That is dependency injection as PRINCIPLES.md asks, and it is how kkt-pro will open one database holding the core's set and its own.

`kkt_store::Db` keeps its name and its two methods, `read` and `write`, which run a closure over `&dyn Reads` or `&dyn Writes`. kkt-rules, kkt-server and the tests do not change how they call the store.

The connection: kkt-db defines an object-safe `Connection` trait with one method per operation. kkt-store's `Sql` (renamed `Store`) implements `Reads` and `Writes` over a `&dyn Connection`, exactly where it implements them over a rusqlite connection today. A write closure runs inside one transaction and commits only if it succeeds, as today. It also runs **as if it were the only write**: no other write commits between its reads and its changes. Three things in kkt-store depend on this:
- checking that a handle is free, then inserting it;
- computing a lane's next version, then inserting it;
- `require_agent` and `require_channel`, then the insert they guard.

SQLite gives this with its one writer connection. A Postgres backend must run writes at serializable isolation or take a lock that has the same effect; read committed would break all three. A conformance test runs two concurrent writes that each read a count and insert the next number, and requires that both numbers are distinct.

`Connection` is synchronous. `Database::read` and `Database::write` run the closure on Tokio's blocking pool, as `run_blocking` does today, so a backend never sees an async caller.

A read closure, unlike today, sees one consistent state across all its statements. Today each statement on a reader sees the latest commit, so a read of several steps could see two states. The SQLite backend gets this by wrapping each read in a deferred transaction. It is cheap but not free: while a read is open, SQLite cannot fold the log back into the database file past that read's snapshot. Reads are short, so this matters only if one stalls. Postgres's repeatable read gives the same guarantee.

## 2. kkt-db's query language

The rule (Sam's decision 1): exactly what kkt-store needs, nothing general. Every operation below is used by a store method today; the method is named.

**Values and columns.** These column types:
- text;
- 64-bit integer;
- bool;
- bytes;
- an *increasing key*, an integer the backend assigns, always above every key the table has ever had.

Any of them can be nullable, and a column can have a default. Times stay as text in RFC 3339 UTC, as today, and compare as text.

Text compares and orders by Unicode code point. That is SQLite's default (byte order of UTF-8); a Postgres backend must use the `C` collation. The conformance suite checks it.

**Reads.** One operation, `Select`, over one table, built from these parts:

| Part | What it is | Used by, today |
|---|---|---|
| column | A column of the row | every read |
| through a reference | A column of the row a reference points to | messages with their channel's name (`MESSAGE_COLUMNS`); memory and memberships bound to a swarm through their avatar or channel |
| count related | How many rows of another table refer to this one, with conditions | member counts; new-message counts in `channel_summary` |
| max related | The largest value of a column among related rows | an avatar's lane's current version (`AGENT_COLUMNS`); a lane's newest version (`LANE_SELECT`) |
| newest related | Columns of the related row with the largest increasing key | `channel_use`'s last message time and author |
| exists related | Whether a related row matching conditions exists | `viewer_is_member`; `shared_with` in `channel_ids` |
| equals, not equals, less, at most, greater, at least | Comparisons to a value | IDs above `after_id`, times since, names after a cursor |
| is null, is not null | | `revoked_at IS NULL`; `retired_at IS NULL` |
| one of | A column among listed values | `channel_ids` filters (`json_each` today) |
| equals ignoring ASCII case | | `agent_name_taken` (`COLLATE NOCASE`) |
| contains ignoring ASCII case | Literal text anywhere in a column; no wildcards | agent and channel text filters (`LIKE … ESCAPE` today) |
| all of, any of, not | Combining conditions | the channel scope (below) |
| order by | Columns or the first non-null of two, ascending or descending | `agents_by_last_seen` (`coalesce(last_seen, created_at)`) |
| after | Rows after given values in the order (keyset paging) | `(c.name, c.id) > (:cname, :cid)` |
| limit | | every paged list |
| count | The number of rows matching | `active_agent_count`, `count_newer_messages` |
| group | For each value of a column: the count and the largest key, ordered by the largest key, newest first | `activity`, `any_activity` (`ORDER BY max(m.id) DESC`) |

An optional filter is a condition left out, not `:x IS NULL OR …`. The backend sees only the conditions that apply.

**The channel scope.** Today `SCOPE_CHANNELS` computes a number for each channel's kind and tests a bit mask in SQL. In kkt-db terms, it is "any of" the allowed kinds. Each kind is "all of": hidden or not, open or not, a membership for the viewer exists or not, the owner is the viewer or not.

kkt-store builds this condition from the `ChannelScope` as it builds the mask today. `the_stores_channel_filters_agree_with_decide` keeps checking it against `decide`, and `decide` does not change.

**Changes.**

| Operation | What it does | Used by, today |
|---|---|---|
| insert | Adds a row; returns its increasing key if the table has one | messages, server runs, the server announcement (`last_insert_rowid`), and every other insert |
| insert unless present | Adds a row unless its key exists | `add_member` (`INSERT OR IGNORE`) |
| save | Adds a row, or replaces its other columns if its key exists | `save_memory_entry` (`ON CONFLICT … DO UPDATE`) |
| update | Sets columns on rows matching conditions; a column may be set only if it is null; returns how many rows matched | the `set_*` writes; `hide_agent` (`coalesce(revoked_at, ?)`); `hide_channel` |
| delete | Removes rows matching conditions; returns how many | `delete_memory_entry`, `remove_member`, the membership deletes in `hide_*` |

A lane's next version number stays a read and an insert in one write, as today.

**What is not in it.** Joins in general; expressions; arithmetic; SQL text of any kind; `rowid`.

The one use of `rowid` today is the tie-break in `memberships()`, `ORDER BY m.joined_at, m.rowid`. The baseline gives memberships an increasing key, filled from the rowid when an old database is adopted (section 5), so the order does not change.

**No third-party builder.** Sam allowed a SQL builder inside kkt-db-sqlite if it is wrapped. The operations are few enough that generating their SQL directly is smaller than wrapping a builder, so I propose none.

## 3. Search

Today:
- the rules split `query` at whitespace into words;
- kkt-store quotes each word and the phrase as FTS5 strings;
- FTS5's `unicode61` tokenizer splits them into tokens, ignoring case and Latin diacritics, so `PDF.js` becomes the adjacent pair "pdf js";
- `snippet(messages_fts, 0, '**', '**', '…', 12)` makes the excerpt.

The tool text tells agents how this behaves (`search_messages` in `agent-tools.toml`): words split at spaces and punctuation, matched ignoring case.

**kkt-db's search operation** (Sam's decision 2). A table declares one text column as searchable. A `Search` takes:
- a list of terms; each term is a run of tokens that must appear together and in order;
- the same conditions, order and limit as `Select`.

It returns the rows and, for each, an excerpt. The store keeps its query syntax: the rules give words and a phrase, and kkt-store turns each word, and the phrase, into one term. So `PDF.js` remains one term of two tokens, and nothing about FTS5's syntax reaches kkt-db.

**Tokens are made by kkt-db, in Rust**, not by the backend:
- A token is a run of letters, digits or private-use characters (Unicode categories L, N and Co). Everything else separates tokens.
- Tokens are case-folded, and diacritics are removed from Latin letters.

That is what FTS5's `unicode61` does by default, written down as our rule.

The SQLite backend:
- stores each message's tokens, joined by spaces, in an FTS5 table with no tokenizer work left to do;
- writes them in the same transaction as the message, in place of today's three sync triggers;
- declares the FTS5 table with the `ascii` tokenizer. kkt-db's tokens contain no ASCII separators, and `ascii` treats every non-ASCII character as part of a token, so FTS5 stores each token whole. `unicode61` would split some again by its Unicode 6.1 tables. A test stores tokens in several scripts and checks that FTS5's own vocabulary (`fts5vocab`) holds exactly them;
- asks FTS5 for whole-token phrase matches.

A Postgres backend would index the same tokens. Matching is then the same on every backend, because no backend tokenizes.

**A term with no tokens**, such as `--` or an emoji, is dropped. A search whose terms are all dropped matches nothing. That is what FTS5 does today: `"--"` alone matches nothing, and `"--" "hello"` matches what `"hello"` matches. The differential test includes such terms.

**Excerpts are made by kkt-db**, from the body and the matched tokens. They use the same rules as today: a window of 12 tokens, matches marked with `**`, and `…` where the window cuts text. Today FTS5 chooses the window; kkt-db will choose it the way FTS5 does. A differential test in kkt-db-sqlite runs both over a generated set of bodies and queries and requires the same excerpt.

**What an agent could notice:**

- *Tokens.* Ours are FTS5's rule, rewritten. The differential test covers ASCII, Latin with diacritics, Greek, Cyrillic, CJK, digits, emoji and punctuation. Differences in rarer characters are possible because the Unicode tables differ: FTS5's are fixed at Unicode 6.1, Rust's are current. Any difference the test finds is listed in the pull request.
- *Excerpts.* The same, except where the test finds that FTS5's window choice cannot be matched; those cases are listed too.
- *Results and order.* No change: the same matches, newest first, with the same filters.

## 4. Constraints

Today 17 triggers protect the data (20 were created, and 3 were dropped with a table rebuild in 0008). In kkt-db they become declared constraints that each backend must enforce, and the conformance suite tries to break each one.

| Constraint | Today's triggers | Tables |
|---|---|---|
| **rows never change or go**: inserted once, never updated, deleted or replaced | `*_no_update`, `*_no_delete`, `*_no_replacement` | `lane_versions`, `operator_instructions`, `swarms` (8 triggers) |
| **rows never go** | `lanes_no_delete` | `lanes` |
| **a column never changes** | `*_swarm_fixed` | `agents.swarm`, `channels.swarm` |
| **a reference, or one named value** that is no row: a foreign key that also accepts `operator` | `channels_creator_exists`, `channels_creator_update`, `messages_agent_exists` | `channels.created_by`, `messages.agent` |
| **search index kept current** (not a constraint: the search index of section 3) | `messages_fts_insert`, `_update`, `_delete` | `messages.body` |

Alongside them, the ordinary declarations: primary keys, foreign keys, unique (`agents.token_hash`), not null, defaults, and positive (`version > 0`). The two `hidden IN (0, 1)` checks become the bool type.

kkt-db-sqlite turns the declared constraints into triggers it generates. A violation comes back as kkt-db's `Constraint` error, naming the constraint, which kkt-store maps to `StoreError::Database` as today. The rules never match on store errors, so nothing above the store changes.

The checks kkt-store makes in Rust stay in kkt-store, as reads in the same write: `require_agent` and `require_channel`, which raise `OutsideSwarm`.

The one test that writes raw SQL is `kkt-rules`'s failing-insert test, which adds a trigger that aborts inserts. It uses the fault hook of `kkt_db::testing` instead.

## 5. Migrations, the baseline and the upgrade from 0.10.0

**Described neutrally.** A migration is a list of additions, each one of:
- add a table;
- add a column (with its default for existing rows);
- add an index;
- add a constraint;
- add a search index;
- fill a column. kkt-db has no expressions, so a fill that computes values is a Rust function the migration names. It runs inside the migration's transaction over a `Connection`, reading rows and writing their new values with kkt-db's operations. Migration 0019, which copied handoff notes into memory entries, would be written this way.

A set of migrations has a name and an ordered list. kkt-db records, per set, how many have been applied; SQLite keeps that record in a table, `kkt_db_versions (name, version)`, and so would Postgres. This is the shape phase 2's named sets need, so phase 2 adds pro's set beside the core's and nothing is reshaped. The rule "migrations only add" stays, now enforced by the types: no operation removes or rewrites anything.

**The baseline** (Sam's decision 3). The core's set starts again:
- **Version 1** is 0.10.0's schema (`0021`), declared in kkt-db's terms, with two exceptions:
  - memberships gain their increasing key (section 2);
  - the two `handoff_note` columns on agents are left out. Migration 0019 copied them into memory entries, and nothing has read them since (question 3).
- **Version 2** is today's `0022`: the swarms table, the `swarm` columns and their constraints.

The 22 SQL files in `crates/kkt-store/migrations/` stop being migrations. They move under `tests/migrations/` as what the fixture test uses to build a 0.10.0 database.

**Opening a database.** kkt-db-sqlite reads the file's `user_version` and the version record:

| What it finds | What happens |
|---|---|
| No file, or no tables | Creates the schema from the declarations. Records the core at its latest version. |
| The version record | Applies the set's later migrations, as today. |
| `user_version` 21 (0.10.0) or 22 (built from `next` since step 1) | **Adopts** the file (below), then applies the migrations after the adopted version. |
| `user_version` 1 to 20 (before 0.10.0) | Refuses, changing nothing, with: "this database is from a kkt version older than 0.10.0; upgrade it with kkt 0.10 first, then with this version". |
| Anything else | Refuses, changing nothing, naming what it found. |

**Adopting** is generic SQLite work and knows no product. It runs in one transaction:
1. For each declared table, build a new table from the declaration and copy the rows, column by column by name. A declared increasing key that the old table lacks is filled from the old rowid. Each increasing key's counter starts above the largest value it ever reached: the old `sqlite_sequence` entry where there is one, otherwise the largest key. Message IDs only go up, across the upgrade too.
2. Drop the old tables, indexes and triggers.
3. Create the declared indexes, constraints and search index. The search index is rebuilt from every message with kkt-db's tokens.
4. Check foreign keys. Record the version. Set `user_version` to a fixed value meaning "recorded in `kkt_db_versions`".

Adoption refuses a file whose tables and columns do not match the declaration at version 21 or 22. Columns that the declaration left out on purpose are the exception, and are listed in it: today, the two handoff columns.

The pre-upgrade backup still runs first: `pending_upgrade` reports adoption as an upgrade, so `serve` copies the database before touching it, as today. The log line keeps its form and gives the old and new versions ("schema 21 to core 2").

**Fresh equals upgraded.** A new test (Sam's decision 3) creates one database fresh and builds another at 0.10.0 from the fixture and upgrades it. The two must have:
- the same `sqlite_master`, entry by entry, byte for byte;
- the same version record and `user_version`;
- the same pragmas that persist.

This holds because adoption rebuilds every table from the same declarations that a fresh install uses. Today's `ALTER TABLE ADD COLUMN` history would not survive it. Beside it, the fixture's pinned reads (`tests/migrations/v0_10_0_reads.txt`) must not change: every read of the store answers the same on the upgraded database.

**The migration tests from before 0.10.0.** About 18 tests in `tests/migrations.rs` build databases from 0.1.0 to 0.9.0 and check one migration each. Those versions are no longer upgraded directly, so those tests go. The behaviour they protected either moves into constraint tests or is already covered by the fixture's reads:
- versions that can't be changed (`instruction_versions_survive_reopen_and_reject_mutation`);
- messages from the operator but no unknown agent;
- lanes kept as versions.

A new test checks that a database older than 0.10.0 is refused and left unchanged. Another checks that the first message after the upgrade gets an ID above every message before it, including when the newest messages were in a table whose counter had run past its largest row.

**How long adoption takes.** Adoption copies every table and re-tokenizes every message once. F measures it on a generated database with a million messages and reports the time in its description. While it runs the server has not started; the pre-upgrade backup already makes a large database start slower once.

**Existing installs.**
- 0.10.0 and `next` databases upgrade in place, after the backup.
- Older databases are refused with directions. That is a visible change, which Sam approved in decision 3.
- Going back is not supported, as today: an older kkt run on an upgraded file is not safe, and the pre-upgrade backup is the way back.
- Table and column names stay as they are, so `kkt backup`, `copy_database`, and tests that open the file with rusqlite to inspect it keep working.

## 6. Pull requests

Each one builds on a merged contract (PRINCIPLES.md rule 9). Sizes are lines changed, tests included, and are estimates.

| PR | Branch base | What | Size |
|---|---|---|---|
| **A** | `multitenant` | This document; the step 4 and step 6 rows of `plan-phase-1.md` | ~400 |
| **B** | `next` | `kkt-db`, part 1: values, column types, declaring tables, keys, references, indexes and constraints; `Select` and the changes; `Database` and `Connection`; errors; migrations and their record. The conformance suite for all of it, runnable by any backend (`testing`). No backend yet. CLAUDE.md row, `check-crates.sh`. | ~1,600 |
| **C** | `next` | `kkt-db`, part 2: search. The tokenizer, terms, excerpts and the `Search` operation, with their conformance tests. | ~700 |
| **D** | `next` | `kkt-db-sqlite`: SQL generation, connections, transactions (reads included), generated triggers, FTS5 over kkt-db's tokens, the version record, `pending_upgrade`, `copy_database`. Passes the conformance suite. The differential tests against FTS5's tokenizer and `snippet`. Adopting is not part of D. | ~2,000 |
| **E** | `next` | kkt-store declares its schema (baseline versions 1 and 2) and implements `Reads` and `Writes` with kkt-db's operations. `sqlite.rs` goes; kkt-store loses rusqlite. The program and kkt-server open the SQLite backend. Fresh installs only: opening a 0.10.0 file is refused until F, so E merges to `next` but no release is cut between E and F. | ~2,500, mostly a rewrite of `sqlite.rs` method by method |
| **F** | `next` | Adopting a 0.10.0 or `next` database in kkt-db-sqlite. Refusing older ones. The fixture's SQL files moved under `tests/`. The fresh-equals-upgraded test. The pre-0.10.0 migration tests removed, with their protections moved. The pre-upgrade backup's report. | ~1,200 |
| **G** | `next` | CLAUDE.md: the Storage rule ("only kkt-db-sqlite knows SQLite; kkt-store declares, never writes SQL"), the three crate rows, "migrations only add" restated for declared migrations, `testing` features. The `the_stores_channel_filters_agree_with_decide` and `swarms_stay_apart` notes. | ~150 |

E and F can be one pull request if Sam prefers no window on `next` where an old database is refused. It would be about 3,700 lines. With two, any install built from `next` cannot be redeployed between E and F: its database, at `user_version` 21 or 22, would be refused until F lands.

Verified by, for the step as a whole:
- every store, rule and end-to-end test passes unchanged;
- `v0_10_0_reads.txt` is unchanged;
- fresh equals upgraded;
- the conformance suite passes on kkt-db-sqlite;
- the differential search tests pass, or their listed differences are approved;
- `tests/backup.rs` and the pre-upgrade backup are unchanged;
- `check-crates.sh` passes with the two new crates and their features.

## 7. Questions for Sam

1. **Search tokens in kkt-db.** Section 3 makes kkt-db tokenize, so search is the same on every backend and the excerpt is ours. The alternative is that each backend tokenizes natively (FTS5 for SQLite, `tsvector` for Postgres) to a written specification. That is less code, but two backends would match and excerpt differently in edge cases. I recommend kkt-db tokenizing. Do you agree?
2. **Refusing databases older than 0.10.0** with the message in section 5, rather than upgrading them through the old SQL files kept for that purpose. Is that what "need not be supported directly" means? `next` databases (`user_version` 22) are adopted too.
3. **Dropping `handoff_note` and `handoff_note_set_at`** from the baseline. Migration 0019 copied them into memory entries and nothing has read them since. Keeping them costs two dead columns forever. Drop them?
4. **E and F as two pull requests** with a short window on `next` where an old database is refused, or as one larger pull request?
