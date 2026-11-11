//! Lanes: named instructions an agent can take on. The operator (the human,
//! or a named admin agent) creates and edits them; each save is a new,
//! immutable version recording who saved it and when. A lane is never
//! deleted, only retired. A lane grants no permissions.
//!
//! An agent takes a lane at `register` or `reclaim`, or with `take_lane`,
//! and keeps the version it took until it takes the lane again. Any number
//! of agents may hold one lane. Taking, switching, moving to a newer
//! version of, or dropping a lane is announced by the agent in every
//! channel it is in; registering with one announces nothing.
use super::*;
use crate::{
    AgentLaneRecord, LaneRecord, LaneRetiredRecord, LaneSourceRecord, LaneVersionRecord,
    NewLaneVersionRecord, TakenLaneRecord,
};
use kkt_names::MAX_LANE_NAME_CHARS;

/// The lane an agent holds, as others see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentLane {
    pub name: String,
    /// The version the agent took.
    pub version: i64,
    /// The lane's newest version.
    pub current_version: i64,
    pub taken_at: String,
}

impl AgentLane {
    /// Whether the lane has a newer version than the one the agent took.
    pub fn out_of_date(&self) -> bool {
        self.version < self.current_version
    }
}

impl From<AgentLaneRecord> for AgentLane {
    fn from(l: AgentLaneRecord) -> Self {
        Self {
            name: l.taken.lane,
            version: l.taken.version,
            current_version: l.current_version,
            taken_at: l.taken.taken_at,
        }
    }
}

/// The lane an agent holds, with the instructions of the version it took:
/// what the agent itself is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeldLane {
    pub lane: AgentLane,
    pub description: String,
    pub text: String,
}

/// What taking a lane changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "change")]
pub enum LaneChange {
    /// The agent held no lane and now holds one.
    Took,
    /// The agent held the lane `from` and now holds another.
    Switched { from: String },
    /// The agent moved from version `from` of its lane to the newest.
    Updated { from: i64 },
    /// The agent held the lane `from` and now holds none.
    Dropped { from: String },
    /// Nothing: the agent already held what it asked for.
    Unchanged,
}

/// The lane an agent holds after `take_lane`, and what changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneTaking {
    pub held: Option<HeldLane>,
    pub change: LaneChange,
}

/// A lane and its current version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Lane {
    pub name: String,
    pub created_at: String,
    pub run_id: i64,
    /// When and by whom it was retired; `None` while it can be taken.
    pub retired: Option<LaneRetired>,
    pub current: LaneVersion,
}

/// One saved version of a lane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneVersion {
    pub version: i64,
    pub description: String,
    pub text: String,
    pub saved_at: String,
    pub run_id: i64,
    /// The admin agent that saved it, acting as the operator, such as
    /// `admin-sentinel`; `None` when the human operator saved it.
    pub saved_by_admin: Option<String>,
    /// Where `kkt lanes import` read it from; `None` when it was saved on the
    /// web page or at `/mcp-admin`.
    pub source: Option<LaneSource>,
}

/// The folder a lane version was imported from, and the git commit checked
/// out there, if the folder was in a git checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneSource {
    pub folder: String,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneRetired {
    pub at: String,
    pub run_id: i64,
    /// The admin agent that retired it; `None` when the human operator did.
    pub by_admin: Option<String>,
}

/// A lane with every version, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneHistory {
    pub lane: Lane,
    pub versions: Vec<LaneVersion>,
}

/// What the operator saves as a lane's next version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneSave {
    pub name: String,
    pub description: String,
    pub text: String,
}

impl From<LaneVersionRecord> for LaneVersion {
    fn from(v: LaneVersionRecord) -> Self {
        Self {
            version: v.version,
            description: v.description,
            text: v.text,
            saved_at: v.saved_at,
            run_id: v.run_id,
            saved_by_admin: v.saved_by_admin,
            source: v.source.map(|s| LaneSource {
                folder: s.folder,
                commit: s.commit,
            }),
        }
    }
}

