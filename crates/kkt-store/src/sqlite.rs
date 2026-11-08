//! The store over SQLite. This is the only file that knows about SQL.
//!
//! One connection writes; several others read. In WAL mode the readers do not
//! block the writer or each other. All calls run on Tokio's blocking pool.

use super::*;
#[cfg(any(test, feature = "real", feature = "testing"))]
use kkt_config::DB_BUSY_TIMEOUT;
#[cfg(any(test, feature = "real"))]
use kkt_config::DB_READERS;
#[cfg(any(test, feature = "real"))]
use rusqlite::OpenFlags;
use rusqlite::{Connection, OptionalExtension, named_params};
#[cfg(any(test, feature = "real"))]
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(any(test, feature = "real", feature = "testing"))]
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_server_runs.sql"),
    include_str!("../migrations/0003_revoked_agents.sql"),
    include_str!("../migrations/0004_identity_recovery.sql"),
    include_str!("../migrations/0005_messages_by_time.sql"),
    include_str!("../migrations/0006_agent_descriptions.sql"),
    include_str!("../migrations/0007_channel_notes.sql"),
    include_str!("../migrations/0008_operator_messages.sql"),
    include_str!("../migrations/0009_handoff_notes.sql"),
    include_str!("../migrations/0010_hidden_entries.sql"),
    include_str!("../migrations/0011_operator_instructions.sql"),
    include_str!("../migrations/0012_operator_channels.sql"),
    include_str!("../migrations/0013_channel_owners.sql"),
    include_str!("../migrations/0014_agent_clients.sql"),
    include_str!("../migrations/0015_instruction_authors.sql"),
    include_str!("../migrations/0016_lanes.sql"),
    include_str!("../migrations/0017_agent_lanes.sql"),
    include_str!("../migrations/0018_lane_sources.sql"),
    include_str!("../migrations/0019_memory.sql"),
    include_str!("../migrations/0020_channel_runs.sql"),
    include_str!("../migrations/0021_messages_by_agent.sql"),
    include_str!("../migrations/0022_swarms.sql"),
];

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> StoreError {
        StoreError::Database(e.to_string())
    }
}

#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    writer: Mutex<Connection>,
    /// Empty for an in-memory database, which can only have one connection.
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

impl Db {
    /// Opens (creating if needed) the database file and brings its tables up to date.
    #[cfg(any(test, feature = "real"))]
    pub fn open(path: &Path) -> StoreResult<Db> {
        let mut writer = Connection::open(path)?;
        configure(&writer)?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&mut writer)?;
        let readers = (0..DB_READERS)
            .map(|_| {
                let conn = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                configure(&conn)?;
                Ok(Mutex::new(conn))
            })
            .collect::<StoreResult<Vec<_>>>()?;
        Ok(Db::from_parts(writer, readers))
    }

    /// A fresh database held in memory, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn open_in_memory() -> StoreResult<Db> {
        let mut conn = Connection::open_in_memory()?;
        configure(&conn)?;
        migrate(&mut conn)?;
        Ok(Db::from_parts(conn, Vec::new()))
    }

    #[cfg(any(test, feature = "real", feature = "testing"))]
    fn from_parts(writer: Connection, readers: Vec<Mutex<Connection>>) -> Db {
        Db {
            inner: Arc::new(Inner {
                writer: Mutex::new(writer),
                readers,
                next_reader: AtomicUsize::new(0),
            }),
        }
    }

    /// Runs `f` on a read connection.
    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&dyn Reads) -> Result<T, E> + Send + 'static,
    {
        let inner = self.inner.clone();
        run_blocking(move || {
            let lock = if inner.readers.is_empty() {
                &inner.writer
            } else {
                let i = inner.next_reader.fetch_add(1, Ordering::Relaxed) % inner.readers.len();
                &inner.readers[i]
            };
            let conn = lock.lock().map_err(|_| StoreError::Poisoned)?;
            f(&Sql(&conn))
        })
        .await
    }

    /// Runs `f` inside a write transaction. The transaction commits only if `f` succeeds.
    pub async fn write<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&dyn Writes) -> Result<T, E> + Send + 'static,
    {
        let inner = self.inner.clone();
        run_blocking(move || {
            let mut conn = inner.writer.lock().map_err(|_| StoreError::Poisoned)?;
            let tx = conn.transaction().map_err(StoreError::from)?;
            let value = f(&Sql(&tx))?;
            tx.commit().map_err(StoreError::from)?;
            Ok(value)
        })
        .await
    }

    /// Runs raw SQL in its own transaction. Tests use it to break the database
    /// on purpose, for example with a trigger that makes inserts fail.
    #[cfg(any(test, feature = "testing"))]
    pub async fn execute_for_tests(&self, sql: &'static str) -> StoreResult<()> {
        let inner = self.inner.clone();
        run_blocking(move || {
            let conn = inner.writer.lock().map_err(|_| StoreError::Poisoned)?;
            conn.execute_batch(sql)?;
            Ok(())
        })
        .await
    }
}

async fn run_blocking<T, E, F>(f: F) -> Result<T, E>
where
    T: Send + 'static,
    E: From<StoreError> + Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_error) => Err(StoreError::Task(join_error.to_string()).into()),
    }
}

#[cfg(any(test, feature = "real", feature = "testing"))]
fn configure(conn: &Connection) -> StoreResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(DB_BUSY_TIMEOUT)?;
    Ok(())
}

/// The upgrade [`Db::open`] would make to the tables in the database file at
/// `path`: from one schema version to another. `None` when there is no file,
/// it has no kkt tables yet, or they are up to date. Changes nothing.
#[cfg(any(test, feature = "real"))]
pub fn pending_upgrade(path: &Path) -> StoreResult<Option<SchemaUpgrade>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&conn)?;
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let from = usize::try_from(current).unwrap_or(0);
    let to = MIGRATIONS.len();
    Ok((from > 0 && from < to).then_some(SchemaUpgrade { from, to }))
}

/// Writes a copy of the database file at `from` to the new file `to` with
/// SQLite's online backup, so the copy is consistent even while a server
/// writes to `from`. The copy is one file (no write-ahead log beside it),
/// and it passes SQLite's integrity check before this returns.
#[cfg(any(test, feature = "real"))]
pub fn copy_database(from: &Path, to: &Path) -> StoreResult<()> {
    if to.exists() {
        return Err(StoreError::Database(format!(
            "{} already exists",
            to.display()
        )));
    }
    let source = Connection::open_with_flags(
        from,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&source)?;
    let mut copy = Connection::open(to)?;
    configure(&copy)?;
    {
        use rusqlite::backup::{Backup, StepResult};
        let backup = Backup::new(&source, &mut copy)?;
        // All pages in one step (-1): one read transaction, so the copy is
        // the database at one moment. In WAL mode it does not block writers.
        // Both connections wait DB_BUSY_TIMEOUT for locks, so busy is final.
        loop {
            match backup.step(-1)? {
                StepResult::Done => break,
                StepResult::More => continue,
                // Busy or locked.
                waiting => {
                    return Err(StoreError::Database(format!(
                        "the database stayed {waiting:?}; try again"
                    )));
                }
            }
        }
    }
    copy.pragma_update(None, "journal_mode", "DELETE")?;
    let check: String = copy.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(StoreError::Database(format!(
            "the copy failed SQLite's integrity check: {check}"
        )));
    }
    Ok(())
}

