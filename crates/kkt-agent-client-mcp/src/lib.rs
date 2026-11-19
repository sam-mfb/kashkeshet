//! kkt-agent-client over MCP, through rmcp and reqwest: `Mcp` implements its
//! `Servers`, `Client` its `Avatars`, and `Remote` its `Watched`, with an
//! admin's view of its own. A client registers and reclaims avatars and watches for
//! messages. It connects on first use and again after a call fails to reach
//! the server. Also plain HTTP requests for a program's test client (`Http`),
//! and what the watch asks for at a time (`Settings`), as plain values.

mod settings;
pub use settings::Settings;

mod client;
pub use client::{Client, Http, HttpReply, Mcp, Remote};
