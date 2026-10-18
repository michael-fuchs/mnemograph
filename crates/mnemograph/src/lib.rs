//! The mnemograph server: HTTP routes and the MCP endpoint on top of [`mnemograph_core`].

mod mcp;

use std::sync::Arc;

use axum::{Router, routing::get};
use mnemograph_core::Knowledge;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

pub use mcp::{McpServer, PageView};

/// Builds the HTTP application: `/health` and the MCP endpoint at `/mcp`.
///
/// `shutdown` stops open MCP streams when cancelled.
pub fn app(knowledge: Arc<Knowledge>, shutdown: CancellationToken) -> Router {
    let mcp_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_cancellation_token(shutdown);
    let mcp: StreamableHttpService<McpServer, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(McpServer::new(knowledge.clone())),
        Default::default(),
        mcp_config,
    );
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest_service("/mcp", mcp)
}
