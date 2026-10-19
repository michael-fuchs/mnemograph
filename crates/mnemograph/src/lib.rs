//! The mnemograph server: HTTP routes and the MCP endpoint on top of [`mnemograph_core`].

pub mod auth;
mod limits;
mod mcp;

use std::sync::Arc;

use axum::{Json, Router, middleware, routing::get};
use mnemograph_core::Knowledge;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

pub use auth::{AuthSettings, Authenticator, Identity};
pub use limits::WriteLimits;
pub use mcp::{
    HitView, LinkView, McpServer, NoteView, PageView, RelatedEntry, RelatedView, SearchView,
};

/// How the HTTP application is set up.
pub struct AppConfig {
    /// Validates agents' access tokens. `None` disables authentication: every request is
    /// anonymous, which is only acceptable on a loopback address during development.
    pub auth: Option<Arc<Authenticator>>,
    /// Host names (optionally with port) under which clients reach the server; requests with
    /// another `Host` header are rejected (DNS rebinding protection).
    pub allowed_hosts: Vec<String>,
    /// Limits on agent writes.
    pub limits: WriteLimits,
    /// Cancelling it stops open MCP streams.
    pub shutdown: CancellationToken,
}

/// Builds the HTTP application: `/health`, the MCP endpoint at `/mcp` and, with
/// authentication, its Protected Resource Metadata.
pub fn app(knowledge: Arc<Knowledge>, config: AppConfig) -> Router {
    let mcp_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_hosts(config.allowed_hosts)
        .with_cancellation_token(config.shutdown);
    let mcp: StreamableHttpService<McpServer, LocalSessionManager> = StreamableHttpService::new(
        {
            let limiter = Arc::new(limits::Limiter::new(config.limits));
            move || Ok(McpServer::new(knowledge.clone(), limiter.clone()))
        },
        Default::default(),
        mcp_config,
    );
    let router = Router::new().route("/health", get(|| async { "ok" }));

    match config.auth {
        None => router.nest_service(
            "/mcp",
            tower::ServiceBuilder::new()
                .layer(middleware::from_fn(auth::anonymous))
                .service(mcp),
        ),
        Some(authenticator) => {
            let metadata = auth::metadata(authenticator.settings());
            let serve_metadata = get(move || {
                let metadata = metadata.clone();
                async move { Json(metadata) }
            });
            router
                .route(auth::METADATA_PATH, serve_metadata.clone())
                // Some clients look for the metadata without the resource path.
                .route("/.well-known/oauth-protected-resource", serve_metadata)
                .nest_service(
                    "/mcp",
                    tower::ServiceBuilder::new()
                        .layer(middleware::from_fn_with_state(
                            authenticator,
                            auth::require_token,
                        ))
                        .service(mcp),
                )
        }
    }
    .layer(middleware::from_fn(log_request))
}

/// Logs every request with its response status (at debug level).
async fn log_request(
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    let response = next.run(request).await;
    tracing::debug!(%method, %path, status = response.status().as_u16(), "request");
    response
}