/// Applies any migrations newer than the database's `user_version`.
#[cfg(any(test, feature = "real", feature = "testing"))]
fn migrate(conn: &mut Connection) -> StoreResult<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        // SQLite's table-rebuild procedure requires foreign keys off before
        // the transaction, then an integrity check before committing.
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        let result = (|| -> StoreResult<()> {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            let invalid = tx.prepare("PRAGMA foreign_key_check")?.exists([])?;
            if invalid {
                return Err(StoreError::Database(
                    "migration violates foreign keys".into(),
                ));
            }
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
            Ok(())
        })();
        conn.pragma_update(None, "foreign_keys", "ON")?;
        result?;
    }
    Ok(())
}

/// The store's questions and changes, over one connection. Inside
/// [`Db::write`] the connection is a transaction.
struct Sql<'a>(&'a Connection);

impl Sql<'_> {
    /// Whether the avatar `name`, hidden or not, is in `swarm`.
    fn agent_in(&self, swarm: &SwarmId, name: &str) -> StoreResult<bool> {
        Ok(self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE name = ? AND swarm = ?)",
            [name, swarm.as_str()],
            |r| r.get(0),
        )?)
    }

    /// Whether the channel `id`, hidden or not, is in `swarm`.
    fn channel_in(&self, swarm: &SwarmId, id: &str) -> StoreResult<bool> {
        Ok(self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id = ? AND swarm = ?)",
            [id, swarm.as_str()],
            |r| r.get(0),
        )?)
    }

    /// Fails unless the avatar `name` is in `swarm`. The operator, which
    /// is no avatar, passes when `operator` allows it.
    fn require_agent(&self, swarm: &SwarmId, name: &str, operator: bool) -> StoreResult<()> {
        if (operator && name == OPERATOR) || self.agent_in(swarm, name)? {
            return Ok(());
        }
        Err(outside(swarm, &format!("avatar {name}")))
    }

    /// Fails unless the channel `id` is in `swarm`.
    fn require_channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<()> {
        if self.channel_in(swarm, id)? {
            return Ok(());
        }
        Err(outside(swarm, &format!("channel {id}")))
    }

    /// One channel as `viewer` sees it, if it exists and is hidden or not as
    /// `hidden` says.
    fn summary(
        &self,
        swarm: &SwarmId,
        id: &str,
        viewer: Option<&str>,
        after_id: Option<i64>,
        hidden: bool,
    ) -> StoreResult<Option<ChannelSummary>> {
        Ok(self
            .0
            .query_row(
                "SELECT c.id, c.name, c.purpose, c.is_open, c.created_by, c.created_at,
                        (SELECT count(*) FROM memberships m WHERE m.channel_id = c.id),
                        EXISTS (SELECT 1 FROM memberships m WHERE m.channel_id = c.id AND m.agent = :viewer),
                        CASE WHEN :after IS NULL THEN NULL
                             ELSE (SELECT count(*) FROM messages g WHERE g.channel_id = c.id AND g.id > :after)
                        END, c.note, c.note_edited_by, c.note_edited_at, c.owner
                 FROM channels c WHERE c.id = :id AND c.hidden = :hidden AND c.swarm = :swarm",
                named_params! {
                    ":id": id, ":viewer": viewer, ":after": after_id, ":hidden": hidden,
                    ":swarm": swarm.as_str(),
                },
                |r| {
                    Ok(ChannelSummary {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        purpose: r.get(2)?,
                        open: r.get(3)?,
                        created_by: r.get(4)?,
                        created_at: r.get(5)?,
                        owner: r.get(12)?,
                        member_count: r.get(6)?,
                        viewer_is_member: r.get(7)?,
                        new_messages: r.get(8)?,
                        note: ChannelNoteRecord { text: r.get(9)?, edited_by: r.get(10)?, edited_at: r.get(11)? },
                    })
                },
            )
            .optional()?)
    }
}

const AGENT_COLUMNS: &str = "name, created_at, last_seen, run_id, revoked_at, description, client_name, client_version, client_recorded_at, client_run_id,
    lane, lane_version, lane_taken_at, lane_run_id,
    (SELECT max(lv.version) FROM lane_versions lv WHERE lv.lane = agents.lane)";

fn memory_entry_from_row(r: &rusqlite::Row) -> rusqlite::Result<MemoryEntryRecord> {
    Ok(MemoryEntryRecord {
        name: r.get(0)?,
        text: r.get(1)?,
        saved_at: r.get(2)?,
        run_id: r.get(3)?,
    })
}

fn agent_from_row(r: &rusqlite::Row) -> rusqlite::Result<AgentRecord> {
    Ok(AgentRecord {
        name: r.get(0)?,
        created_at: r.get(1)?,
        last_seen: r.get(2)?,
        run_id: r.get(3)?,
        revoked_at: r.get(4)?,
        description: r.get(5)?,
        client: match r.get::<_, Option<String>>(6)? {
            Some(name) => Some(AgentClient {
                name,
                version: r.get(7)?,
                recorded_at: r.get(8)?,
                run_id: r.get(9)?,
            }),
            None => None,
        },
        lane: match r.get::<_, Option<String>>(10)? {
            Some(lane) => Some(AgentLaneRecord {
                taken: TakenLaneRecord {
                    lane,
                    version: r.get(11)?,
                    taken_at: r.get(12)?,
                    run_id: r.get(13)?,
                },
                current_version: r.get(14)?,
            }),
            None => None,
        },
    })
}

/// The author recorded for the operator's messages and channels: no avatar.
const OPERATOR: &str = "operator";

/// The error for a change tied to something not in the swarm.
fn outside(swarm: &SwarmId, what: &str) -> StoreError {
    StoreError::Database(format!("no {what} in swarm {}", swarm.as_str()))
}

/// The handles of one swarm's avatars, as a subquery taking the swarm as
/// its one positional parameter.
const SWARM_AGENTS: &str = "SELECT name FROM agents WHERE swarm = ?";

/// The IDs of one swarm's channels, as a subquery taking the swarm as its
/// one positional parameter.
const SWARM_CHANNELS: &str = "SELECT id FROM channels WHERE swarm = ?";

const MESSAGE_COLUMNS: &str = "m.id, m.channel_id, c.name, m.agent, m.body, m.created_at, m.run_id";

fn message_from_row(r: &rusqlite::Row) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        id: r.get(0)?,
        channel_id: r.get(1)?,
        channel_name: r.get(2)?,
        agent: r.get(3)?,
        body: r.get(4)?,
        created_at: r.get(5)?,
        run_id: r.get(6)?,
    })
}