/// A save checked against the lane rules: its name, its description trimmed
/// to one line of at most `max_description_chars`, and its instructions as
/// given, at most `max_text_chars`.
pub(super) fn checked_save(
    save: LaneSave,
    max_description_chars: usize,
    max_text_chars: usize,
) -> Result<LaneSave> {
    names::check_lane_name(&save.name, MAX_LANE_NAME_CHARS).map_err(RuleError::invalid)?;
    let description = save.description.trim().to_string();
    if description.is_empty() {
        return Err(RuleError::invalid("the lane needs a description"));
    }
    if description.contains(['\n', '\r']) {
        return Err(RuleError::invalid(
            "the lane's description must be one line",
        ));
    }
    if description.chars().count() > max_description_chars {
        return Err(RuleError::invalid(format!(
            "the lane's description must be at most {max_description_chars} characters"
        )));
    }
    if save.text.trim().is_empty() {
        return Err(RuleError::invalid("the lane needs instructions"));
    }
    if save.text.chars().count() > max_text_chars {
        return Err(RuleError::invalid(format!(
            "the lane's instructions must be at most {max_text_chars} characters"
        )));
    }
    Ok(LaneSave {
        name: save.name,
        description,
        text: save.text,
    })
}

/// What [`write_lane`] did.
pub(super) enum LaneWrite {
    /// The lane is retired; nothing changed.
    Retired,
    /// Its current version already says this; nothing changed.
    Unchanged(Box<LaneRecord>),
    /// A new version was saved: `created` when the lane is new.
    Saved { created: bool, version: i64 },
}

/// The lane save rule, shared by the operator's save and by import: a
/// retired lane is not changed, a save that matches the current version
/// changes nothing, and otherwise the lane is created if `found` is `None`
/// and gets its next version. `found` is the stored lane of `save`'s name.
/// `run_id` is asked for only when something is written. `save` is checked
/// already ([`checked_save`]).
pub(super) fn write_lane(
    s: &Writer<'_>,
    found: Option<LaneRecord>,
    save: LaneSave,
    at: &str,
    run_id: impl FnOnce(&Writer<'_>) -> Result<i64>,
    saved_by_admin: Option<String>,
    source: Option<LaneSourceRecord>,
) -> Result<LaneWrite> {
    let created = match found {
        Some(lane) if lane.retired.is_some() => return Ok(LaneWrite::Retired),
        Some(lane)
            if lane.current.description == save.description && lane.current.text == save.text =>
        {
            return Ok(LaneWrite::Unchanged(Box::new(lane)));
        }
        Some(_) => false,
        None => true,
    };
    let run_id = run_id(s)?;
    if created {
        s.insert_lane(&save.name, at, run_id)?;
    }
    let saved = s.append_lane_version(&NewLaneVersionRecord {
        lane: save.name,
        description: save.description,
        text: save.text,
        saved_at: at.to_string(),
        run_id,
        saved_by_admin,
        source,
    })?;
    Ok(LaneWrite::Saved {
        created,
        version: saved.version,
    })
}

impl From<LaneRecord> for Lane {
    fn from(l: LaneRecord) -> Self {
        Self {
            name: l.name,
            created_at: l.created_at,
            run_id: l.run_id,
            retired: l.retired.map(|r| LaneRetired {
                at: r.at,
                run_id: r.run_id,
                by_admin: r.by_admin,
            }),
            current: l.current.into(),
        }
    }
}

/// Whether a save may only create a lane, or only change one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneMustBe {
    New,
    Existing,
}

/// The author stored for the operator's change, [`Audit::author`], or the
/// refusal for an avatar: only the operator edits lanes.
fn operator_name(actor: Actor, what: &str) -> Result<Option<String>> {
    match actor {
        Actor::Operator(operator) => Ok(operator.audit.author()),
        Actor::Avatar(_) => Err(RuleError::operator_only(what)),
    }
}

impl Rules {
    /// Every lane, retired ones included, in name order.
    pub async fn lanes(&self, operator: &Operator) -> Result<Vec<Lane>> {
        operator.may(Permission::Read)?;
        self.db()
            .read(|s, _| Ok(s.lanes()?.into_iter().map(Lane::from).collect()))
            .await
    }

