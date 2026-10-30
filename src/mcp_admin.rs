//! The admin endpoint's MCP server, at `/mcp-admin`: the tools an admin
//! agent sees, separate from the agents' tools in `mcp.rs`. Only callers
//! with the admin key reach it (see `app.rs`). It offers no tools yet.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool_handler, tool_router};

/// What the admin endpoint tells every admin agent that connects.
pub const ADMIN_INSTRUCTIONS: &str = include_str!("../admin-instructions.md");

#[derive(Clone)]
pub struct AdminServer {
    tool_router: ToolRouter<Self>,
}

impl AdminServer {
    pub fn new() -> AdminServer {
        AdminServer {
            tool_router: Self::tool_router(),
        }
    }
}

// Empty until the read tools are added.
#[tool_router(allow_empty)]
impl AdminServer {}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AdminServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("kashkeshet-admin", env!("CARGO_PKG_VERSION"))
                    .with_title("Kashkeshet (kkt) admin"),
            )
            .with_instructions(ADMIN_INSTRUCTIONS)
    }
}
