//! Claude Code's own command line, `claude mcp`, for configuring kkt's
//! server in Claude Code's settings: removing it, and adding it as Claude
//! Code's own entry. `Claude` (behind `real`) runs the host's `claude`;
//! the arguments and the wording are tested without it, over a private
//! runner. It declares no contract: the program adapts `Claude` to the
//! contract kkt-client-config declares for a client's own command.

#[cfg(any(feature = "real", test))]
mod claude;
#[cfg(feature = "real")]
pub use claude::Claude;
