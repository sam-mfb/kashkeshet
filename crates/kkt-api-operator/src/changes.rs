#[derive(Clone, Debug)]
pub enum ChannelChange {
    SetOpen(bool),
    AddMember(String),
    RemoveMember(String),
    /// Makes the named member the owner, or with `None` leaves the channel
    /// with no owner.
    SetOwner(Option<String>),
    Hide,
}
