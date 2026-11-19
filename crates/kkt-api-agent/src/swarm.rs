/// The ID of [`SwarmId::main`], as the core's second migration writes it.
const MAIN: &str = "sw_main";

/// Which swarm an avatar or channel belongs to, and so which swarm a lookup
/// is bound to. An avatar or channel never moves to another swarm.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SwarmId(String);

impl SwarmId {
    /// The swarm with this ID.
    pub fn new(id: impl Into<String>) -> SwarmId {
        SwarmId(id.into())
    }

    /// The swarm the database's upgrade to swarms made, `sw_main`, which
    /// every avatar and channel from before swarms is in. A server in key
    /// mode has this one swarm and no other.
    pub fn main() -> SwarmId {
        SwarmId::new(MAIN)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
