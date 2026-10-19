//! End-to-end tests of the web UI's views (without login, as in local development).

use std::sync::Arc;
use std::time::Duration;

use mnemograph::AppConfig;
use mnemograph_core::Knowledge;
use rmcp::{
    ServiceExt, model::CallToolRequestParams, service::RoleClient, service::RunningService,
    transport::StreamableHttpClientTransport,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

struct Ui {
    base: String,
    agent: RunningService<RoleClient, ()>,
    shutdown: CancellationToken,
    _dir: tempfile::TempDir,
}

impl Ui {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let knowledge = Knowledge::open(dir.path().join("ui.db")).unwrap();
        let shutdown = CancellationToken::new();
        let app = mnemograph::app(
            Arc::new(knowledge),
            AppConfig {
                auth: None,
                allowed_hosts: vec!["127.0.0.1".into()],
                limits: Default::default(),
                web_login: None,
                shutdown: shutdown.child_token(),
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown.cancelled_owned())
                    .await
                    .unwrap();
            }
        });
        let agent = ()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "{base}/mcp"
            )))
            .await
            .unwrap();
        Self {
            base,
            agent,
            shutdown,
            _dir: dir,
        }
    }

    async fn call(&self, tool: &'static str, args: Value) {
        let result = self
            .agent
            .call_tool(
                CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
    }

    async fn html(&self, path: &str) -> (u16, String) {
        let response = reqwest::get(format!("{}{path}", self.base)).await.unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn search_page_finds_pages_and_notes() {
    let ui = Ui::start().await;
    ui.call(
        "write_page",
        json!({ "title": "Speicher", "body": "Wir vergleichen Datenbanken <script>.", "page_type": "decision" }),
    )
    .await;
    ui.call("add_note", json!({ "body": "Die Datenbank läuft." }))
        .await;

    let (status, html) = ui.html("/search?q=datenbank").await;
    assert_eq!(status, 200);
    assert!(
        html.contains("<a href=\"/pages/Speicher\">Speicher</a>"),
        "{html}"
    );
    assert!(html.contains("<mark>Datenbanken</mark>"), "{html}");
    assert!(
        html.contains("&lt;script&gt;"),
        "snippets are escaped: {html}"
    );
    assert!(html.contains("Note #1"), "{html}");
    assert!(
        html.contains("value=\"datenbank\""),
        "the query stays in the box"
    );

    let (_, only_decisions) = ui.html("/search?q=datenbank&type=decision").await;
    assert!(only_decisions.contains("Speicher") && !only_decisions.contains("Note #1"));
    let (_, nothing) = ui.html("/search?q=zebra").await;
    assert!(nothing.contains("Nothing found."));
}

#[tokio::test]
async fn history_lists_revisions_and_each_can_be_viewed() {
    let ui = Ui::start().await;
    ui.call(
        "write_page",
        json!({ "title": "Proxy", "body": "We use **Traefik**." }),
    )
    .await;
    ui.call(
        "write_page",
        json!({ "title": "Proxy", "body": "We use **Caddy**.", "base_revision": 1, "new_title": "Reverse proxy" }),
    )
    .await;

    let (status, history) = ui.html("/pages/Reverse%20proxy/history").await;
    assert_eq!(status, 200);
    assert!(
        history.contains(
            "href=\"/pages/Reverse%20proxy/revisions/2\">2</a> <span class=\"type\">current</span>"
        ),
        "{history}"
    );
    assert!(
        history.contains("<td>Proxy</td>"),
        "old title in revision 1: {history}"
    );
    assert!(history.contains("anonymous"), "{history}");

    let (status, old) = ui.html("/pages/Reverse%20proxy/revisions/1").await;
    assert_eq!(status, 200);
    assert!(old.contains("revision 1 of 2"), "{old}");
    assert!(old.contains("<strong>Traefik</strong>"), "{old}");

    let via_alias = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!("{}/pages/proxy/history", ui.base))
        .send()
        .await
        .unwrap();
    assert_eq!(via_alias.status(), 308);
    assert_eq!(
        via_alias.headers()["location"],
        "/pages/Reverse%20proxy/history"
    );
    assert_eq!(ui.html("/pages/Reverse%20proxy/revisions/9").await.0, 404);
}

#[tokio::test]
async fn open_views_receive_a_patch_when_an_agent_writes() {
    let ui = Ui::start().await;
    ui.call(
        "write_page",
        json!({ "title": "Rust", "body": "Version one." }),
    )
    .await;
    let (_, page) = ui.html("/pages/Rust").await;
    assert!(
        page.contains("data-init=\"@get(&#39;/live?view=%2Fpages%2FRust&#39;)\""),
        "{page}"
    );

    let mut stream = reqwest::get(format!("{}/live?view=%2Fpages%2FRust", ui.base))
        .await
        .unwrap();
    assert_eq!(stream.headers()["content-type"], "text/event-stream");

    ui.call(
        "write_page",
        json!({ "title": "Rust", "body": "Version **two**.", "base_revision": 1 }),
    )
    .await;

    let received = tokio::time::timeout(Duration::from_secs(5), async {
        let mut text = String::new();
        while !text.contains("Version <strong>two</strong>") {
            let chunk = stream.chunk().await.unwrap().expect("stream ended");
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
        text
    })
    .await
    .expect("no update within 5 seconds");
    assert!(
        received.contains("event: datastar-patch-elements"),
        "{received}"
    );
    assert!(
        received.contains("data: elements <main id=\"content\">"),
        "{received}"
    );
}

#[tokio::test]
async fn unknown_live_views_are_rejected() {
    let ui = Ui::start().await;
    let response = reqwest::get(format!("{}/live?view=%2Fsearch", ui.base))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}
