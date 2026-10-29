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
        Self::start_with(mnemograph::web::DEFAULT_GRAPH_NODES).await
    }

    async fn start_with(graph_max_nodes: usize) -> Self {
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
                graph_max_nodes,
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
        json!({ "title": "Rust", "body": "Version **two**, see [[Cargo]].", "base_revision": 1 }),
    )
    .await;

    let received = tokio::time::timeout(Duration::from_secs(5), async {
        let mut text = String::new();
        while !(text.contains("Version <strong>two</strong>") && text.contains("</main>")) {
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
    // The new link reaches the graph with the same patch.
    assert!(
        received.contains("<mnemo-graph graph=") && received.contains("&#34;Cargo&#34;"),
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

/// The data of the page's `mnemo-graph` element, if it has one.
fn graph_data(html: &str) -> Option<Value> {
    let start = html.find("<mnemo-graph graph=\"")? + "<mnemo-graph graph=\"".len();
    let end = start + html[start..].find('"')?;
    let unescaped = html[start..end]
        .replace("&#34;", "\"")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    Some(serde_json::from_str(&unescaped).unwrap())
}

#[tokio::test]
async fn pages_and_stubs_show_their_neighborhood_graph() {
    let ui = Ui::start().await;
    ui.call(
        "write_page",
        json!({ "title": "Rust", "body": "", "aliases": ["rustlang"], "page_type": "technology" }),
    )
    .await;
    ui.call(
        "write_page",
        json!({ "title": "mnemograph", "body": "[[rustlang]] and [[Tantivy]] <b>&</b>", "page_type": "project" }),
    )
    .await;
    ui.call("write_page", json!({ "title": "Alone", "body": "" }))
        .await;

    let (status, html) = ui.html("/pages/Rust").await;
    assert_eq!(status, 200);
    let graph = graph_data(&html).expect("the page has a graph");
    let nodes = graph["nodes"].as_array().unwrap();
    let summary: Vec<_> = nodes
        .iter()
        .map(|n| {
            (
                n["label"].as_str().unwrap(),
                n["distance"].as_u64().unwrap(),
                n["stub"].as_bool().unwrap(),
            )
        })
        .collect();
    // The center first, then nearest first; the link through the alias counts.
    assert_eq!(
        summary,
        [
            ("Rust", 0, false),
            ("mnemograph", 1, false),
            ("Tantivy", 2, true)
        ]
    );
    assert_eq!(nodes[2]["url"], "/pages/Tantivy");
    assert_eq!(graph["edges"], json!([[0, 1], [1, 2]]));
    // Page types are colored by their position in the configured list, with a legend.
    assert_eq!(nodes[0]["color"], 1);
    assert_eq!(nodes[1]["color"], 0);
    assert!(nodes[2]["color"].is_null());
    assert!(
        html.contains("var(--type-0)\"></span> project</span>"),
        "{html}"
    );
    assert!(html.contains("<script src=\"/assets/graph.js\"></script>"));

    let (status, stub) = ui.html("/pages/Tantivy").await;
    assert_eq!(status, 200);
    let graph = graph_data(&stub).expect("a stub has a graph");
    assert_eq!(graph["nodes"][0]["label"], "Tantivy");
    assert_eq!(graph["nodes"][0]["stub"], true);

    let (_, alone) = ui.html("/pages/Alone").await;
    assert!(graph_data(&alone).is_none(), "nothing to draw");

    let script = reqwest::get(format!("{}/assets/cytoscape.min.js", ui.base))
        .await
        .unwrap();
    assert_eq!(script.status(), 200);
    assert_eq!(script.headers()["content-encoding"], "gzip");
}

/// Labels of the graph's nodes, in the order sent.
fn graph_labels(graph: &Value) -> Vec<&str> {
    graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["label"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_wiki_graph_shows_everything_filters_by_type_and_caps() {
    let ui = Ui::start_with(3).await;
    for (title, page_type, body) in [
        (
            "mnemograph",
            "project",
            "[[Rust]] [[SQLite]] [[Caddy]] [[Garage]]",
        ),
        ("Rust", "technology", "[[SQLite]]"),
        ("SQLite", "technology", ""),
        ("Caddy", "tool", ""),
        ("Alone", "concept", ""),
    ] {
        ui.call(
            "write_page",
            json!({ "title": title, "body": body, "page_type": page_type }),
        )
        .await;
    }

    let (status, html) = ui.html("/graph").await;
    assert_eq!(status, 200);
    assert!(
        html.contains("<a href=\"/graph\">Graph</a>"),
        "the header links to it"
    );
    let graph = graph_data(&html).unwrap();
    // Six pages and stubs, of which the three most-linked fit.
    assert_eq!(graph_labels(&graph), ["mnemograph", "Rust", "SQLite"]);
    assert_eq!(graph["nodes"][0]["degree"], 4);
    assert!(
        html.contains("Showing the 3 most-linked pages and stubs; 3 more are hidden."),
        "{html}"
    );
    assert!(html.contains("mode=\"wiki\""));

    let (_, html) = ui.html("/graph?type=tool&type=concept&type=nonsense").await;
    let graph = graph_data(&html).unwrap();
    // Unlinked pages are included; stubs are not, having no type.
    assert_eq!(graph_labels(&graph), ["Alone", "Caddy"]);
    assert_eq!(graph["edges"], json!([]));
    assert!(!html.contains("hidden"), "{html}");
    assert!(
        !html.contains("swatch stub"),
        "no stubs, so none in the legend"
    );
    assert!(html.contains("value=\"tool\" checked"));
    assert!(html.contains("value=\"project\">"));
    assert!(html.contains(
        "data-init=\"@get(&#39;/live?view=%2Fgraph%3Ftype%3Dtool%26type%3Dconcept&#39;)\""
    ));

    let (_, html) = ui.html("/graph?type=person").await;
    assert!(html.contains("No pages of these types yet."));
}

#[tokio::test]
async fn the_wiki_graph_updates_live() {
    let ui = Ui::start().await;
    ui.call("write_page", json!({ "title": "Rust", "body": "" }))
        .await;
    let mut stream = reqwest::get(format!("{}/live?view=%2Fgraph", ui.base))
        .await
        .unwrap();
    ui.call(
        "write_page",
        json!({ "title": "Cargo", "body": "[[Rust]]" }),
    )
    .await;
    let received = tokio::time::timeout(Duration::from_secs(5), async {
        let mut text = String::new();
        while !text.contains("</main>") {
            let chunk = stream.chunk().await.unwrap().expect("stream ended");
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
        text
    })
    .await
    .expect("no update within 5 seconds");
    assert!(received.contains("&#34;Cargo&#34;"), "{received}");
}
