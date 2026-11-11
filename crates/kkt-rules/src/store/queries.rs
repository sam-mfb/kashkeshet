//! [`Reads`] and [`Writes`] in kkt-db's terms, over any database.
//!
//! Every question about avatars, channels, memberships, messages, notes or
//! memory is bound to its swarm: through the row's own `swarm` column, or
//! through the channel or avatar the row belongs to. A question over many
//! messages asks for those in the channels its [`ChannelScope`] covers,
//! which a backend finds once for the whole question.

use super::schema::OPERATOR;
use super::*;
use kkt_db::search::{Excerpt, Search};
use kkt_db::{
    Condition, Connection, Count, Delete, Field, Group, Insert, Operand, OrderBy, Related, Row,
    Select, Sort, Update, Value, col, through,
};

/// The store's questions and changes, over one connection. Inside
/// [`Db::write`] the connection is a transaction.
pub(crate) struct Store<'a>(pub(crate) &'a dyn Connection);

/// How search excerpts are drawn: matches in `**`, `…` where the text is
/// cut, a window of 12 words.
const EXCERPT: Excerpt = Excerpt {
    open: "**",
    close: "**",
    ellipsis: "…",
    words: 12,
};

fn eq(column: &'static str, value: impl Into<Value>) -> Condition {
    Condition::Eq(col(column), value.into())
}

fn is(operand: Operand, value: impl Into<Value>) -> Condition {
    Condition::Eq(operand, value.into())
}

/// No row.
fn never() -> Condition {
    Condition::Any(Vec::new())
}

fn opt<T>(value: Option<T>, condition: impl FnOnce(T) -> Condition) -> Condition {
    value.map_or(Condition::True, condition)
}

