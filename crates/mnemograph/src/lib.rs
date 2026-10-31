//! The mnemograph server: HTTP routes and the MCP endpoint on top of [`mnemograph_core`].

pub mod auth;
mod limits;
mod mcp;
pub mod spheres;
pub mod web;

use std::sync::Arc;

use axum::{Json, Router, middleware, routing::get};
use mnemograph_core::{Knowledge, Spheres};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

pub use auth::{AuthSettings, Authenticator, Identity};
pub use limits::WriteLimits;
pub use mcp::{
    HitView, LinkView, McpServer, NoteView, PageView, RelatedEntry, RelatedView, SearchView,
};
pub use spheres::SphereConfig;

/// How many days the access log keeps entries unless configured otherwise.
pub const DEFAULT_ACCESS_LOG_DAYS: u32 = 90;
/// How often old access log entries are deleted.
const PRUNE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How the HTTP application is set up.
pub struct AppConfig {
    /// Validates agents' access tokens. `None` disables authentication: every request is
    /// anonymous, which is only acceptable on a loopback address during development.
    pub auth: Option<Arc<Authenticator>>,
    /// The configured spheres. Without authentication, every request may use all of them.
    pub spheres: Spheres,
    /// Host names (optionally with port) under which clients reach the server; requests with
    /// another `Host` header are rejected (DNS rebinding protection).
    pub allowed_hosts: Vec<String>,
    /// Limits on agent writes.
    pub limits: WriteLimits,
    /// The owner's browser login for the web UI. `None` disables it: everyone is the owner,
    /// which is only acceptable on a loopback address during development.
    pub web_login: Option<web::login::LoginSettings>,
    /// The most pages and stubs the whole-wiki graph shows; the most-linked are kept.
    pub graph_max_nodes: usize,
    /// How many days the access log keeps what agents read and searched.
    pub access_log_days: u32,
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
        .with_cancellation_token(config.shutdown.clone());
    tokio::spawn(prune_access_log(
        knowledge.clone(),
        config.access_log_days,
        config.shutdown.clone(),
    ));
    let changes = Arc::new(tokio::sync::watch::Sender::new(0u64));
    let web = web::router(
        knowledge.clone(),
        config.web_login,
        config.graph_max_nodes,
        config.access_log_days,
        changes.subscribe(),
        config.shutdown.clone(),
    );
    let mcp: StreamableHttpService<McpServer, LocalSessionManager> = StreamableHttpService::new(
        {
            let limiter = Arc::new(limits::Limiter::new(config.limits));
            move || {
                Ok(McpServer::new(
                    knowledge.clone(),
                    limiter.clone(),
                    changes.clone(),
                ))
            }
        },
        Default::default(),
        mcp_config,
    );
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(web);

    match config.auth {
        None => router.nest_service(
            "/mcp",
            tower::ServiceBuilder::new()
                .layer(middleware::from_fn_with_state(
                    config.spheres,
                    auth::anonymous,
                ))
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

/// Deletes access log entries older than `days` days now and then every [`PRUNE_INTERVAL`],
/// until shutdown.
async fn prune_access_log(knowledge: Arc<Knowledge>, days: u32, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(PRUNE_INTERVAL);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = interval.tick() => {}
        }
        let knowledge = knowledge.clone();
        match tokio::task::spawn_blocking(move || knowledge.prune_access_log(days)).await {
            Ok(Ok(0)) => {}
            Ok(Ok(deleted)) => tracing::info!(deleted, days, "pruned the access log"),
            Ok(Err(error)) => tracing::warn!(%error, "pruning the access log failed"),
            Err(error) => tracing::warn!(%error, "pruning the access log failed"),
        }
    }
}
