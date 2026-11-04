//! How the MCP endpoints are served over HTTP: rmcp's streamable HTTP
//! service, with every transport setting chosen here, and the cancellation
//! token that ends its sessions when the server stops.
//!
//! The agent endpoint (`mcp.rs`) and the admin endpoint (`mcp_admin.rs`)
//! each mount their server through one shared [`Transport`]. rmcp's and
//! tokio-util's types stay inside the MCP parts: the server gets back only
//! axum routers.

use crate::config::{MCP_MAX_REQUEST_BYTES, MCP_SSE_KEEP_ALIVE, MCP_SSE_RETRY};
use crate::hosts::AllowedHosts;
use crate::shutdown::Shutdown;
use axum::Router;
use rmcp::ServerHandler;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The transport both MCP endpoints share: which browser origins and hosts
/// they accept, and one cancellation token for rmcp that is cancelled when
/// the server's [`Shutdown`] stops.
pub struct Transport {
    allowed_origins: Vec<String>,
    allowed_hosts: Vec<String>,
    stopped: CancellationToken,
}

impl Transport {
    /// A transport that accepts `allowed_origins` and `allowed_hosts`, and
    /// whose sessions end when `shutdown` stops.
    ///
    /// Must be called inside the tokio runtime: it spawns one task that
    /// waits for `shutdown` and then cancels rmcp's token, so sessions learn
    /// of the stop one scheduler turn after it.
    pub fn new(
        allowed_origins: Vec<String>,
        allowed_hosts: &AllowedHosts,
        shutdown: &Shutdown,
    ) -> Transport {
        let stopped = CancellationToken::new();
        let cancel = stopped.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            shutdown.stopped().await;
            cancel.cancel();
        });
        Transport {
            allowed_origins,
            allowed_hosts: allowed_hosts.entries(),
            stopped,
        }
    }

    /// A router serving `server` at `path`. Each request gets a clone of
    /// `server`.
    pub(crate) fn serve<S>(&self, path: &str, server: S) -> Router
    where
        S: ServerHandler + Clone,
    {
        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            // A session manager with no event store: there are no sessions,
            // and a broken event stream is not resumed. That is the only
            // state `default()` can build, so there is nothing further to set.
            Arc::new(NeverSessionManager::default()),
            self.config(),
        );
        Router::new().nest_service(path, service)
    }

    /// The MCP transport settings. Every option is set here by name, so a
    /// change to the library's defaults cannot change the server's behavior.
    fn config(&self) -> StreamableHttpServerConfig {
        let mut config = StreamableHttpServerConfig::default()
            // Serve every request on its own, with no session to resume.
            .with_legacy_session_mode(false)
            // Answer with event streams, never with plain JSON bodies.
            .with_json_response(false)
            .with_sse_keep_alive(Some(MCP_SSE_KEEP_ALIVE))
            .with_sse_retry(Some(MCP_SSE_RETRY))
            .with_allowed_hosts(self.allowed_hosts.clone())
            // Refuse any browser origin not listed, even when the list is empty.
            .with_allowed_origins(self.allowed_origins.clone())
            .enforce_origin_validation()
            .with_max_request_body_bytes(MCP_MAX_REQUEST_BYTES)
            // Older clients do not send per-request protocol metadata; accept them.
            .with_stateless_protocol_metadata_required(false)
            .with_cancellation_token(self.stopped.clone());
        config.session_store = None;
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stopping_the_server_cancels_the_token_given_to_rmcp() {
        let shutdown = Shutdown::new();
        let transport = Transport::new(Vec::new(), &AllowedHosts::any(), &shutdown);
        let token = transport.config().cancellation_token;
        assert!(!token.is_cancelled());
        shutdown.stop();
        token.cancelled().await;
    }
}