fn limit(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// The error for a change tied to something not in the swarm.
fn outside(swarm: &SwarmId, what: &str) -> StoreError {
    StoreError::OutsideSwarm(format!("no {what} in swarm {}", swarm.as_str()))
}

// ---- avatars ----

/// An avatar row's fields, as [`agent_from_row`] reads them.
fn agent_fields() -> Vec<Field> {
    let mut fields: Vec<Field> = [
        "name",
        "created_at",
        "last_seen",
        "run_id",
        "revoked_at",
        "description",
        "client_name",
        "client_version",
        "client_recorded_at",
        "client_run_id",
        "lane",
        "lane_version",
        "lane_taken_at",
        "lane_run_id",
    ]
    .into_iter()
    .map(|c| Field::Value(col(c)))
    .collect();
    // The current version of the avatar's lane.
    fields.push(Field::Max(
        Related::new("lane_versions", "lane", "lane"),
        "version",
    ));
    fields
}

fn agent_from_row(r: &Row) -> StoreResult<AgentRecord> {
    Ok(AgentRecord {
        name: r.get(0)?,
        created_at: r.get(1)?,
        last_seen: r.get(2)?,
        run_id: r.get(3)?,
        revoked_at: r.get(4)?,
        description: r.get(5)?,
        client: match r.get::<Option<String>>(6)? {
            Some(name) => Some(AgentClient {
                name,
                version: r.get(7)?,
                recorded_at: r.get(8)?,
                run_id: r.get(9)?,
            }),
            None => None,
        },
        lane: match r.get::<Option<String>>(10)? {
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

fn agents_select() -> Select {
    Select::new("agents", agent_fields())
}

// ---- messages ----

fn message_fields() -> Vec<Field> {
    vec![
        Field::Value(col("id")),
        Field::Value(col("channel_id")),
        Field::Value(through("channel_id", "name")),
        Field::Value(col("agent")),
        Field::Value(col("body")),
        Field::Value(col("created_at")),
        Field::Value(col("run_id")),
    ]
}

fn message_from_row(r: &Row) -> StoreResult<MessageRecord> {
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

fn messages_select() -> Select {
    Select::new("messages", message_fields())
}

/// A message's channel is in `swarm`.
fn message_in(swarm: &SwarmId) -> Condition {
    is(through("channel_id", "swarm"), swarm.as_str())
}

// ---- channels and scopes ----

/// Whether a channel row is of `kind` for `viewer`. With no viewer, no
/// channel counts as joined or owned.
fn kind_condition(kind: &ChannelKind, viewer: Option<&str>) -> Condition {
    let member = match viewer {
        Some(v) => Condition::exists(
            Related::new("memberships", "channel_id", "id").filter(eq("agent", v)),
        ),
        None => never(),
    };
    let owner = match viewer {
        Some(v) => eq("owner", v),
        None => never(),
    };
    let either = |holds: bool, c: Condition| if holds { c } else { Condition::not(c) };
    Condition::All(vec![
        eq("hidden", kind.hidden),
        eq("is_open", kind.open),
        either(kind.member, member),
        either(kind.owner, owner),
    ])
}

/// The channels a scope covers, as a condition on channel rows: in its
/// swarm and of a kind it allows.
fn scope_condition(scope: &ChannelScope) -> Condition {
    eq("swarm", scope.swarm().as_str()).and(Condition::Any(
        scope
            .allowed()
            .iter()
            .map(|k| kind_condition(k, scope.viewer()))
            .collect(),
    ))
}

/// A channel's name or purpose contains `text`, ignoring ASCII case.
fn names_or_purpose(text: &str) -> Condition {
    Condition::Any(vec![
        Condition::ContainsIgnoringAsciiCase(col("name"), text.to_string()),
        Condition::ContainsIgnoringAsciiCase(col("purpose"), text.to_string()),
    ])
}

fn summary_fields(viewer: Option<&str>, after_id: Option<i64>) -> Vec<Field> {
    let viewer_is_member = Related::new("memberships", "channel_id", "id").filter(match viewer {
        Some(v) => eq("agent", v),
        None => never(),
    });
    // Without `after_id` nothing is counted, and the count reads back as
    // `None`.
    let newer = match after_id {
        Some(after) => Condition::Gt(col("id"), after.into()),
        None => never(),
    };
    let new_messages = Field::Count(Related::new("messages", "channel_id", "id").filter(newer));
    vec![
        Field::Value(col("id")),
        Field::Value(col("name")),
        Field::Value(col("purpose")),
        Field::Value(col("is_open")),
        Field::Value(col("created_by")),
        Field::Value(col("created_at")),
        Field::Count(Related::new("memberships", "channel_id", "id")),
        Field::Exists(viewer_is_member),
        new_messages,
        Field::Value(col("note")),
        Field::Value(col("note_edited_by")),
        Field::Value(col("note_edited_at")),
        Field::Value(col("owner")),
    ]
}

fn summary_from_row(r: &Row, counted: bool) -> StoreResult<ChannelSummary> {
    Ok(ChannelSummary {
        id: r.get(0)?,
        name: r.get(1)?,
        purpose: r.get(2)?,
        open: r.get(3)?,
        created_by: r.get(4)?,
        created_at: r.get(5)?,
        member_count: r.get(6)?,
        viewer_is_member: r.get(7)?,
        new_messages: if counted { r.get(8)? } else { None },
        note: ChannelNoteRecord {
            text: r.get(9)?,
            edited_by: r.get(10)?,
            edited_at: r.get(11)?,
        },
        owner: r.get(12)?,
    })
}

fn ids(rows: Vec<Row>) -> StoreResult<Vec<String>> {
    rows.iter().map(|r| Ok(r.get(0)?)).collect()
}

// ---- lanes ----

/// The current version's columns, in [`lane_from_row`]'s order.
const VERSION_COLUMNS: [&str; 8] = [
    "version",
    "description",
    "text",
    "saved_at",
    "run_id",
    "saved_by_admin",
    "source_folder",
    "source_commit",
];

fn versions() -> Related {
    Related::new("lane_versions", "lane", "name")
}

/// Lanes with their newest version. A lane always has one: it is saved
/// with its first.
fn lanes_select() -> Select {
    let mut fields: Vec<Field> = [
        "name",
        "created_at",
        "run_id",
        "retired_at",
        "retired_run_id",
        "retired_by_admin",
    ]
    .into_iter()
    .map(|c| Field::Value(col(c)))
    .collect();
    fields.extend(VERSION_COLUMNS.into_iter().map(|column| Field::Newest {
        related: versions(),
        by: "version",
        column,
    }));
    Select::new("lanes", fields).filter(Condition::exists(versions()))
}

fn lane_from_row(r: &Row) -> StoreResult<LaneRecord> {
    Ok(LaneRecord {
        name: r.get(0)?,
        created_at: r.get(1)?,
        run_id: r.get(2)?,
        retired: match r.get::<Option<String>>(3)? {
            Some(at) => Some(LaneRetiredRecord {
                at,
                run_id: r.get(4)?,
                by_admin: r.get(5)?,
            }),
            None => None,
        },
        current: version_from_row(r, 6)?,
    })
}

fn version_from_row(r: &Row, at: usize) -> StoreResult<LaneVersionRecord> {
    Ok(LaneVersionRecord {
        version: r.get(at)?,
        description: r.get(at + 1)?,
        text: r.get(at + 2)?,
        saved_at: r.get(at + 3)?,
        run_id: r.get(at + 4)?,
        saved_by_admin: r.get(at + 5)?,
        source: r
            .get::<Option<String>>(at + 6)?
            .map(|folder| -> StoreResult<LaneSourceRecord> {
                Ok(LaneSourceRecord {
                    folder,
                    commit: r.get(at + 7)?,
                })
            })
            .transpose()?,
    })
}

fn announcement_from_row(r: &Row) -> StoreResult<ServerAnnouncementRecord> {
    Ok(ServerAnnouncementRecord {
        version: r.get(0)?,
        text: r.get(1)?,
        saved_at: r.get(2)?,
        run_id: r.get(3)?,
        saved_by_admin: r.get(4)?,
    })
}

fn announcements() -> Select {
    Select::columns(
        "operator_instructions",
        &["version", "text", "saved_at", "run_id", "saved_by_admin"],
    )
    .order(Sort::desc(col("version")))
}

fn memory_from_row(r: &Row) -> StoreResult<MemoryEntryRecord> {
    Ok(MemoryEntryRecord {
        name: r.get(0)?,
        text: r.get(1)?,
        saved_at: r.get(2)?,
        run_id: r.get(3)?,
    })
}

fn rows<T>(rows: Vec<Row>, read: impl Fn(&Row) -> StoreResult<T>) -> StoreResult<Vec<T>> {
    rows.iter().map(read).collect()
}

impl Store<'_> {
    fn select(&self, select: &Select) -> StoreResult<Vec<Row>> {
        Ok(self.0.select(select)?)
    }

    fn first<T>(
        &self,
        select: &Select,
        read: impl Fn(&Row) -> StoreResult<T>,
    ) -> StoreResult<Option<T>> {
        self.0.first(select)?.as_ref().map(read).transpose()
    }

    fn count(&self, table: &'static str, filter: Condition) -> StoreResult<i64> {
        Ok(self.0.count(&Count { table, filter })?)
    }

    fn exists(&self, table: &'static str, filter: Condition) -> StoreResult<bool> {
        Ok(self.count(table, filter)? > 0)
    }

    /// Whether the avatar `name`, hidden or not, is in `swarm`.
    fn agent_in(&self, swarm: &SwarmId, name: &str) -> StoreResult<bool> {
        self.exists("agents", eq("name", name).and(eq("swarm", swarm.as_str())))
    }

    /// Fails unless the avatar `name` is in `swarm`. The operator, which is
    /// no avatar, passes when `operator` allows it.
    fn require_agent(&self, swarm: &SwarmId, name: &str, operator: bool) -> StoreResult<()> {
        if (operator && name == OPERATOR) || self.agent_in(swarm, name)? {
            return Ok(());
        }
        Err(outside(swarm, &format!("avatar {name}")))
    }

    /// Fails unless the channel `id`, hidden or not, is in `swarm`.
    fn require_channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<()> {
        if self.exists("channels", eq("id", id).and(eq("swarm", swarm.as_str())))? {
            return Ok(());
        }
        Err(outside(swarm, &format!("channel {id}")))
    }

    /// A message's channel is one `scope` covers: the channels are found
    /// once for the whole question.
    fn in_scope(&self, scope: &ChannelScope) -> StoreResult<Condition> {
        Ok(Condition::In(
            col("channel_id"),
            Box::new(Select::columns("channels", &["id"]).filter(scope_condition(scope))),
        ))
    }

    fn summary(
        &self,
        swarm: &SwarmId,
        id: &str,
        viewer: Option<&str>,
        after_id: Option<i64>,
        hidden: bool,
    ) -> StoreResult<Option<ChannelSummary>> {
        let select = Select::new("channels", summary_fields(viewer, after_id))
            .filter(eq("id", id))
            .filter(eq("hidden", hidden))
            .filter(eq("swarm", swarm.as_str()));
        self.first(&select, |r| summary_from_row(r, after_id.is_some()))
    }

    fn activity_rows(&self, filter: Condition) -> StoreResult<Vec<ActivityRecord>> {
        let groups = self.0.group(&Group {
            table: "messages",
            by: col("channel_id"),
            also: vec![through("channel_id", "name")],
            max: "id",
            filter,
        })?;
        rows(groups, |r| {
            Ok(ActivityRecord {
                channel_id: r.get(0)?,
                channel_name: r.get(1)?,
                new_messages: r.get(2)?,
                latest_id: r.get(3)?,
            })
        })
    }
}

impl Reads for Store<'_> {
    fn current_server_announcement(&self) -> StoreResult<Option<ServerAnnouncementRecord>> {
        self.first(&announcements(), announcement_from_row)
    }

    fn server_announcement_history(&self) -> StoreResult<Vec<ServerAnnouncementRecord>> {
        rows(self.select(&announcements())?, announcement_from_row)
    }

    fn lanes(&self) -> StoreResult<Vec<LaneRecord>> {
        rows(
            self.select(&lanes_select().order(Sort::asc(col("name"))))?,
            lane_from_row,
        )
    }

    fn lanes_after(&self, after: Option<&str>, n: usize) -> StoreResult<Vec<LaneRecord>> {
        let select = lanes_select()
            .filter(opt(after, |a| Condition::Gt(col("name"), a.into())))
            .order(Sort::asc(col("name")))
            .limit(limit(n));
        rows(self.select(&select)?, lane_from_row)
    }

    fn lane(&self, name: &str) -> StoreResult<Option<LaneRecord>> {
        self.first(&lanes_select().filter(eq("name", name)), lane_from_row)
    }

    fn lane_history(&self, name: &str) -> StoreResult<Vec<LaneVersionRecord>> {
        let select = Select::columns("lane_versions", &VERSION_COLUMNS)
            .filter(eq("lane", name))
            .order(Sort::desc(col("version")));
        rows(self.select(&select)?, |r| version_from_row(r, 0))
    }

    fn lane_version(&self, name: &str, version: i64) -> StoreResult<Option<LaneVersionRecord>> {
        let select = Select::columns("lane_versions", &VERSION_COLUMNS)
            .filter(eq("lane", name))
            .filter(eq("version", version));
        self.first(&select, |r| version_from_row(r, 0))
    }

    fn takeable_lanes_after(&self, after: Option<&str>, n: usize) -> StoreResult<Vec<LaneRecord>> {
        let select = lanes_select()
            .filter(Condition::IsNull(col("retired_at")))
            .filter(opt(after, |a| Condition::Gt(col("name"), a.into())))
            .order(Sort::asc(col("name")))
            .limit(limit(n));
        rows(self.select(&select)?, lane_from_row)
    }

    fn agent(&self, swarm: &SwarmId, name: &str) -> StoreResult<Option<AgentRecord>> {
        let select = agents_select()
            .filter(eq("name", name))
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()));
        self.first(&select, agent_from_row)
    }

    fn agent_name_taken(&self, name: &str) -> StoreResult<bool> {
        self.exists(
            "agents",
            Condition::EqIgnoringAsciiCase(col("name"), name.to_string()),
        )
    }

    fn agent_by_token_hash(
        &self,
        swarm: &SwarmId,
        hash: &[u8],
    ) -> StoreResult<Option<AgentRecord>> {
        let select = agents_select()
            .filter(eq("token_hash", hash))
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()));
        self.first(&select, agent_from_row)
    }

    fn agent_by_recovery_hash(
        &self,
        swarm: &SwarmId,
        name: &str,
        hash: &[u8],
    ) -> StoreResult<Option<AgentRecord>> {
        let select = agents_select()
            .filter(eq("name", name))
            .filter(eq("recovery_hash", hash))
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()));
        self.first(&select, agent_from_row)
    }

    fn memory_entry(
        &self,
        swarm: &SwarmId,
        agent: &str,
        name: &str,
    ) -> StoreResult<Option<MemoryEntryRecord>> {
        let select = Select::columns("memory_entries", &["name", "text", "saved_at", "run_id"])
            .filter(eq("agent", agent))
            .filter(eq("name", name))
            .filter(is(through("agent", "swarm"), swarm.as_str()));
        self.first(&select, memory_from_row)
    }

    fn memory_entries(&self, swarm: &SwarmId, agent: &str) -> StoreResult<Vec<MemoryEntryRecord>> {
        let select = Select::columns("memory_entries", &["name", "text", "saved_at", "run_id"])
            .filter(eq("agent", agent))
            .filter(is(through("agent", "swarm"), swarm.as_str()))
            .order(Sort::asc(col("name")));
        rows(self.select(&select)?, memory_from_row)
    }

    fn agents(
        &self,
        swarm: &SwarmId,
        text: Option<&str>,
        lane: Option<&str>,
        after_name: Option<&str>,
        n: usize,
    ) -> StoreResult<Vec<AgentRecord>> {
        let select = agents_select()
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()))
            .filter(opt(text, |t| {
                Condition::Any(vec![
                    Condition::ContainsIgnoringAsciiCase(col("name"), t.to_string()),
                    Condition::ContainsIgnoringAsciiCase(col("description"), t.to_string()),
                ])
            }))
            .filter(opt(lane, |l| eq("lane", l)))
            .filter(opt(after_name, |a| Condition::Gt(col("name"), a.into())))
            .order(Sort::asc(col("name")))
            .limit(limit(n));
        rows(self.select(&select)?, agent_from_row)
    }

    fn active_agent_count(&self, swarm: &SwarmId) -> StoreResult<i64> {
        self.count(
            "agents",
            Condition::All(vec![
                Condition::IsNull(col("revoked_at")),
                eq("hidden", false),
                eq("swarm", swarm.as_str()),
            ]),
        )
    }

    fn agents_by_last_seen(&self, swarm: &SwarmId) -> StoreResult<Vec<AgentRecord>> {
        let select = agents_select()
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()))
            .order(Sort::Desc(OrderBy::FirstOf(
                col("last_seen"),
                col("created_at"),
            )));
        rows(self.select(&select)?, agent_from_row)
    }

    fn channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<Option<ChannelFacts>> {
        let select = Select::columns("channels", &["id", "name", "is_open", "owner"])
            .filter(eq("id", id))
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()));
        self.first(&select, |r| {
            Ok(ChannelFacts {
                id: r.get(0)?,
                name: r.get(1)?,
                open: r.get(2)?,
                owner: r.get(3)?,
            })
        })
    }

    fn channel_id_taken(&self, id: &str) -> StoreResult<bool> {
        self.exists("channels", eq("id", id))
    }

    fn is_member(&self, swarm: &SwarmId, channel_id: &str, agent: &str) -> StoreResult<bool> {
        self.exists(
            "memberships",
            Condition::All(vec![
                eq("channel_id", channel_id),
                eq("agent", agent),
                is(through("channel_id", "swarm"), swarm.as_str()),
            ]),
        )
    }

    fn channel_ids(&self, filter: &ChannelFilter) -> StoreResult<Vec<String>> {
        let members = |agent: &str| {
            Condition::exists(
                Related::new("memberships", "channel_id", "id").filter(eq("agent", agent)),
            )
        };
        let shared = opt(filter.shared_with.as_deref(), |with| {
            match filter.scope.viewer() {
                Some(viewer) => members(viewer).and(members(with)),
                None => never(),
            }
        });
        let mut select = Select::columns("channels", &["id"])
            .filter(scope_condition(&filter.scope))
            .filter(shared)
            .filter(opt(filter.text.as_deref(), names_or_purpose))
            .order(Sort::asc(col("name")))
            .order(Sort::asc(col("id")))
            .limit(limit(filter.limit));
        if let Some((name, id)) = &filter.after {
            select = select.after(vec![name.into(), id.into()]);
        }
        ids(self.select(&select)?)
    }

    fn all_channel_ids(&self, swarm: &SwarmId, member: Option<&str>) -> StoreResult<Vec<String>> {
        let select = Select::columns("channels", &["id"])
            .filter(eq("hidden", false))
            .filter(eq("swarm", swarm.as_str()))
            .filter(opt(member, |m| {
                Condition::exists(
                    Related::new("memberships", "channel_id", "id").filter(eq("agent", m)),
                )
            }))
            .order(Sort::asc(col("name")))
            .order(Sort::asc(col("id")));
        ids(self.select(&select)?)
    }

    fn owned_channel_ids(&self, swarm: &SwarmId, owner: &str) -> StoreResult<Vec<String>> {
        let select = Select::columns("channels", &["id"])
            .filter(eq("hidden", false))
            .filter(eq("owner", owner))
            .filter(eq("swarm", swarm.as_str()))
            .order(Sort::asc(col("name")))
            .order(Sort::asc(col("id")));
        ids(self.select(&select)?)
    }

    fn hidden_channel_ids(&self, swarm: &SwarmId) -> StoreResult<Vec<String>> {
        let select = Select::columns("channels", &["id"])
            .filter(eq("hidden", true))
            .filter(eq("swarm", swarm.as_str()))
            .order(Sort::asc(col("name")))
            .order(Sort::asc(col("id")));
        ids(self.select(&select)?)
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
        n: usize,
    ) -> StoreResult<Vec<String>> {
        let select = Select::columns("memberships", &["agent"])
            .filter(eq("channel_id", channel_id))
            .filter(is(through("channel_id", "swarm"), swarm.as_str()))
            .order(Sort::asc(col("joined_at")))
            .order(Sort::asc(col("agent")))
            .limit(limit(n));
        ids(self.select(&select)?)
    }

    fn member_count(&self, swarm: &SwarmId, channel_id: &str) -> StoreResult<i64> {
        self.count(
            "memberships",
            eq("channel_id", channel_id).and(is(through("channel_id", "swarm"), swarm.as_str())),
        )
    }

    fn members_by_name(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        after_name: Option<&str>,
        n: usize,
    ) -> StoreResult<Vec<AgentRecord>> {
        let membership = Related::new("memberships", "agent", "name").filter(
            eq("channel_id", channel_id).and(is(through("channel_id", "swarm"), swarm.as_str())),
        );
        let select = agents_select()
            .filter(Condition::exists(membership))
            .filter(eq("swarm", swarm.as_str()))
            .filter(opt(after_name, |a| Condition::Gt(col("name"), a.into())))
            .order(Sort::asc(col("name")))
            .limit(limit(n));
        rows(self.select(&select)?, agent_from_row)
    }

    fn message(&self, swarm: &SwarmId, id: i64) -> StoreResult<Option<MessageRecord>> {
        let select = messages_select()
            .filter(eq("id", id))
            .filter(message_in(swarm));
        self.first(&select, message_from_row)
    }

    fn channel_messages(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        from_agent: Option<&str>,
        range: MessageRange,
        n: usize,
    ) -> StoreResult<Vec<MessageRecord>> {
        let select = messages_select()
            .filter(eq("channel_id", channel_id))
            .filter(message_in(swarm))
            .filter(opt(from_agent, |a| eq("agent", a)));
        let select = match range {
            MessageRange::After(id) => select
                .filter(Condition::Gt(col("id"), id.into()))
                .order(Sort::asc(col("id"))),
            MessageRange::Before(id) => select
                .filter(Condition::Lt(col("id"), id.into()))
                .order(Sort::desc(col("id"))),
            MessageRange::Latest => select.order(Sort::desc(col("id"))),
        };
        rows(self.select(&select.limit(limit(n)))?, message_from_row)
    }

    fn newer_messages(&self, filter: &NewerFilter, n: usize) -> StoreResult<Vec<MessageRecord>> {
        let select = messages_select()
            .filter(Condition::Gt(col("id"), filter.after_id.into()))
            .filter(self.in_scope(&filter.scope)?)
            .filter(listed(filter.channel_ids.as_deref()))
            .order(Sort::asc(col("id")))
            .limit(limit(n));
        rows(self.select(&select)?, message_from_row)
    }

    fn count_newer_messages(&self, filter: &NewerFilter) -> StoreResult<i64> {
        self.count(
            "messages",
            Condition::All(vec![
                Condition::Gt(col("id"), filter.after_id.into()),
                self.in_scope(&filter.scope)?,
                listed(filter.channel_ids.as_deref()),
            ]),
        )
    }

    fn latest_message_from(&self, filter: &SentFilter) -> StoreResult<Option<MessageRecord>> {
        let select = messages_select()
            .filter(eq("agent", filter.agent.as_str()))
            .filter(opt(filter.channel_id.as_deref(), |c| eq("channel_id", c)))
            .filter(self.in_scope(&filter.scope)?)
            .order(Sort::desc(col("id")));
        self.first(&select, message_from_row)
    }

    fn recent_messages(&self, swarm: &SwarmId, n: usize) -> StoreResult<Vec<MessageRecord>> {
        let select = messages_select()
            .filter(is(through("channel_id", "hidden"), false))
            .filter(message_in(swarm))
            .order(Sort::desc(col("id")))
            .limit(limit(n));
        rows(self.select(&select)?, message_from_row)
    }

    fn message_count_from(&self, swarm: &SwarmId, agent: &str) -> StoreResult<i64> {
        self.count("messages", eq("agent", agent).and(message_in(swarm)))
    }

    fn search(&self, filter: &SearchFilter) -> StoreResult<Vec<SearchHitRecord>> {
        let mut terms = filter.words.clone();
        terms.extend(filter.phrase.clone());
        let select = Select::new(
            "messages",
            vec![
                Field::Value(col("id")),
                Field::Value(col("channel_id")),
                Field::Value(through("channel_id", "name")),
                Field::Value(col("agent")),
                Field::Value(col("created_at")),
            ],
        )
        .filter(self.in_scope(&filter.scope)?)
        .filter(opt(filter.channel_id.as_deref(), |c| eq("channel_id", c)))
        .filter(opt(filter.from_agent.as_deref(), |a| eq("agent", a)))
        .filter(opt(filter.after_id, |a| Condition::Gt(col("id"), a.into())))
        .filter(opt(filter.before_id, |b| {
            Condition::Lt(col("id"), b.into())
        }))
        .filter(opt(filter.since.as_deref(), |s| {
            Condition::Ge(col("created_at"), s.into())
        }))
        .filter(opt(filter.until.as_deref(), |u| {
            Condition::Lt(col("created_at"), u.into())
        }))
        .order(Sort::desc(col("id")))
        .limit(limit(filter.limit));
        let hits = self.0.search(&Search {
            select,
            terms,
            excerpt: EXCERPT,
        })?;
        hits.into_iter()
            .map(|hit| {
                let r = &hit.row;
                Ok(SearchHitRecord {
                    message_id: r.get(0)?,
                    channel_id: r.get(1)?,
                    channel_name: r.get(2)?,
                    agent: r.get(3)?,
                    created_at: r.get(4)?,
                    excerpt: hit.excerpt,
                })
            })
            .collect()
    }

    fn activity(&self, scope: &ChannelScope, after_id: i64) -> StoreResult<Vec<ActivityRecord>> {
        let filter = Condition::Gt(col("id"), after_id.into()).and(self.in_scope(scope)?);
        self.activity_rows(filter)
    }

    fn channel_use(&self, swarm: &SwarmId) -> StoreResult<Vec<ChannelUseRecord>> {
        let messages = || Related::new("messages", "channel_id", "id");
        let select = Select::new(
            "channels",
            vec![
                Field::Value(col("id")),
                Field::Count(messages()),
                Field::Newest {
                    related: messages(),
                    by: "id",
                    column: "created_at",
                },
                Field::Newest {
                    related: messages(),
                    by: "id",
                    column: "agent",
                },
            ],
        )
        .filter(eq("swarm", swarm.as_str()));
        rows(self.select(&select)?, |r| {
            Ok(ChannelUseRecord {
                channel_id: r.get(0)?,
                message_count: r.get(1)?,
                last_message_at: r.get(2)?,
                last_speaker: r.get(3)?,
            })
        })
    }

    fn memberships(&self, swarm: &SwarmId) -> StoreResult<Vec<MembershipRecord>> {
        let select = Select::new(
            "memberships",
            vec![
                Field::Value(col("channel_id")),
                Field::Value(through("channel_id", "name")),
                Field::Value(col("agent")),
            ],
        )
        .filter(is(through("channel_id", "swarm"), swarm.as_str()))
        .order(Sort::asc(col("joined_at")))
        .order(Sort::asc(col("seq")));
        rows(self.select(&select)?, |r| {
            Ok(MembershipRecord {
                channel_id: r.get(0)?,
                channel_name: r.get(1)?,
                agent: r.get(2)?,
            })
        })
    }

    fn message_times_since(
        &self,
        swarm: &SwarmId,
        since: &str,
    ) -> StoreResult<Vec<MessageTimeRecord>> {
        let select = Select::columns("messages", &["channel_id", "created_at"])
            .filter(Condition::Ge(col("created_at"), since.into()))
            .filter(message_in(swarm))
            .order(Sort::asc(col("created_at")))
            .order(Sort::asc(col("id")));
        rows(self.select(&select)?, |r| {
            Ok(MessageTimeRecord {
                channel_id: r.get(0)?,
                created_at: r.get(1)?,
            })
        })
    }

    fn server_runs(&self) -> StoreResult<Vec<ServerRunRecord>> {
        let select = Select::columns(
            "server_runs",
            &[
                "id",
                "started_at",
                "version",
                "git_commit",
                "settings",
                "instructions_sha256",
            ],
        )
        .order(Sort::desc(col("id")));
        rows(self.select(&select)?, |r| {
            Ok(ServerRunRecord {
                id: r.get(0)?,
                started_at: r.get(1)?,
                version: r.get(2)?,
                git_commit: r.get(3)?,
                settings: r.get(4)?,
                instructions_sha256: r.get(5)?,
            })
        })
    }

    fn any_channel(&self, swarm: &SwarmId, id: &str) -> StoreResult<Option<AnyChannelRecord>> {
        let select = Select::new(
            "channels",
            vec![
                Field::Value(col("id")),
                Field::Value(col("name")),
                Field::Value(col("purpose")),
                Field::Value(col("is_open")),
                Field::Value(col("created_by")),
                Field::Value(col("created_at")),
                Field::Count(Related::new("memberships", "channel_id", "id")),
                Field::Value(col("note")),
                Field::Value(col("note_edited_by")),
                Field::Value(col("note_edited_at")),
                Field::Value(col("owner")),
                Field::Value(col("hidden")),
                Field::Value(col("run_id")),
                Field::Value(col("note_run_id")),
            ],
        )
        .filter(eq("id", id))
        .filter(eq("swarm", swarm.as_str()));
        self.first(&select, |r| {
            Ok(AnyChannelRecord {
                summary: ChannelSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    purpose: r.get(2)?,
                    open: r.get(3)?,
                    created_by: r.get(4)?,
                    created_at: r.get(5)?,
                    member_count: r.get(6)?,
                    viewer_is_member: false,
                    new_messages: None,
                    note: ChannelNoteRecord {
                        text: r.get(7)?,
                        edited_by: r.get(8)?,
                        edited_at: r.get(9)?,
                    },
                    owner: r.get(10)?,
                },
                hidden: r.get(11)?,
                run_id: r.get(12)?,
                note_run_id: r.get(13)?,
            })
        })
    }

    fn any_channel_ids(
        &self,
        swarm: &SwarmId,
        text: Option<&str>,
        after: Option<&(String, String)>,
        n: usize,
    ) -> StoreResult<Vec<String>> {
        let mut select = Select::columns("channels", &["id"])
            .filter(eq("swarm", swarm.as_str()))
            .filter(opt(text, names_or_purpose))
            .order(Sort::asc(col("name")))
            .order(Sort::asc(col("id")))
            .limit(limit(n));
        if let Some((name, id)) = after {
            select = select.after(vec![name.into(), id.into()]);
        }
        ids(self.select(&select)?)
    }

    fn any_activity(&self, swarm: &SwarmId, after_id: i64) -> StoreResult<Vec<ActivityRecord>> {
        self.activity_rows(Condition::Gt(col("id"), after_id.into()).and(message_in(swarm)))
    }

    fn any_newer_messages(
        &self,
        swarm: &SwarmId,
        after_id: i64,
        channel_ids: Option<&[String]>,
        n: usize,
    ) -> StoreResult<Vec<MessageRecord>> {
        let select = messages_select()
            .filter(Condition::Gt(col("id"), after_id.into()))
            .filter(message_in(swarm))
            .filter(listed(channel_ids))
            .order(Sort::asc(col("id")))
            .limit(limit(n));
        rows(self.select(&select)?, message_from_row)
    }

    fn count_any_newer_messages(
        &self,
        swarm: &SwarmId,
        after_id: i64,
        channel_ids: Option<&[String]>,
    ) -> StoreResult<i64> {
        self.count(
            "messages",
            Condition::All(vec![
                Condition::Gt(col("id"), after_id.into()),
                message_in(swarm),
                listed(channel_ids),
            ]),
        )
    }
}

