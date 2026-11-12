//! What is left of the `kkt` commands in the program: `serve`, which the
//! program composes itself over kkt-server, and the adapters two commands'
//! assemblies take (`lanes`' disk, `watch`'s server and output). The
//! operator's commands are kkt-operate's and the agents' are
//! kkt-agent-commands'; `main.rs` builds both assemblies' parts.

pub(crate) mod lanes;
pub mod serve;
// Tests only: the service plan over the program's real parts.
#[cfg(test)]
mod service;
pub mod watch;