    /// One page of lanes, retired ones included, in name order: at most
    /// `limit`, after the lane named by `cursor` if given. The cursor is the
    /// name of the last lane on the previous page.
    pub async fn lanes_page(
        &self,
        operator: &Operator,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<Lane>> {
        operator.may(Permission::Read)?;
        if let Some(c) = &cursor {
            names::check_lane_name(c, MAX_LANE_NAME_CHARS)
                .map_err(|_| RuleError::invalid("cursor is not one list_lanes gave"))?;
        }
        self.db()
            .read(move |s, _| {
                let mut items: Vec<Lane> = s
                    .lanes_after(cursor.as_deref(), limit + 1)?
                    .into_iter()
                    .map(Lane::from)
                    .collect();
                let more = items.len() > limit;
                items.truncate(limit);
                let next_cursor = match (more, items.last()) {
                    (true, Some(last)) => Some(last.name.clone()),
                    _ => None,
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// One lane, retired or not.
    pub async fn lane(&self, operator: &Operator, name: String) -> Result<Lane> {
        operator.may(Permission::Read)?;
        self.db()
            .read(move |s, _| {
                s.lane(&name)?
                    .map(Lane::from)
                    .ok_or_else(|| RuleError::lane_not_found(&name))
            })
            .await
    }

    /// One lane, retired or not, with every version, newest first.
    pub async fn lane_history(&self, operator: &Operator, name: String) -> Result<LaneHistory> {
        operator.may(Permission::Read)?;
        self.db()
            .read(move |s, _| {
                let lane = s
                    .lane(&name)?
                    .map(Lane::from)
                    .ok_or_else(|| RuleError::lane_not_found(&name))?;
                let versions = s
                    .lane_history(&name)?
                    .into_iter()
                    .map(LaneVersion::from)
                    .collect();
                Ok(LaneHistory { lane, versions })
            })
            .await
    }

    /// Creates the lane, or saves its next version. The description is one
    /// line, trimmed; the instructions are kept as given. Saving what the
    /// current version already says changes nothing. A retired lane cannot
    /// be saved. The human or an admin agent may save; the version records
    /// which.
    pub async fn save_lane(&self, actor: Actor, save: LaneSave) -> Result<Lane> {
        self.save(actor, save, None).await
    }

    /// As [`Rules::save_lane`], but only a new lane: refused if a lane, retired
    /// or not, has the name.
    pub async fn create_lane(&self, actor: Actor, save: LaneSave) -> Result<Lane> {
        self.save(actor, save, Some(LaneMustBe::New)).await
    }

    /// As [`Rules::save_lane`], but only an existing lane: `not_found` if
    /// none has the name.
    pub async fn update_lane(&self, actor: Actor, save: LaneSave) -> Result<Lane> {
        self.save(actor, save, Some(LaneMustBe::Existing)).await
    }

    async fn save(
        &self,
        actor: Actor,
        save: LaneSave,
        must_be: Option<LaneMustBe>,
    ) -> Result<Lane> {
        actor.may_change()?;
        let saved_by_admin = operator_name(actor, "save a lane")?;
        let LaneSave {
            name,
            description,
            text,
        } = checked_save(
            save,
            self.settings().max_lane_description_chars,
            self.settings().max_lane_chars,
        )?;
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        self.db()
            .write(move |s, _| {
                let found = s.lane(&name)?;
                match (&found, must_be) {
                    (Some(_), Some(LaneMustBe::New)) => {
                        return Err(RuleError::invalid(format!(
                            "a lane named {name} already exists"
                        )));
                    }
                    (None, Some(LaneMustBe::Existing)) => {
                        return Err(RuleError::lane_not_found(&name));
                    }
                    _ => {}
                }
                let save = LaneSave {
                    name: name.clone(),
                    description,
                    text,
                };
                let wrote = write_lane(s, found, save, &at, |_| Ok(run_id), saved_by_admin, None)?;
                match wrote {
                    LaneWrite::Retired => Err(RuleError::invalid(format!(
                        "the lane {name} is retired, so it cannot be changed"
                    ))),
                    LaneWrite::Unchanged(lane) => Ok(Lane::from(*lane)),
                    LaneWrite::Saved { .. } => s
                        .lane(&name)?
                        .map(Lane::from)
                        .ok_or_else(|| RuleError::lane_not_found(&name)),
                }
            })
            .await
    }

    /// Retires the lane: it stays, with its history, but can no longer be
    /// taken or changed. Retiring a retired lane changes nothing.
    pub async fn retire_lane(&self, actor: Actor, name: String) -> Result<Lane> {
        actor.may_change()?;
        let by_admin = operator_name(actor, "retire a lane")?;
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        self.db()
            .write(move |s, _| {
                let lane = s
                    .lane(&name)?
                    .ok_or_else(|| RuleError::lane_not_found(&name))?;
                if lane.retired.is_none() {
                    s.retire_lane(
                        &name,
                        &LaneRetiredRecord {
                            at,
                            run_id,
                            by_admin,
                        },
                    )?;
                }
                s.lane(&name)?
                    .map(Lane::from)
                    .ok_or_else(|| RuleError::lane_not_found(&name))
            })
            .await
    }

    /// One page of the lanes an agent may take, which are those not
    /// retired, in name order: at most `limit`, after the lane named by
    /// `cursor` if given. Anyone may ask, without a token.
    pub async fn takeable_lanes_page(
        &self,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<Lane>> {
        if let Some(c) = &cursor {
            names::check_lane_name(c, MAX_LANE_NAME_CHARS)
                .map_err(|_| RuleError::invalid("cursor is not one list_lanes gave"))?;
        }
        self.db()
            .read(move |s, _| {
                let mut items: Vec<Lane> = s
                    .takeable_lanes_after(cursor.as_deref(), limit + 1)?
                    .into_iter()
                    .map(Lane::from)
                    .collect();
                let more = items.len() > limit;
                items.truncate(limit);
                let next_cursor = match (more, items.last()) {
                    (true, Some(last)) => Some(last.name.clone()),
                    _ => None,
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// The lane `me` holds, with the instructions of the version it took,
    /// or `None` if it holds none.
    pub async fn held_lane(&self, me: &Agent) -> Result<Option<HeldLane>> {
        let handle = me.handle.clone();
        self.db()
            .read(move |s, swarm| held_lane(s, swarm, &handle))
            .await
    }

    /// `me` takes the lane named `lane`: a lane other than the one it holds
    /// switches to it, the one it holds moves to its newest version, and
    /// `None` drops the lane it holds. A change is announced by `me` in
    /// every channel it is in. An unknown or retired lane is refused and
    /// changes nothing. An agent changes only its own lane.
    pub async fn take_lane(&self, me: &Agent, lane: Option<String>) -> Result<LaneTaking> {
        if let Some(name) = &lane {
            names::check_lane_name(name, MAX_LANE_NAME_CHARS).map_err(RuleError::invalid)?;
        }
        let rules = self.clone();
        let handle = me.handle.clone();
        let at = self.now_text();
        let (taking, notices) = self
            .db()
            .write(move |s, swarm| {
                rules.take_lane_in(s, swarm, &handle, lane.as_deref(), &at, "Nothing changed.")
            })
            .await?;
        for notice in &notices {
            self.publish_message(notice);
        }
        Ok(taking)
    }

    /// [`Rules::take_lane`]'s change, inside a write the caller holds.
    /// Returns what `handle` holds now and the messages it posted, for the
    /// caller to publish once the write is done. A refusal ends with
    /// `nothing`, which says what was left undone.
    pub(super) fn take_lane_in(
        &self,
        store: &Writer<'_>,
        swarm: &SwarmId,
        handle: &str,
        wanted: Option<&str>,
        at: &str,
        nothing: &str,
    ) -> Result<(LaneTaking, Vec<Message>)> {
        let before = store
            .agent(swarm, handle)?
            .ok_or_else(|| RuleError::agent_not_found(handle))?
            .lane
            .map(AgentLane::from);
        let target = wanted
            .map(|name| takeable_lane(store, name, nothing))
            .transpose()?;
        let change = match (&before, &target) {
            (None, None) => LaneChange::Unchanged,
            (None, Some(_)) => LaneChange::Took,
            (Some(b), None) => LaneChange::Dropped {
                from: b.name.clone(),
            },
            (Some(b), Some(t)) if b.name != t.name => LaneChange::Switched {
                from: b.name.clone(),
            },
            (Some(b), Some(t)) if b.version != t.current.version => {
                LaneChange::Updated { from: b.version }
            }
            (Some(_), Some(_)) => LaneChange::Unchanged,
        };
        let mut notices = Vec::new();
        if change != LaneChange::Unchanged {
            let taken = target.as_ref().map(|t| TakenLaneRecord {
                lane: t.name.clone(),
                version: t.current.version,
                taken_at: at.to_string(),
                run_id: self.inner.deps.run_id,
            });
            store.set_agent_lane(swarm, handle, taken.as_ref())?;
            let body = lane_announcement(handle, &change, taken.as_ref());
            for id in store.all_channel_ids(swarm, Some(handle))? {
                let channel = member_channel(store, swarm, handle, &id)?;
                notices.push(self.post_notice(store, swarm, &channel, handle, body.clone())?);
            }
        }
        let held = held_lane(store, swarm, handle)?;
        Ok((LaneTaking { held, change }, notices))
    }
}

/// The lane named `name`, if an agent may take it: it exists and is not
/// retired. A refusal is `lane_unavailable` and ends with `nothing`, which
/// says what was left undone.
pub(super) fn takeable_lane(store: &Reader<'_>, name: &str, nothing: &str) -> Result<LaneRecord> {
    match store.lane(name)? {
        None => Err(RuleError::new(
            Code::LaneUnavailable,
            format!("no lane named {name}: `list_lanes` shows the lanes you can take. {nothing}"),
        )),
        Some(lane) if lane.retired.is_some() => Err(RuleError::new(
            Code::LaneUnavailable,
            format!("the lane {name} is retired, so it can no longer be taken. {nothing}"),
        )),
        Some(lane) => Ok(lane),
    }
}

/// The lane `handle` holds, with the instructions of the version it took.
pub(super) fn held_lane(
    store: &Reader<'_>,
    swarm: &SwarmId,
    handle: &str,
) -> Result<Option<HeldLane>> {
    let Some(lane) = store
        .agent(swarm, handle)?
        .ok_or_else(|| RuleError::agent_not_found(handle))?
        .lane
        .map(AgentLane::from)
    else {
        return Ok(None);
    };
    let version = store
        .lane_version(&lane.name, lane.version)?
        .ok_or_else(|| {
            RuleError::new(
                Code::Internal,
                format!(
                    "version {} of the lane {} vanished",
                    lane.version, lane.name
                ),
            )
        })?;
    Ok(Some(HeldLane {
        lane,
        description: version.description,
        text: version.text,
    }))
}

/// The lane `handle` holds, if a newer version of it exists than the one
/// it took.
pub(super) fn newer_lane(
    store: &Reader<'_>,
    swarm: &SwarmId,
    handle: &str,
) -> Result<Option<AgentLane>> {
    Ok(store
        .agent(swarm, handle)?
        .ok_or_else(|| RuleError::agent_not_found(handle))?
        .lane
        .map(AgentLane::from)
        .filter(AgentLane::out_of_date))
}

/// What `handle` says in its channels about a change to its lane; `taken`
/// is what it holds after the change.
fn lane_announcement(handle: &str, change: &LaneChange, taken: Option<&TakenLaneRecord>) -> String {
    let now = taken.map_or(String::new(), |t| {
        format!("the {} lane (version {})", t.lane, t.version)
    });
    match change {
        LaneChange::Took => format!("{handle} took {now}."),
        LaneChange::Switched { from } => {
            format!("{handle} switched from the {from} lane to {now}.")
        }
        LaneChange::Updated { from } => {
            format!("{handle} moved from version {from} to {now}.")
        }
        LaneChange::Dropped { from } => format!("{handle} dropped the {from} lane."),
        LaneChange::Unchanged => format!("{handle} kept {now}."),
    }
}
