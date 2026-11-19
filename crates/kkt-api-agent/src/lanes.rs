use super::*;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneRetired {
    pub at: String,
    pub run_id: i64,
    /// The admin agent that retired it; `None` when the human operator did.
    pub by_admin: Option<String>,
}

/// The folder a lane version was imported from, and the git commit checked
/// out there, if the folder was in a git checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneSource {
    pub folder: String,
    pub commit: Option<String>,
}