/// The IDs of the channels a [`ChannelScope`] covers, as a subquery. Uses
/// `:swarm`, the scope's swarm, which no channel of another swarm passes,
/// `:viewer`, the scope's viewer (with none, no channel counts as
/// joined or owned), and `:allowed`, from [`scope_mask`]. Each
/// channel's kind for the viewer is a number from 0 to 15, as [`kind_bit`]
/// gives it, and `:allowed` has that bit set for each allowed kind. A
/// subquery not tied to the outer row, so SQLite runs it once per query.
const SCOPE_CHANNELS: &str = "SELECT s.id FROM channels s
    WHERE s.swarm = :swarm AND (:allowed >> ((s.hidden != 0) * 8 + (s.is_open != 0) * 4
        + EXISTS (SELECT 1 FROM memberships sm WHERE sm.channel_id = s.id AND sm.agent = :viewer) * 2
        + coalesce(s.owner = :viewer, 0))) & 1 = 1";

/// A channel kind's number in [`SCOPE_CHANNELS`].
fn kind_bit(kind: &ChannelKind) -> u32 {
    (kind.hidden as u32) << 3
        | (kind.open as u32) << 2
        | (kind.member as u32) << 1
        | kind.owner as u32
}

/// The `:allowed` of [`SCOPE_CHANNELS`].
fn scope_mask(scope: &ChannelScope) -> i64 {
    scope
        .allowed()
        .iter()
        .fold(0, |mask, kind| mask | 1 << kind_bit(kind))
}

/// Which messages a `NewerFilter` covers. Uses `:after`, `:filter` and those
/// of [`SCOPE_CHANNELS`].
fn newer_scope() -> String {
    format!(
        "m.id > :after
    AND m.channel_id IN ({SCOPE_CHANNELS})
    AND (:filter IS NULL OR m.channel_id IN (SELECT value FROM json_each(:filter)))"
    )
}

/// The channel ID list of a `NewerFilter`, as the JSON array `json_each` reads.
fn channel_ids_json(filter: &NewerFilter) -> StoreResult<Option<String>> {
    filter
        .channel_ids
        .as_ref()
        .map(|ids| serde_json::to_string(ids).map_err(|e| StoreError::Database(e.to_string())))
        .transpose()
}

/// The server announcement keeps the table name it had before the rename.
fn server_announcement_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerAnnouncementRecord> {
    Ok(ServerAnnouncementRecord {
        version: row.get(0)?,
        text: row.get(1)?,
        saved_at: row.get(2)?,
        run_id: row.get(3)?,
        saved_by_admin: row.get(4)?,
    })
}

/// A lane with its newest version. Uses the alias `l` for `lanes` and `v`
/// for that version.
const LANE_SELECT: &str =
    "SELECT l.name, l.created_at, l.run_id, l.retired_at, l.retired_run_id, l.retired_by_admin,
            v.version, v.description, v.text, v.saved_at, v.run_id, v.saved_by_admin,
            v.source_folder, v.source_commit
     FROM lanes l JOIN lane_versions v ON v.lane = l.name
      AND v.version = (SELECT max(version) FROM lane_versions WHERE lane = l.name)";

fn lane_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LaneRecord> {
    Ok(LaneRecord {
        name: r.get(0)?,
        created_at: r.get(1)?,
        run_id: r.get(2)?,
        retired: match r.get::<_, Option<String>>(3)? {
            Some(at) => Some(LaneRetiredRecord {
                at,
                run_id: r.get(4)?,
                by_admin: r.get(5)?,
            }),
            None => None,
        },
        current: LaneVersionRecord {
            version: r.get(6)?,
            description: r.get(7)?,
            text: r.get(8)?,
            saved_at: r.get(9)?,
            run_id: r.get(10)?,
            saved_by_admin: r.get(11)?,
            source: lane_source(r.get(12)?, r.get(13)?),
        },
    })
}

fn lane_version_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LaneVersionRecord> {
    Ok(LaneVersionRecord {
        version: r.get(0)?,
        description: r.get(1)?,
        text: r.get(2)?,
        saved_at: r.get(3)?,
        run_id: r.get(4)?,
        saved_by_admin: r.get(5)?,
        source: lane_source(r.get(6)?, r.get(7)?),
    })
}

/// An imported version's source, from its two columns; `None` without a
/// folder, for a version saved on the web page or at `/mcp-admin`.
fn lane_source(folder: Option<String>, commit: Option<String>) -> Option<LaneSourceRecord> {
    folder.map(|folder| LaneSourceRecord { folder, commit })
}

