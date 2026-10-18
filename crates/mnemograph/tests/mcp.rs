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
async fn server_lists_its_tools_and_instructions() {
    let server = TestServer::start().await;

    let tools = server.client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["add_note", "read_page", "related", "write_page"]);

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

#[tokio::test]
async fn read_page_shows_links_stubs_and_backlinks() {
    let server = TestServer::start().await;
    server
        .call(
            "write_page",
            json!({ "title": "Rust", "body": "A language." }),
        )
        .await;
    let written = server
        .call(
            "write_page",
            json!({ "title": "mnemograph", "body": "Uses [[Rust]] and [[Tantivy|a search index]]." }),
        )
        .await;
    assert_eq!(
        structured(&written)["links"],
        json!([{ "title": "Rust", "stub": false }, { "title": "Tantivy", "stub": true }])
    );

    let rust = server.call("read_page", json!({ "title": "Rust" })).await;
    assert_eq!(structured(&rust)["backlinks"], json!(["mnemograph"]));
}

#[tokio::test]
async fn reading_a_stub_names_the_linking_pages_until_it_is_written() {
    let server = TestServer::start().await;
    server
        .call(
            "write_page",
            json!({ "title": "Notes", "body": "See [[Caddy]]." }),
        )
        .await;

    let result = server.call("read_page", json!({ "title": "Caddy" })).await;
    let error = error_text(&result);
    assert!(
        error.contains("stub linked from the pages: Notes"),
        "{error}"
    );

    server
        .call(
            "write_page",
            json!({ "title": "Caddy", "body": "A reverse proxy." }),
        )
        .await;
    let caddy = server.call("read_page", json!({ "title": "Caddy" })).await;
    assert_eq!(structured(&caddy)["backlinks"], json!(["Notes"]));
    let notes = server.call("read_page", json!({ "title": "Notes" })).await;
    assert_eq!(
        structured(&notes)["links"],
        json!([{ "title": "Caddy", "stub": false }])
    );
}

#[tokio::test]
async fn pages_are_found_by_alias_and_any_case() {
    let server = TestServer::start().await;
    let written = server
        .call(
            "write_page",
            json!({ "title": "Rust", "body": "A language.", "page_type": "Technology", "aliases": ["rustlang"] }),
        )
        .await;
    assert_eq!(structured(&written)["page_type"], "technology");
    assert_eq!(structured(&written)["aliases"], json!(["rustlang"]));

    let read = server
        .call("read_page", json!({ "title": "RUSTLANG" }))
        .await;
    assert_eq!(structured(&read)["title"], "Rust");
}

#[tokio::test]
async fn rename_keeps_links_working() {
    let server = TestServer::start().await;
    server
        .call("write_page", json!({ "title": "Rauthy", "body": "IdP" }))
        .await;
    server
        .call(
            "write_page",
            json!({ "title": "Setup", "body": "Uses [[Rauthy]]." }),
        )
        .await;

    let renamed = server
        .call(
            "write_page",
            json!({ "title": "Rauthy", "body": "IdP", "base_revision": 1, "new_title": "Rauthy IdP" }),
        )
        .await;
    assert_eq!(structured(&renamed)["title"], "Rauthy IdP");
    assert_eq!(structured(&renamed)["aliases"], json!(["Rauthy"]));
    assert_eq!(structured(&renamed)["backlinks"], json!(["Setup"]));

    let setup = server.call("read_page", json!({ "title": "Setup" })).await;
    assert_eq!(
        structured(&setup)["links"],
        json!([{ "title": "Rauthy", "stub": false, "page": "Rauthy IdP" }])
    );
}

#[tokio::test]
async fn unknown_page_type_lists_the_allowed_types() {
    let server = TestServer::start().await;
    let result = server
        .call(
            "write_page",
            json!({ "title": "serde", "body": "", "page_type": "crate" }),
        )
        .await;
    let error = error_text(&result);
    assert!(
        error.contains("allowed types: project, technology"),
        "{error}"
    );

    let info = server.client.peer_info().expect("server info");
    assert!(
        info.instructions
            .as_deref()
            .unwrap_or("")
            .contains("Allowed page types: project, technology, decision, person, tool, concept")
    );
}

#[tokio::test]
async fn empty_page_type_removes_the_type() {
    let server = TestServer::start().await;
    server
        .call(
            "write_page",
            json!({ "title": "Rust", "body": "", "page_type": "tool" }),
        )
        .await;
    let updated = server
        .call(
            "write_page",
            json!({ "title": "Rust", "body": "", "base_revision": 1, "page_type": "" }),
        )
        .await;
    assert_eq!(structured(&updated).get("page_type"), None);
    assert_eq!(structured(&updated)["revision"], 2);
}

#[tokio::test]
async fn notes_appear_on_the_pages_they_link_to() {
    let server = TestServer::start().await;
    server
        .call(
            "write_page",
            json!({ "title": "Reverse proxy", "body": "" }),
        )
        .await;
    let note = server
        .call(
            "add_note",
            json!({ "body": "Decided [[Reverse proxy|the proxy]] is [[Caddy]]: automatic HTTPS." }),
        )
        .await;
    let note = structured(&note).clone();
    assert_eq!(
        note["links"],
        json!([
            { "title": "Reverse proxy", "stub": false },
            { "title": "Caddy", "stub": true }
        ])
    );

    let page = server
        .call("read_page", json!({ "title": "reverse proxy" }))
        .await;
    assert_eq!(structured(&page)["notes"], json!([note]));
}

#[tokio::test]
async fn reading_a_stub_shows_its_notes() {
    let server = TestServer::start().await;
    server
        .call(
            "add_note",
            json!({ "body": "Evaluate [[Tantivy]] for search." }),
        )
        .await;
    let result = server
        .call("read_page", json!({ "title": "Tantivy" }))
        .await;
    let error = error_text(&result);
    assert!(error.contains("it is a stub"), "{error}");
    assert!(error.contains("Evaluate"), "{error}");
}

#[tokio::test]
async fn empty_note_is_rejected() {
    let server = TestServer::start().await;
    let result = server.call("add_note", json!({ "body": "  " })).await;
    assert!(error_text(&result).contains("must not be empty"));
}

#[tokio::test]
async fn related_returns_the_neighborhood_nearest_first() {
    let server = TestServer::start().await;
    for (title, body) in [
        (
            "mnemograph",
            "Written in [[Rust]], searches with [[Tantivy]].",
        ),
        ("Rust", "See [[Ferris]]."),
        ("Ferris", "The mascot."),
    ] {
        server
            .call("write_page", json!({ "title": title, "body": body }))
            .await;
    }

    let near = server
        .call("related", json!({ "title": "mnemograph", "hops": 1 }))
        .await;
    assert_eq!(
        structured(&near)["related"],
        json!([
            { "title": "Rust", "stub": false, "distance": 1 },
            { "title": "Tantivy", "stub": true, "distance": 1 }
        ])
    );

    let default_hops = server
        .call("related", json!({ "title": "mnemograph" }))
        .await;
    assert_eq!(structured(&default_hops)["related"][2]["title"], "Ferris");
    assert_eq!(structured(&default_hops)["related"][2]["distance"], 2);
}

#[tokio::test]
async fn related_of_an_unknown_title_is_a_tool_error() {
    let server = TestServer::start().await;
    let result = server.call("related", json!({ "title": "Nothing" })).await;
    assert!(error_text(&result).contains("no page titled"));
}
