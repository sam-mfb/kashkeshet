//! Claude Code's own command line, `claude mcp`, for configuring one MCP
//! server, named by the caller, in Claude Code's settings: removing it, and
//! adding it as Claude Code's own entry. `Claude` runs the host's `claude`;
//! the arguments and the wording are tested without it, over a private
//! runner. A caller adapts `Claude` to its own contract.

mod claude;
pub use claude::{Claude, ClaudeError};
