//! What is installed on this machine, as the program reads it: the setup
//! record, the note of a setup not finished, and a server installed here
//! with no record.

use crate::{Purpose, Record, Unfinished, Unrecorded};

/// The three reads of what is installed on this machine. Each is made only
/// when asked, so a caller that stops at the record reads nothing more.
pub trait InstalledHere {
    /// This machine's setup record, if `kkt setup` wrote one.
    fn record(&self) -> Result<Option<Record>, String>;
    /// The note of a setup not finished, if the install it noted is still
    /// here.
    fn unfinished(&self) -> Result<Option<Unfinished>, String>;
    /// A server kkt installed here, in Docker or as a service, with no
    /// record. An error says how the server it is about runs, and why it
    /// cannot be made out.
    fn unrecorded(&self) -> Result<Option<Unrecorded>, (Purpose, String)>;
}