/// Only the channels listed, if a list is given.
fn listed(channel_ids: Option<&[String]>) -> Condition {
    opt(channel_ids, |ids| {
        Condition::OneOf(col("channel_id"), ids.iter().map(Value::from).collect())
    })
}

impl Writes for Store<'_> {
    fn append_server_announcement(
        &self,
        text: &str,
        at: &str,
        run_id: i64,
        saved_by_admin: Option<&str>,
    ) -> StoreResult<ServerAnnouncementRecord> {
        let version = self.0.insert(
            &Insert::new("operator_instructions")
                .value("text", text)
                .value("saved_at", at)
                .value("run_id", run_id)
                .value("saved_by_admin", saved_by_admin),
        )?;
        Ok(ServerAnnouncementRecord {
            version: version
                .ok_or_else(|| StoreError::Database("no version was assigned".into()))?,
            text: text.into(),
            saved_at: at.into(),
            run_id,
            saved_by_admin: saved_by_admin.map(str::to_string),
        })
    }

    fn insert_lane(&self, name: &str, at: &str, run_id: i64) -> StoreResult<()> {
        self.0.insert(
            &Insert::new("lanes")
                .value("name", name)
                .value("created_at", at)
                .value("run_id", run_id),
        )?;
        Ok(())
    }

    fn append_lane_version(&self, v: &NewLaneVersionRecord) -> StoreResult<LaneVersionRecord> {
        let newest = self.select(
            &Select::new("lanes", vec![Field::Max(versions(), "version")])
                .filter(eq("name", v.lane.as_str())),
        )?;
        let newest: Option<i64> = match newest.first() {
            Some(r) => r.get(0)?,
            None => None,
        };
        let version = newest.unwrap_or(0) + 1;
        self.0.insert(
            &Insert::new("lane_versions")
                .value("lane", v.lane.as_str())
                .value("version", version)
                .value("description", v.description.as_str())
                .value("text", v.text.as_str())
                .value("saved_at", v.saved_at.as_str())
                .value("run_id", v.run_id)
                .value("saved_by_admin", v.saved_by_admin.as_deref())
                .value(
                    "source_folder",
                    v.source.as_ref().map(|s| s.folder.as_str()),
                )
                .value(
                    "source_commit",
                    v.source.as_ref().and_then(|s| s.commit.as_deref()),
                ),
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
        self.0.update(
            &Update::new(
                "lanes",
                eq("name", name).and(Condition::IsNull(col("retired_at"))),
            )
            .set("retired_at", retired.at.as_str())
            .set("retired_run_id", retired.run_id)
            .set("retired_by_admin", retired.by_admin.as_deref()),
        )?;
        Ok(())
    }

    fn insert_swarm(&self, swarm: &SwarmId, at: &str, run_id: i64) -> StoreResult<()> {
        self.0.insert(
            &Insert::new("swarms")
                .value("id", swarm.as_str())
                .value("created_at", at)
                .value("run_id", run_id),
        )?;
        Ok(())
    }

    fn insert_agent(&self, swarm: &SwarmId, agent: &NewAgentRecord) -> StoreResult<()> {
        self.0.insert(
            &Insert::new("agents")
                .value("name", agent.name.as_str())
                .value("token_hash", agent.token_hash.as_slice())
                .value("recovery_hash", agent.recovery_hash.as_deref())
                .value("created_at", agent.created_at.as_str())
                .value("last_seen", agent.last_seen.as_deref())
                .value("run_id", agent.run_id)
                .value("swarm", swarm.as_str()),
        )?;
        Ok(())
    }

    fn set_token_hash(&self, swarm: &SwarmId, name: &str, hash: &[u8]) -> StoreResult<()> {
        self.0
            .update(&Update::new("agents", agent_key(swarm, name)).set("token_hash", hash))?;
        Ok(())
    }

    fn set_recovery_hash(&self, swarm: &SwarmId, name: &str, hash: &[u8]) -> StoreResult<()> {
        self.0
            .update(&Update::new("agents", agent_key(swarm, name)).set("recovery_hash", hash))?;
        Ok(())
    }

    fn save_memory_entry(
        &self,
        swarm: &SwarmId,
        agent: &str,
        entry: &MemoryEntryRecord,
    ) -> StoreResult<()> {
        self.require_agent(swarm, agent, false)?;
        self.0.save(
            &Insert::new("memory_entries")
                .value("agent", agent)
                .value("name", entry.name.as_str())
                .value("text", entry.text.as_str())
                .value("saved_at", entry.saved_at.as_str())
                .value("run_id", entry.run_id),
        )?;
        Ok(())
    }

    fn delete_memory_entry(&self, swarm: &SwarmId, agent: &str, name: &str) -> StoreResult<bool> {
        let deleted = self.0.delete(&Delete::new(
            "memory_entries",
            Condition::All(vec![
                eq("agent", agent),
                eq("name", name),
                is(through("agent", "swarm"), swarm.as_str()),
            ]),
        ))?;
        Ok(deleted > 0)
    }

    fn set_last_seen(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()> {
        self.0
            .update(&Update::new("agents", agent_key(swarm, name)).set("last_seen", at))?;
        Ok(())
    }

    fn set_description(
        &self,
        swarm: &SwarmId,
        name: &str,
        description: Option<&str>,
    ) -> StoreResult<()> {
        self.0.update(
            &Update::new("agents", agent_key(swarm, name)).set("description", description),
        )?;
        Ok(())
    }

    fn set_agent_client(
        &self,
        swarm: &SwarmId,
        name: &str,
        client: Option<&AgentClient>,
    ) -> StoreResult<()> {
        self.0.update(
            &Update::new("agents", agent_key(swarm, name))
                .set("client_name", client.map(|c| c.name.as_str()))
                .set("client_version", client.map(|c| c.version.as_str()))
                .set("client_recorded_at", client.map(|c| c.recorded_at.as_str()))
                .set("client_run_id", client.map(|c| c.run_id)),
        )?;
        Ok(())
    }

    fn set_agent_lane(
        &self,
        swarm: &SwarmId,
        name: &str,
        lane: Option<&TakenLaneRecord>,
    ) -> StoreResult<()> {
        self.0.update(
            &Update::new("agents", agent_key(swarm, name))
                .set("lane", lane.map(|l| l.lane.as_str()))
                .set("lane_version", lane.map(|l| l.version))
                .set("lane_taken_at", lane.map(|l| l.taken_at.as_str()))
                .set("lane_run_id", lane.map(|l| l.run_id)),
        )?;
        Ok(())
    }

    fn set_revoked(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()> {
        self.0
            .update(&Update::new("agents", agent_key(swarm, name)).set("revoked_at", at))?;
        Ok(())
    }

    fn insert_channel(&self, swarm: &SwarmId, channel: &NewChannelRecord) -> StoreResult<()> {
        self.require_agent(swarm, &channel.created_by, true)?;
        if let Some(owner) = &channel.owner {
            self.require_agent(swarm, owner, false)?;
        }
        self.0.insert(
            &Insert::new("channels")
                .value("id", channel.id.as_str())
                .value("name", channel.name.as_str())
                .value("purpose", channel.purpose.as_deref())
                .value("is_open", channel.open)
                .value("created_by", channel.created_by.as_str())
                .value("created_at", channel.created_at.as_str())
                .value("owner", channel.owner.as_deref())
                .value("run_id", channel.run_id)
                .value("swarm", swarm.as_str()),
        )?;
        Ok(())
    }

    fn set_purpose(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        purpose: Option<&str>,
    ) -> StoreResult<()> {
        self.0.update(
            &Update::new("channels", channel_key(swarm, channel_id)).set("purpose", purpose),
        )?;
        Ok(())
    }

    fn set_channel_open(&self, swarm: &SwarmId, channel_id: &str, open: bool) -> StoreResult<()> {
        self.0.update(
            &Update::new("channels", channel_key(swarm, channel_id)).set("is_open", open),
        )?;
        Ok(())
    }

    fn hide_channel(&self, swarm: &SwarmId, channel_id: &str) -> StoreResult<()> {
        let hidden = self
            .0
            .update(&Update::new("channels", channel_key(swarm, channel_id)).set("hidden", true))?;
        if hidden > 0 {
            self.0
                .delete(&Delete::new("memberships", eq("channel_id", channel_id)))?;
        }
        Ok(())
    }

    fn set_owner(&self, swarm: &SwarmId, channel_id: &str, owner: Option<&str>) -> StoreResult<()> {
        if let Some(owner) = owner {
            self.require_agent(swarm, owner, false)?;
        }
        self.0
            .update(&Update::new("channels", channel_key(swarm, channel_id)).set("owner", owner))?;
        Ok(())
    }

    fn hide_agent(&self, swarm: &SwarmId, name: &str, at: &str) -> StoreResult<()> {
        let hidden = self.0.update(
            &Update::new("agents", agent_key(swarm, name))
                .set("hidden", true)
                .set_if_null("revoked_at", at),
        )?;
        if hidden > 0 {
            self.0
                .delete(&Delete::new("memberships", eq("agent", name)))?;
            self.0.update(
                &Update::new(
                    "channels",
                    eq("owner", name).and(eq("swarm", swarm.as_str())),
                )
                .set("owner", Value::Null),
            )?;
        }
        Ok(())
    }

    fn set_note(
        &self,
        swarm: &SwarmId,
        channel_id: &str,
        edit: &NoteEditRecord,
    ) -> StoreResult<()> {
        self.0.update(
            &Update::new("channels", channel_key(swarm, channel_id))
                .set("note", edit.text.as_str())
                .set("note_edited_by", edit.editor.as_str())
                .set("note_edited_at", edit.at.as_str())
                .set("note_run_id", edit.run_id),
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
        self.0.insert_unless_present(
            &Insert::new("memberships")
                .value("channel_id", channel_id)
                .value("agent", agent)
                .value("joined_at", joined_at),
        )?;
        Ok(())
    }

    fn remove_member(&self, swarm: &SwarmId, channel_id: &str, agent: &str) -> StoreResult<()> {
        self.0.delete(&Delete::new(
            "memberships",
            Condition::All(vec![
                eq("channel_id", channel_id),
                eq("agent", agent),
                is(through("channel_id", "swarm"), swarm.as_str()),
            ]),
        ))?;
        Ok(())
    }

    fn insert_message(&self, swarm: &SwarmId, message: &NewMessageRecord) -> StoreResult<i64> {
        self.require_channel(swarm, &message.channel_id)?;
        self.require_agent(swarm, &message.agent, true)?;
        let id = self.0.insert(
            &Insert::new("messages")
                .value("channel_id", message.channel_id.as_str())
                .value("agent", message.agent.as_str())
                .value("body", message.body.as_str())
                .value("created_at", message.created_at.as_str())
                .value("run_id", message.run_id),
        )?;
        id.ok_or_else(|| StoreError::Database("no message ID was assigned".into()))
    }

    fn insert_server_run(&self, run: &NewServerRunRecord) -> StoreResult<i64> {
        let id = self.0.insert(
            &Insert::new("server_runs")
                .value("started_at", run.started_at.as_str())
                .value("version", run.version.as_str())
                .value("git_commit", run.git_commit.as_str())
                .value("settings", run.settings.as_str())
                .value("instructions_sha256", run.instructions_sha256.as_str()),
        )?;
        id.ok_or_else(|| StoreError::Database("no run ID was assigned".into()))
    }
}

fn agent_key(swarm: &SwarmId, name: &str) -> Condition {
    eq("name", name).and(eq("swarm", swarm.as_str()))
}

fn channel_key(swarm: &SwarmId, id: &str) -> Condition {
    eq("id", id).and(eq("swarm", swarm.as_str()))
}
