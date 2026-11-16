//! kkt-agent-client over MCP, through rmcp and reqwest: `Mcp` implements its
//! `Servers`, and `Client`, `Remote` and `AdminRemote` its `Avatars` and
//! `Watched`. A client registers and reclaims avatars and watches for
//! messages. It connects on first use and again after a call fails to reach
//! the server. Also plain HTTP requests for a program's test client (`Http`),
//! and what the watch asks for at a time (`Settings`), as plain values.

mod settings;
pub use settings::Settings;

mod client;
pub use client::{AdminRemote, Client, Http, HttpReply, Mcp, Remote};
