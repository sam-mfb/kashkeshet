//! Lanes: named instructions an agent can take on. The operator (the human,
//! or a named admin agent) creates and edits them; each save is a new,
//! immutable version recording who saved it and when. A lane is never
//! deleted, only retired. A lane grants no permissions.
use super::*;
use crate::config::MAX_LANE_DESCRIPTION_CHARS;
use crate::store::{LaneRecord, LaneRetiredRecord, LaneVersionRecord, NewLaneVersionRecord};

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
        }
    }
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

/// The admin agent's name when `actor` is the operator, or the refusal for
/// an agent: only the operator edits lanes.
fn operator_name(actor: Actor, what: &str) -> Result<Option<String>> {
    match actor {
        Actor::Human => Ok(None),
        Actor::Admin(name) => Ok(Some(name.to_string())),
        Actor::Agent(_) => Err(RuleError::invalid(format!("only the operator may {what}"))),
    }
}

impl Rules {
    /// Every lane, retired ones included, in name order.
    pub async fn lanes(&self) -> Result<Vec<Lane>> {
        self.db()
            .read(|s| Ok(s.lanes()?.into_iter().map(Lane::from).collect()))
            .await
    }

    /// One lane, retired or not.
    pub async fn lane(&self, name: String) -> Result<Lane> {
        self.db()
            .read(move |s| {
                s.lane(&name)?
                    .map(Lane::from)
                    .ok_or_else(|| RuleError::lane_not_found(&name))
            })
            .await
    }

    /// One lane, retired or not, with every version, newest first.
    pub async fn lane_history(&self, name: String) -> Result<LaneHistory> {
        self.db()
            .read(move |s| {
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
        let saved_by_admin = operator_name(actor, "save a lane")?;
        names::check_lane_name(&save.name).map_err(RuleError::invalid)?;
        let description = save.description.trim().to_string();
        if description.is_empty() {
            return Err(RuleError::invalid("the lane needs a description"));
        }
        if description.contains(['\n', '\r']) {
            return Err(RuleError::invalid(
                "the lane's description must be one line",
            ));
        }
        if description.chars().count() > MAX_LANE_DESCRIPTION_CHARS {
            return Err(RuleError::invalid(format!(
                "the lane's description must be at most {MAX_LANE_DESCRIPTION_CHARS} characters"
            )));
        }
        if save.text.trim().is_empty() {
            return Err(RuleError::invalid("the lane needs instructions"));
        }
        let max = self.limits().max_lane_chars;
        if save.text.chars().count() > max {
            return Err(RuleError::invalid(format!(
                "the lane's instructions must be at most {max} characters"
            )));
        }
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        let name = save.name;
        let text = save.text;
        self.db()
            .write(move |s| {
                match (s.lane(&name)?, must_be) {
                    (Some(_), Some(LaneMustBe::New)) => {
                        return Err(RuleError::invalid(format!(
                            "a lane named {name} already exists"
                        )));
                    }
                    (None, Some(LaneMustBe::Existing)) => {
                        return Err(RuleError::lane_not_found(&name));
                    }
                    (Some(lane), _) if lane.retired.is_some() => {
                        return Err(RuleError::invalid(format!(
                            "the lane {name} is retired, so it cannot be changed"
                        )));
                    }
                    (Some(lane), _)
                        if lane.current.description == description && lane.current.text == text =>
                    {
                        return Ok(Lane::from(lane));
                    }
                    (Some(_), _) => {}
                    (None, _) => s.insert_lane(&name, &at, run_id)?,
                }
                s.append_lane_version(&NewLaneVersionRecord {
                    lane: name.clone(),
                    description,
                    text,
                    saved_at: at,
                    run_id,
                    saved_by_admin,
                })?;
                s.lane(&name)?
                    .map(Lane::from)
                    .ok_or_else(|| RuleError::lane_not_found(&name))
            })
            .await
    }

    /// Retires the lane: it stays, with its history, but can no longer be
    /// taken or changed. Retiring a retired lane changes nothing.
    pub async fn retire_lane(&self, actor: Actor, name: String) -> Result<Lane> {
        let by_admin = operator_name(actor, "retire a lane")?;
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        self.db()
            .write(move |s| {
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
}
