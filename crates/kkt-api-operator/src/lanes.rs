use super::*;

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
