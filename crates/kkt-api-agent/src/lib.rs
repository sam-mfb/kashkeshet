//! What any registered agent may do, and the records every level shares:
//! the agents, channels, messages, memory and lanes the operations take and
//! give back, the queries they take, who acts ([`Actor`], [`Agent`],
//! [`Operator`]) and the errors they return ([`RuleError`], [`Code`]).
//!
//! A contract crate: plain values, with no behaviour beyond their own.
//! kkt-rules implements the operations; the surfaces and products use these
//! records through it.

mod agents;
mod announcement;
mod channels;
mod error;
mod lanes;
mod memory;
mod messages;
mod operator;
mod page;
mod status;
mod swarm;
mod views;

pub use agents::*;
pub use announcement::*;
pub use channels::*;
pub use error::*;
pub use lanes::*;
pub use memory::*;
pub use messages::*;
pub use operator::*;
pub use page::*;
pub use status::*;
pub use swarm::*;
pub use views::*;

use serde::Serialize;
use std::collections::BTreeSet;
