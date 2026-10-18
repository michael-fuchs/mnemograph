//! End-to-end tests: an MCP client talks to the server over HTTP.

use std::sync::Arc;

use mnemograph_core::Knowledge;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::{RoleClient, RunningService},
    transport::StreamableHttpClientTransport,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

struct TestServer {
    client: RunningService<RoleClient, ()>,
    shutdown: CancellationToken,
    _dir: tempfile::TempDir,
}

impl TestServer {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let knowledge = Knowledge::open(dir.path().join("test.db")).unwrap();
        let shutdown = CancellationToken::new();
        let app = mnemograph::app(Arc::new(knowledge), shutdown.child_token());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown.cancelled_owned())
                    .await
                    .unwrap();
            }
        });

        let transport = StreamableHttpClientTransport::from_uri(format!("http://{addr}/mcp"));
        let client = ().serve(transport).await.unwrap();
        Self {
            client,
            shutdown,
            _dir: dir,
        }
    }

    async fn call(&self, tool: &'static str, args: Value) -> CallToolResult {
        let args = args.as_object().unwrap().clone();
        self.client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
            .await
            .unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

fn structured(result: &CallToolResult) -> &Value {
    assert_ne!(result.is_error, Some(true), "tool failed: {result:?}");
    result
        .structured_content
        .as_ref()
        .expect("structured content")
}

fn error_text(result: &CallToolResult) -> String {
    assert_eq!(
        result.is_error,
        Some(true),
        "expected a tool error: {result:?}"
    );
    serde_json::to_string(&result.content).unwrap()
}

#[tokio::test]
async fn agent_writes_and_reads_a_page() {
    let server = TestServer::start().await;

    let written = server
        .call(
            "write_page",
            json!({ "title": "Rauthy", "body": "An **OIDC** provider." }),
        )
        .await;
    assert_eq!(structured(&written)["title"], "Rauthy");

    let read = server.call("read_page", json!({ "title": "Rauthy" })).await;
    let page = structured(&read);
    assert_eq!(page["title"], "Rauthy");
    assert_eq!(page["body"], "An **OIDC** provider.");
    assert!(page["created_at"].as_str().unwrap().ends_with('Z'));
}

#[tokio::test]
async fn server_lists_both_tools_and_instructions() {
    let server = TestServer::start().await;

    let tools = server.client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["read_page", "write_page"]);

    let info = server.client.peer_info().expect("server info");
    assert!(
        info.instructions
            .as_deref()
            .unwrap_or("")
            .contains("mnemograph")
    );
}

#[tokio::test]
async fn reading_a_missing_page_is_a_tool_error() {
    let server = TestServer::start().await;
    let result = server
        .call("read_page", json!({ "title": "Nothing" }))
        .await;
    assert!(error_text(&result).contains("no page titled"));
}

#[tokio::test]
async fn writing_an_existing_title_without_base_is_a_conflict() {
    let server = TestServer::start().await;
    server
        .call("write_page", json!({ "title": "Rust", "body": "one" }))
        .await;
    let result = server
        .call("write_page", json!({ "title": "Rust", "body": "two" }))
        .await;
    let error = error_text(&result);
    assert!(error.contains("already exists"), "{error}");
    assert!(
        error.contains("one"),
        "error must contain the current body: {error}"
    );
}

#[tokio::test]
async fn agent_updates_a_page_with_its_base_revision() {
    let server = TestServer::start().await;
    server
        .call("write_page", json!({ "title": "Caddy", "body": "v1" }))
        .await;
    let read = server.call("read_page", json!({ "title": "Caddy" })).await;
    assert_eq!(structured(&read)["revision"], 1);

    let updated = server
        .call(
            "write_page",
            json!({ "title": "Caddy", "body": "v2", "base_revision": 1 }),
        )
        .await;
    assert_eq!(structured(&updated)["revision"], 2);
    let read = server.call("read_page", json!({ "title": "Caddy" })).await;
    assert_eq!(structured(&read)["body"], "v2");
}

#[tokio::test]
async fn stale_base_revision_is_rejected_with_the_current_revision() {
    let server = TestServer::start().await;
    server
        .call("write_page", json!({ "title": "Caddy", "body": "v1" }))
        .await;
    server
        .call(
            "write_page",
            json!({ "title": "Caddy", "body": "v2 by agent A", "base_revision": 1 }),
        )
        .await;

    let result = server
        .call(
            "write_page",
            json!({ "title": "Caddy", "body": "v2 by agent B", "base_revision": 1 }),
        )
        .await;
    let error = error_text(&result);
    assert!(error.contains("current revision is 2"), "{error}");
    assert!(error.contains("v2 by agent A"), "{error}");

    let read = server.call("read_page", json!({ "title": "Caddy" })).await;
    assert_eq!(structured(&read)["body"], "v2 by agent A");
}