impl Reads for Sql<'_> {
    fn lanes(&self) -> StoreResult<Vec<LaneRecord>> {
        let mut statement = self.0.prepare(&format!("{LANE_SELECT} ORDER BY l.name"))?;
        Ok(statement
            .query_map([], lane_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn lanes_after(&self, after: Option<&str>, limit: usize) -> StoreResult<Vec<LaneRecord>> {
        let mut statement = self.0.prepare(&format!(
            "{LANE_SELECT} WHERE (?1 IS NULL OR l.name > ?1) ORDER BY l.name LIMIT ?2"
        ))?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        Ok(statement
            .query_map(rusqlite::params![after, limit], lane_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn lane(&self, name: &str) -> StoreResult<Option<LaneRecord>> {
        Ok(self
            .0
            .query_row(&format!("{LANE_SELECT} WHERE l.name = ?"), [name], lane_row)
            .optional()?)
    }

    fn lane_history(&self, name: &str) -> StoreResult<Vec<LaneVersionRecord>> {
        let mut statement = self.0.prepare(
            "SELECT version, description, text, saved_at, run_id, saved_by_admin,
                    source_folder, source_commit
             FROM lane_versions WHERE lane = ? ORDER BY version DESC",
        )?;
        Ok(statement
            .query_map([name], lane_version_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn lane_version(&self, name: &str, version: i64) -> StoreResult<Option<LaneVersionRecord>> {
        Ok(self
            .0
            .query_row(
                "SELECT version, description, text, saved_at, run_id, saved_by_admin,
                        source_folder, source_commit
                 FROM lane_versions WHERE lane = ? AND version = ?",
                rusqlite::params![name, version],
                lane_version_row,
            )
            .optional()?)
    }

    fn takeable_lanes_after(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<LaneRecord>> {
        let mut statement = self.0.prepare(&format!(
            "{LANE_SELECT} WHERE l.retired_at IS NULL AND (?1 IS NULL OR l.name > ?1)
             ORDER BY l.name LIMIT ?2"
        ))?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        Ok(statement
            .query_map(rusqlite::params![after, limit], lane_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn current_server_announcement(&self) -> StoreResult<Option<ServerAnnouncementRecord>> {
        Ok(self.0.query_row("SELECT version, text, saved_at, run_id, saved_by_admin FROM operator_instructions ORDER BY version DESC LIMIT 1", [], server_announcement_row).optional()?)
    }
    fn server_announcement_history(&self) -> StoreResult<Vec<ServerAnnouncementRecord>> {
        let mut statement = self.0.prepare("SELECT version, text, saved_at, run_id, saved_by_admin FROM operator_instructions ORDER BY version DESC")?;
        Ok(statement
            .query_map([], server_announcement_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn agent_name_taken(&self, name: &str) -> StoreResult<bool> {
        Ok(self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE name = ? COLLATE NOCASE)",
            [name],
            |r| r.get(0),
        )?)
    }

    fn agent(&self, swarm: &SwarmId, name: &str) -> StoreResult<Option<AgentRecord>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {AGENT_COLUMNS} FROM agents WHERE name = ? AND hidden = 0 AND swarm = ?"
                ),
                [name, swarm.as_str()],
                agent_from_row,
            )
            .optional()?)
    }

    fn agent_by_token_hash(
        &self,
        swarm: &SwarmId,
        hash: &[u8],
    ) -> StoreResult<Option<AgentRecord>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {AGENT_COLUMNS} FROM agents WHERE token_hash = ? AND hidden = 0 AND swarm = ?"
                ),
                rusqlite::params![hash, swarm.as_str()],
                agent_from_row,
            )
            .optional()?)
    }

    fn agent_by_recovery_hash(
        &self,
        swarm: &SwarmId,
        name: &str,
        hash: &[u8],
    ) -> StoreResult<Option<AgentRecord>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {AGENT_COLUMNS} FROM agents WHERE name = ? AND recovery_hash = ? AND hidden = 0 AND swarm = ?"),
                rusqlite::params![name, hash, swarm.as_str()],
                agent_from_row,
            )
            .optional()?)
    }

    fn memory_entry(
        &self,
        swarm: &SwarmId,
        agent: &str,
        name: &str,
    ) -> StoreResult<Option<MemoryEntryRecord>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT name, text, saved_at, run_id FROM memory_entries
                     WHERE agent = ? AND name = ? AND agent IN ({SWARM_AGENTS})"
                ),
                [agent, name, swarm.as_str()],
                memory_entry_from_row,
            )
            .optional()?)
    }

    fn memory_entries(&self, swarm: &SwarmId, agent: &str) -> StoreResult<Vec<MemoryEntryRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT name, text, saved_at, run_id FROM memory_entries
             WHERE agent = ? AND agent IN ({SWARM_AGENTS}) ORDER BY name"
        ))?;
        let rows = stmt.query_map([agent, swarm.as_str()], memory_entry_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn agents(
        &self,
        swarm: &SwarmId,
        text: Option<&str>,
        lane: Option<&str>,
        after_name: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AgentRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents
             WHERE hidden = 0 AND swarm = :swarm
               AND (:pattern IS NULL OR name LIKE :pattern ESCAPE '\\'
                 OR description LIKE :pattern ESCAPE '\\')
               AND (:lane IS NULL OR lane = :lane)
               AND (:cursor IS NULL OR name > :cursor)
             ORDER BY name LIMIT :limit"
        ))?;
        let agents = stmt
            .query_map(
                named_params! {
                    ":swarm": swarm.as_str(),
                    ":pattern": text.map(like_pattern),
                    ":lane": lane,
                    ":cursor": after_name,
                    ":limit": limit as i64,
                },
                agent_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(agents)
    }

    fn active_agent_count(&self, swarm: &SwarmId) -> StoreResult<i64> {
        Ok(self.0.query_row(
            "SELECT count(*) FROM agents WHERE revoked_at IS NULL AND hidden = 0 AND swarm = ?",
            [swarm.as_str()],
            |r| r.get(0),
        )?)
    }

    fn agents_by_last_seen(&self, swarm: &SwarmId) -> StoreResult<Vec<AgentRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents WHERE hidden = 0 AND swarm = ?
             ORDER BY coalesce(last_seen, created_at) DESC"
        ))?;
        let agents = stmt
            .query_map([swarm.as_str()], agent_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(agents)
    }

    fn channel_id_taken(&self, id: &str) -> StoreResult<bool> {
        Ok(self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id = ?)",
            [id],
            |r| r.get(0),
        )?)
    }

    fn channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<Option<ChannelFacts>> {
        Ok(self
            .0
            .query_row(
                "SELECT id, name, is_open, owner FROM channels WHERE id = ? AND hidden = 0 AND swarm = ?",
                [id, swarm.as_str()],
                |r| {
                    Ok(ChannelFacts {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        open: r.get(2)?,
                        owner: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    fn is_member(&self, swarm: &SwarmId, channel_id: &str, agent: &str) -> StoreResult<bool> {
        let found: Option<i64> = self
            .0
            .query_row(
                &format!(
                    "SELECT 1 FROM memberships WHERE channel_id = ? AND agent = ?
                     AND channel_id IN ({SWARM_CHANNELS})"
                ),
                [channel_id, agent, swarm.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    fn channel_ids(&self, filter: &ChannelFilter) -> StoreResult<Vec<String>> {
        let (cursor_name, cursor_id) = match &filter.after {
            Some((name, id)) => (Some(name.as_str()), Some(id.as_str())),
            None => (None, None),
        };
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT c.id FROM channels c
             WHERE c.id IN ({SCOPE_CHANNELS})
               AND (:with IS NULL OR (
                     EXISTS (SELECT 1 FROM memberships m WHERE m.channel_id = c.id AND m.agent = :viewer)
                 AND EXISTS (SELECT 1 FROM memberships w WHERE w.channel_id = c.id AND w.agent = :with)))
               AND (:pattern IS NULL
                    OR c.name LIKE :pattern ESCAPE '\\'
                    OR coalesce(c.purpose, '') LIKE :pattern ESCAPE '\\')
               AND (:cname IS NULL OR (c.name, c.id) > (:cname, :cid))
             ORDER BY c.name, c.id
             LIMIT :limit"
        ))?;
        let ids = stmt
            .query_map(
                named_params! {
                    ":swarm": filter.scope.swarm().as_str(),
                    ":viewer": filter.scope.viewer(),
                    ":allowed": scope_mask(&filter.scope),
                    ":with": filter.shared_with,
                    ":pattern": filter.text.as_deref().map(like_pattern),
                    ":cname": cursor_name,
                    ":cid": cursor_id,
                    ":limit": filter.limit as i64,
                },
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    fn all_channel_ids(&self, swarm: &SwarmId, member: Option<&str>) -> StoreResult<Vec<String>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT c.id FROM channels c
             WHERE c.hidden = 0 AND c.swarm = :swarm AND (:member IS NULL
                OR EXISTS (SELECT 1 FROM memberships m WHERE m.channel_id = c.id AND m.agent = :member))
             ORDER BY c.name, c.id",
        )?;
        let ids = stmt
            .query_map(
                named_params! { ":swarm": swarm.as_str(), ":member": member },
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    fn owned_channel_ids(&self, swarm: &SwarmId, owner: &str) -> StoreResult<Vec<String>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT id FROM channels WHERE hidden = 0 AND owner = ? AND swarm = ? ORDER BY name, id",
        )?;
        let ids = stmt
            .query_map([owner, swarm.as_str()], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    fn hidden_channel_ids(&self, swarm: &SwarmId) -> StoreResult<Vec<String>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT id FROM channels WHERE hidden = 1 AND swarm = ? ORDER BY name, id",
        )?;
        let ids = stmt
            .query_map([swarm.as_str()], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    fn channel_summary(
        &self,
        swarm: &SwarmId,
        id: &str,
        viewer: Option<&str>,
        after_id: Option<i64>,
    ) -> StoreResult<Option<ChannelSummary>> {
        self.summary(swarm, id, viewer, after_id, false)
    }

    fn hidden_channel_summary(
        &self,
        swarm: &SwarmId,
        id: &str,
    ) -> StoreResult<Option<ChannelSummary>> {
        self.summary(swarm, id, None, None, true)
    }

    fn members_by_join(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<String>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT agent FROM memberships WHERE channel_id = ? AND channel_id IN ({SWARM_CHANNELS})
             ORDER BY joined_at, agent LIMIT ?"
        ))?;
        let names = stmt
            .query_map(
                rusqlite::params![channel_id, swarm.as_str(), limit as i64],
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(names)
    }

    fn member_count(&self, swarm: &SwarmId, channel_id: &str) -> StoreResult<i64> {
        Ok(self.0.query_row(
            &format!(
                "SELECT count(*) FROM memberships
                 WHERE channel_id = ? AND channel_id IN ({SWARM_CHANNELS})"
            ),
            [channel_id, swarm.as_str()],
            |r| r.get(0),
        )?)
    }

    fn members_by_name(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        after_name: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AgentRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents
             WHERE name IN (SELECT agent FROM memberships WHERE channel_id = :id)
               AND :id IN (SELECT id FROM channels WHERE swarm = :swarm)
               AND swarm = :swarm
               AND (:cursor IS NULL OR name > :cursor)
             ORDER BY name LIMIT :limit"
        ))?;
        let members = stmt
            .query_map(
                named_params! {
                    ":id": channel_id, ":swarm": swarm.as_str(), ":cursor": after_name,
                    ":limit": limit as i64,
                },
                agent_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(members)
    }

    fn message(&self, swarm: &SwarmId, id: i64) -> StoreResult<Option<MessageRecord>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN channels c ON c.id = m.channel_id
                     WHERE m.id = ? AND c.swarm = ?"
                ),
                rusqlite::params![id, swarm.as_str()],
                message_from_row,
            )
            .optional()?)
    }

    fn channel_messages(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        from_agent: Option<&str>,
        range: MessageRange,
        limit: usize,
    ) -> StoreResult<Vec<MessageRecord>> {
        let (sql, bound) = match range {
            MessageRange::After(id) => ("m.id > :bound ORDER BY m.id ASC", id),
            MessageRange::Before(id) => ("m.id < :bound ORDER BY m.id DESC", id),
            MessageRange::Latest => ("m.id <= :bound ORDER BY m.id DESC", i64::MAX),
        };
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE m.channel_id = :ch AND c.swarm = :swarm
               AND (:from IS NULL OR m.agent = :from) AND {sql}
             LIMIT :limit"
        ))?;
        let messages = stmt
            .query_map(
                named_params! {
                    ":ch": channel_id, ":swarm": swarm.as_str(), ":from": from_agent,
                    ":bound": bound, ":limit": limit as i64,
                },
                message_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    fn newer_messages(
        &self,
        filter: &NewerFilter,
        limit: usize,
    ) -> StoreResult<Vec<MessageRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE {} ORDER BY m.id LIMIT :limit",
            newer_scope()
        ))?;
        let messages = stmt
            .query_map(
                named_params! {
                    ":after": filter.after_id,
                    ":swarm": filter.scope.swarm().as_str(),
                    ":viewer": filter.scope.viewer(),
                    ":allowed": scope_mask(&filter.scope),
                    ":filter": channel_ids_json(filter)?,
                    ":limit": limit as i64,
                },
                message_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    fn count_newer_messages(&self, filter: &NewerFilter) -> StoreResult<i64> {
        Ok(self.0.query_row(
            &format!("SELECT count(*) FROM messages m WHERE {}", newer_scope()),
            named_params! {
                ":after": filter.after_id,
                ":swarm": filter.scope.swarm().as_str(),
                ":viewer": filter.scope.viewer(),
                ":allowed": scope_mask(&filter.scope),
                ":filter": channel_ids_json(filter)?,
            },
            |r| r.get(0),
        )?)
    }

    fn latest_message_from(&self, filter: &SentFilter) -> StoreResult<Option<MessageRecord>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN channels c ON c.id = m.channel_id
                     WHERE m.agent = :agent
                       AND (:ch IS NULL OR m.channel_id = :ch)
                       AND m.channel_id IN ({SCOPE_CHANNELS})
                     ORDER BY m.id DESC LIMIT 1"
                ),
                named_params! {
                    ":agent": filter.agent,
                    ":ch": filter.channel_id,
                    ":swarm": filter.scope.swarm().as_str(),
                    ":viewer": filter.scope.viewer(),
                    ":allowed": scope_mask(&filter.scope),
                },
                message_from_row,
            )
            .optional()?)
    }

    fn recent_messages(&self, swarm: &SwarmId, limit: usize) -> StoreResult<Vec<MessageRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE c.hidden = 0 AND c.swarm = ? ORDER BY m.id DESC LIMIT ?"
        ))?;
        let messages = stmt
            .query_map(
                rusqlite::params![swarm.as_str(), limit as i64],
                message_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    fn message_count_from(&self, swarm: &SwarmId, agent: &str) -> StoreResult<i64> {
        Ok(self.0.query_row(
            &format!(
                "SELECT count(*) FROM messages WHERE agent = ? AND channel_id IN ({SWARM_CHANNELS})"
            ),
            [agent, swarm.as_str()],
            |r| r.get(0),
        )?)
    }

    fn search(&self, filter: &SearchFilter) -> StoreResult<Vec<SearchHitRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT m.id, m.channel_id, c.name, m.agent, m.created_at,
                    snippet(messages_fts, 0, '**', '**', '…', 12)
             FROM messages_fts
             JOIN messages m ON m.id = messages_fts.rowid
             JOIN channels c ON c.id = m.channel_id
             WHERE messages_fts MATCH :q
               AND m.channel_id IN ({SCOPE_CHANNELS})
               AND (:ch IS NULL OR m.channel_id = :ch)
               AND (:from IS NULL OR m.agent = :from)
               AND (:after IS NULL OR m.id > :after)
               AND (:before IS NULL OR m.id < :before)
               AND (:since IS NULL OR m.created_at >= :since)
               AND (:until IS NULL OR m.created_at < :until)
             ORDER BY m.id DESC
             LIMIT :limit"
        ))?;
        let hits = stmt
            .query_map(
                named_params! {
                    ":q": fts_search(&filter.words, filter.phrase.as_deref()),
                    ":swarm": filter.scope.swarm().as_str(),
                    ":viewer": filter.scope.viewer(),
                    ":allowed": scope_mask(&filter.scope),
                    ":ch": filter.channel_id,
                    ":from": filter.from_agent,
                    ":after": filter.after_id,
                    ":before": filter.before_id,
                    ":since": filter.since,
                    ":until": filter.until,
                    ":limit": filter.limit as i64,
                },
                |r| {
                    Ok(SearchHitRecord {
                        message_id: r.get(0)?,
                        channel_id: r.get(1)?,
                        channel_name: r.get(2)?,
                        agent: r.get(3)?,
                        created_at: r.get(4)?,
                        excerpt: r.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(hits)
    }

    fn activity(&self, scope: &ChannelScope, after_id: i64) -> StoreResult<Vec<ActivityRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT m.channel_id, c.name, count(*), max(m.id)
             FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE m.id > :after
               AND m.channel_id IN ({SCOPE_CHANNELS})
             GROUP BY m.channel_id
             ORDER BY max(m.id) DESC"
        ))?;
        let params = named_params! {
            ":after": after_id,
            ":swarm": scope.swarm().as_str(),
            ":viewer": scope.viewer(),
            ":allowed": scope_mask(scope),
        };
        let rows = stmt
            .query_map(params, |r| {
                Ok(ActivityRecord {
                    channel_id: r.get(0)?,
                    channel_name: r.get(1)?,
                    new_messages: r.get(2)?,
                    latest_id: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn channel_use(&self, swarm: &SwarmId) -> StoreResult<Vec<ChannelUseRecord>> {
        // The newest message is found through the (channel_id, id) index.
        let mut stmt = self.0.prepare_cached(
            "SELECT c.id,
                    (SELECT count(*) FROM messages m WHERE m.channel_id = c.id),
                    last.created_at, last.agent
             FROM channels c
             LEFT JOIN messages last ON last.id =
                 (SELECT max(m.id) FROM messages m WHERE m.channel_id = c.id)
             WHERE c.swarm = ?",
        )?;
        let rows = stmt
            .query_map([swarm.as_str()], |r| {
                Ok(ChannelUseRecord {
                    channel_id: r.get(0)?,
                    message_count: r.get(1)?,
                    last_message_at: r.get(2)?,
                    last_speaker: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn memberships(&self, swarm: &SwarmId) -> StoreResult<Vec<MembershipRecord>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT m.channel_id, c.name, m.agent
             FROM memberships m JOIN channels c ON c.id = m.channel_id
             WHERE c.swarm = ?
             ORDER BY m.joined_at, m.rowid",
        )?;
        let rows = stmt
            .query_map([swarm.as_str()], |r| {
                Ok(MembershipRecord {
                    channel_id: r.get(0)?,
                    channel_name: r.get(1)?,
                    agent: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn message_times_since(
        &self,
        swarm: &SwarmId,
        since: &str,
    ) -> StoreResult<Vec<MessageTimeRecord>> {
        // A range on the messages_by_time index: it reads only the answer,
        // and does not assume times follow message IDs.
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT channel_id, created_at FROM messages
             WHERE created_at >= ? AND channel_id IN ({SWARM_CHANNELS}) ORDER BY created_at"
        ))?;
        let times = stmt
            .query_map([since, swarm.as_str()], |r| {
                Ok(MessageTimeRecord {
                    channel_id: r.get(0)?,
                    created_at: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(times)
    }

    fn server_runs(&self) -> StoreResult<Vec<ServerRunRecord>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT id, started_at, version, git_commit, settings, instructions_sha256
             FROM server_runs ORDER BY id DESC",
        )?;
        let runs = stmt
            .query_map([], |r| {
                Ok(ServerRunRecord {
                    id: r.get(0)?,
                    started_at: r.get(1)?,
                    version: r.get(2)?,
                    git_commit: r.get(3)?,
                    settings: r.get(4)?,
                    instructions_sha256: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(runs)
    }

    fn any_channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<Option<AnyChannelRecord>> {
        Ok(self
            .0
            .query_row(
                "SELECT c.id, c.name, c.purpose, c.is_open, c.created_by, c.created_at,
                        (SELECT count(*) FROM memberships m WHERE m.channel_id = c.id),
                        c.note, c.note_edited_by, c.note_edited_at, c.owner, c.hidden,
                        c.run_id, c.note_run_id
                 FROM channels c WHERE c.id = ? AND c.swarm = ?",
                [id, swarm.as_str()],
                |r| {
                    Ok(AnyChannelRecord {
                        summary: ChannelSummary {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            purpose: r.get(2)?,
                            open: r.get(3)?,
                            created_by: r.get(4)?,
                            created_at: r.get(5)?,
                            owner: r.get(10)?,
                            member_count: r.get(6)?,
                            viewer_is_member: false,
                            new_messages: None,
                            note: ChannelNoteRecord {
                                text: r.get(7)?,
                                edited_by: r.get(8)?,
                                edited_at: r.get(9)?,
                            },
                        },
                        hidden: r.get(11)?,
                        run_id: r.get(12)?,
                        note_run_id: r.get(13)?,
                    })
                },
            )
            .optional()?)
    }

    fn any_channel_ids(
        &self,
        swarm: &SwarmId,
        text: Option<&str>,
        after: Option<&(String, String)>,
        limit: usize,
    ) -> StoreResult<Vec<String>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT c.id FROM channels c
             WHERE c.swarm = :swarm
               AND (:pattern IS NULL
                    OR c.name LIKE :pattern ESCAPE '\\'
                    OR coalesce(c.purpose, '') LIKE :pattern ESCAPE '\\')
               AND (:cname IS NULL OR (c.name, c.id) > (:cname, :cid))
             ORDER BY c.name, c.id
             LIMIT :limit",
        )?;
        let ids = stmt
            .query_map(
                named_params! {
                    ":swarm": swarm.as_str(),
                    ":pattern": text.map(like_pattern),
                    ":cname": after.map(|(name, _)| name.as_str()),
                    ":cid": after.map(|(_, id)| id.as_str()),
                    ":limit": limit as i64,
                },
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    fn any_activity(&self, swarm: &SwarmId, after_id: i64) -> StoreResult<Vec<ActivityRecord>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT m.channel_id, c.name, count(*), max(m.id)
             FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE m.id > :after AND c.swarm = :swarm
             GROUP BY m.channel_id
             ORDER BY max(m.id) DESC",
        )?;
        let rows = stmt
            .query_map(
                named_params! { ":after": after_id, ":swarm": swarm.as_str() },
                |r| {
                    Ok(ActivityRecord {
                        channel_id: r.get(0)?,
                        channel_name: r.get(1)?,
                        new_messages: r.get(2)?,
                        latest_id: r.get(3)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn any_newer_messages(
        &self,
        swarm: &SwarmId,
        after_id: i64,
        channel_ids: Option<&[String]>,
        limit: usize,
    ) -> StoreResult<Vec<MessageRecord>> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE {ANY_NEWER_SCOPE} ORDER BY m.id LIMIT :limit"
        ))?;
        let messages = stmt
            .query_map(
                named_params! {
                    ":after": after_id,
                    ":swarm": swarm.as_str(),
                    ":filter": ids_json(channel_ids)?,
                    ":limit": limit as i64,
                },
                message_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    fn count_any_newer_messages(
        &self,
        swarm: &SwarmId,
        after_id: i64,
        channel_ids: Option<&[String]>,
    ) -> StoreResult<i64> {
        Ok(self.0.query_row(
            &format!("SELECT count(*) FROM messages m WHERE {ANY_NEWER_SCOPE}"),
            named_params! {
                ":after": after_id,
                ":swarm": swarm.as_str(),
                ":filter": ids_json(channel_ids)?,
            },
            |r| r.get(0),
        )?)
    }
}

/// Which messages [`Reads::any_newer_messages`] covers. Uses `:after`,
/// `:swarm` and `:filter`.
const ANY_NEWER_SCOPE: &str = "m.id > :after
    AND m.channel_id IN (SELECT id FROM channels WHERE swarm = :swarm)
    AND (:filter IS NULL OR m.channel_id IN (SELECT value FROM json_each(:filter)))";

/// A channel ID list as the JSON array `json_each` reads.
fn ids_json(ids: Option<&[String]>) -> StoreResult<Option<String>> {
    ids.map(|ids| serde_json::to_string(ids).map_err(|e| StoreError::Database(e.to_string())))
        .transpose()
}

impl Writes for Sql<'_> {
    fn append_server_announcement(
        &self,
        text: &str,
        at: &str,
        run_id: i64,
        saved_by_admin: Option<&str>,
    ) -> StoreResult<ServerAnnouncementRecord> {
        self.0.execute(
            "INSERT INTO operator_instructions (text, saved_at, run_id, saved_by_admin) VALUES (?, ?, ?, ?)",
            rusqlite::params![text, at, run_id, saved_by_admin],
        )?;
        Ok(ServerAnnouncementRecord {
            version: self.0.last_insert_rowid(),
            text: text.into(),
            saved_at: at.into(),
            run_id,
            saved_by_admin: saved_by_admin.map(str::to_string),
        })
    }

    fn insert_lane(&self, name: &str, at: &str, run_id: i64) -> StoreResult<()> {
        self.0.execute(
            "INSERT INTO lanes (name, created_at, run_id) VALUES (?, ?, ?)",
            rusqlite::params![name, at, run_id],
        )?;
        Ok(())
    }

    fn append_lane_version(&self, v: &NewLaneVersionRecord) -> StoreResult<LaneVersionRecord> {
        let version: i64 = self.0.query_row(
            "SELECT coalesce(max(version), 0) + 1 FROM lane_versions WHERE lane = ?",
            [&v.lane],
            |r| r.get(0),
        )?;
        self.0.execute(
            "INSERT INTO lane_versions (lane, version, description, text, saved_at, run_id,
                                        saved_by_admin, source_folder, source_commit)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                v.lane,
                version,
                v.description,
                v.text,
                v.saved_at,
                v.run_id,
                v.saved_by_admin,
                v.source.as_ref().map(|s| &s.folder),
                v.source.as_ref().and_then(|s| s.commit.as_ref()),
            ],
        )?;
        Ok(LaneVersionRecord {
            version,
            description: v.description.clone(),
            text: v.text.clone(),
            saved_at: v.saved_at.clone(),
            run_id: v.run_id,
            saved_by_admin: v.saved_by_admin.clone(),
            source: v.source.clone(),
        })
    }

    fn retire_lane(&self, name: &str, retired: &LaneRetiredRecord) -> StoreResult<()> {
        self.0.execute(
            "UPDATE lanes SET retired_at = ?, retired_run_id = ?, retired_by_admin = ?
             WHERE name = ? AND retired_at IS NULL",
            rusqlite::params![retired.at, retired.run_id, retired.by_admin, name],
        )?;
        Ok(())
    }

    fn insert_swarm(&self, swarm: &SwarmId, at: &str, run_id: i64) -> StoreResult<()> {
        self.0.execute(
            "INSERT INTO swarms (id, created_at, run_id) VALUES (?, ?, ?)",
            rusqlite::params![swarm.as_str(), at, run_id],
        )?;
        Ok(())
    }

    fn insert_agent(&self, swarm: &SwarmId, agent: &NewAgentRecord) -> StoreResult<()> {
        self.0.execute(
            "INSERT INTO agents (name, token_hash, recovery_hash, created_at, last_seen, run_id, swarm)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                agent.name,
                agent.token_hash,
                agent.recovery_hash,
                agent.created_at,
                agent.last_seen,
                agent.run_id,
                swarm.as_str(),
            ],
        )?;
        Ok(())
    }

    fn set_token_hash(&self, swarm: &SwarmId, name: &str, hash: &[u8]) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET token_hash = ? WHERE name = ? AND swarm = ?",
            rusqlite::params![hash, name, swarm.as_str()],
        )?;
        Ok(())
    }

    fn set_recovery_hash(&self, swarm: &SwarmId, name: &str, hash: &[u8]) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET recovery_hash = ? WHERE name = ? AND swarm = ?",
            rusqlite::params![hash, name, swarm.as_str()],
        )?;
        Ok(())
    }

    fn save_memory_entry(
        &self,
        swarm: &SwarmId,
        agent: &str,
        entry: &MemoryEntryRecord,
    ) -> StoreResult<()> {
        self.require_agent(swarm, agent, false)?;
        self.0.execute(
            "INSERT INTO memory_entries (agent, name, text, saved_at, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (agent, name) DO UPDATE
             SET text = excluded.text, saved_at = excluded.saved_at, run_id = excluded.run_id",
            rusqlite::params![agent, entry.name, entry.text, entry.saved_at, entry.run_id],
        )?;
        Ok(())
    }

    fn delete_memory_entry(&self, swarm: &SwarmId, agent: &str, name: &str) -> StoreResult<bool> {
        let deleted = self.0.execute(
            &format!(
                "DELETE FROM memory_entries WHERE agent = ? AND name = ? AND agent IN ({SWARM_AGENTS})"
            ),
            [agent, name, swarm.as_str()],
        )?;
        Ok(deleted > 0)
    }

    fn set_last_seen(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET last_seen = ? WHERE name = ? AND swarm = ?",
            [at, name, swarm.as_str()],
        )?;
        Ok(())
    }

    fn set_agent_client(
        &self,
        swarm: &SwarmId,
        name: &str,
        client: Option<&AgentClient>,
    ) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET client_name = ?1, client_version = ?2,
             client_recorded_at = ?3, client_run_id = ?4 WHERE name = ?5 AND swarm = ?6",
            rusqlite::params![
                client.map(|c| &c.name),
                client.map(|c| &c.version),
                client.map(|c| &c.recorded_at),
                client.map(|c| c.run_id),
                name,
                swarm.as_str(),
            ],
        )?;
        Ok(())
    }

    fn set_agent_lane(
        &self,
        swarm: &SwarmId,
        name: &str,
        lane: Option<&TakenLaneRecord>,
    ) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET lane = ?1, lane_version = ?2,
             lane_taken_at = ?3, lane_run_id = ?4 WHERE name = ?5 AND swarm = ?6",
            rusqlite::params![
                lane.map(|l| &l.lane),
                lane.map(|l| l.version),
                lane.map(|l| &l.taken_at),
                lane.map(|l| l.run_id),
                name,
                swarm.as_str(),
            ],
        )?;
        Ok(())
    }

    fn set_description(
        &self,
        swarm: &SwarmId,
        name: &str,
        description: Option<&str>,
    ) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET description = ? WHERE name = ? AND swarm = ?",
            rusqlite::params![description, name, swarm.as_str()],
        )?;
        Ok(())
    }

    fn set_revoked(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()> {
        self.0.execute(
            "UPDATE agents SET revoked_at = ? WHERE name = ? AND swarm = ?",
            [at, name, swarm.as_str()],
        )?;
        Ok(())
    }

    fn insert_channel(&self, swarm: &SwarmId, channel: &NewChannelRecord) -> StoreResult<()> {
        self.require_agent(swarm, &channel.created_by, true)?;
        if let Some(owner) = &channel.owner {
            self.require_agent(swarm, owner, false)?;
        }
        self.0.execute(
            "INSERT INTO channels
                 (id, name, purpose, is_open, created_by, created_at, owner, run_id, swarm)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                channel.id,
                channel.name,
                channel.purpose,
                channel.open,
                channel.created_by,
                channel.created_at,
                channel.owner,
                channel.run_id,
                swarm.as_str(),
            ],
        )?;
        Ok(())
    }

    fn set_purpose(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        purpose: Option<&str>,
    ) -> StoreResult<()> {
        self.0.execute(
            "UPDATE channels SET purpose = ? WHERE id = ? AND swarm = ?",
            rusqlite::params![purpose, channel_id, swarm.as_str()],
        )?;
        Ok(())
    }

    fn set_channel_open(&self, swarm: &SwarmId, channel_id: &str, open: bool) -> StoreResult<()> {
        self.0.execute(
            "UPDATE channels SET is_open = ? WHERE id = ? AND swarm = ?",
            rusqlite::params![open, channel_id, swarm.as_str()],
        )?;
        Ok(())
    }

    fn hide_channel(&self, swarm: &SwarmId, channel_id: &str) -> StoreResult<()> {
        let hidden = self.0.execute(
            "UPDATE channels SET hidden = 1 WHERE id = ? AND swarm = ?",
            [channel_id, swarm.as_str()],
        )?;
        if hidden > 0 {
            self.0
                .execute("DELETE FROM memberships WHERE channel_id = ?", [channel_id])?;
        }
        Ok(())
    }

    fn hide_agent(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()> {
        let hidden = self.0.execute(
            "UPDATE agents SET hidden = 1, revoked_at = coalesce(revoked_at, ?)
             WHERE name = ? AND swarm = ?",
            [at, name, swarm.as_str()],
        )?;
        if hidden > 0 {
            self.0
                .execute("DELETE FROM memberships WHERE agent = ?", [name])?;
            self.0.execute(
                "UPDATE channels SET owner = NULL WHERE owner = ? AND swarm = ?",
                [name, swarm.as_str()],
            )?;
        }
        Ok(())
    }

    fn set_owner(&self, swarm: &SwarmId, channel_id: &str, owner: Option<&str>) -> StoreResult<()> {
        if let Some(owner) = owner {
            self.require_agent(swarm, owner, false)?;
        }
        self.0.execute(
            "UPDATE channels SET owner = ? WHERE id = ? AND swarm = ?",
            rusqlite::params![owner, channel_id, swarm.as_str()],
        )?;
        Ok(())
    }

    fn set_note(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        edit: &NoteEditRecord,
    ) -> StoreResult<()> {
        self.0.execute(
            "UPDATE channels
             SET note = ?, note_edited_by = ?, note_edited_at = ?, note_run_id = ?
             WHERE id = ? AND swarm = ?",
            rusqlite::params![
                edit.text,
                edit.editor,
                edit.at,
                edit.run_id,
                channel_id,
                swarm.as_str()
            ],
        )?;
        Ok(())
    }

    fn add_member(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        agent: &str,
        joined_at: &str,
    ) -> StoreResult<()> {
        self.require_channel(swarm, channel_id)?;
        self.require_agent(swarm, agent, false)?;
        self.0.execute(
            "INSERT OR IGNORE INTO memberships (channel_id, agent, joined_at) VALUES (?, ?, ?)",
            [channel_id, agent, joined_at],
        )?;
        Ok(())
    }

    fn remove_member(&self, swarm: &SwarmId, channel_id: &str, agent: &str) -> StoreResult<()> {
        self.0.execute(
            &format!(
                "DELETE FROM memberships WHERE channel_id = ? AND agent = ?
                 AND channel_id IN ({SWARM_CHANNELS})"
            ),
            [channel_id, agent, swarm.as_str()],
        )?;
        Ok(())
    }

    fn insert_message(&self, swarm: &SwarmId, message: &NewMessageRecord) -> StoreResult<i64> {
        self.require_channel(swarm, &message.channel_id)?;
        self.require_agent(swarm, &message.agent, true)?;
        self.0.execute(
            "INSERT INTO messages (channel_id, agent, body, created_at, run_id)
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![
                message.channel_id,
                message.agent,
                message.body,
                message.created_at,
                message.run_id
            ],
        )?;
        Ok(self.0.last_insert_rowid())
    }

    fn insert_server_run(&self, run: &NewServerRunRecord) -> StoreResult<i64> {
        self.0.execute(
            "INSERT INTO server_runs (started_at, version, git_commit, settings, instructions_sha256)
             VALUES (?, ?, ?, ?, ?)",
            [
                &run.started_at,
                &run.version,
                &run.git_commit,
                &run.settings,
                &run.instructions_sha256,
            ],
        )?;
        Ok(self.0.last_insert_rowid())
    }
}

/// Escapes `%`, `_` and `\` so text matches literally in a `LIKE ... ESCAPE '\'` pattern.
pub(super) fn like_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('%');
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// An FTS5 query that matches messages containing every word. Each word is
/// quoted, so FTS5 syntax in the words has no effect.
pub(super) fn fts_query(words: &[String]) -> String {
    words
        .iter()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An FTS5 query that matches messages containing every word and, if given,
/// the phrase: its words together and in order. The phrase is quoted as one
/// FTS5 string, which FTS5 splits into words as it does message bodies, so
/// `long-poll` matches "long-poll" and "long poll" but not "longpoll".
pub(super) fn fts_search(words: &[String], phrase: Option<&str>) -> String {
    let mut parts = Vec::with_capacity(words.len() + 1);
    parts.extend(words.iter().cloned());
    parts.extend(phrase.map(str::to_string));
    fts_query(&parts)
}
