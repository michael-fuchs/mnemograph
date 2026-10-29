//! The owner's read-only web UI: browse pages, stubs and notes with their neighborhood graphs,
//! the whole wiki as a graph, search, page histories, and statistics.
//! Open views update themselves when agents write (Datastar over server-sent events).

mod chart;
mod live;
pub mod login;
pub mod render;

use std::collections::HashSet;
use std::sync::Arc;

use askama::Template;
use askama_web::WebTemplate;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Router, middleware};
use mnemograph_core::{
    Author, Found, Knowledge, MAX_HOPS, Neighborhood, Note, Session, WikiGraph, title_key,
};
use serde::Deserialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use login::{LoginSettings, PendingLogins};
use render::{page_url, render};

/// How many recent notes the start page and a page show.
const RECENT_NOTES: u32 = 20;
/// How many hits the search page shows.
const SEARCH_HITS: usize = 50;
/// How many pages and stubs a neighborhood graph shows at most, nearest first.
const GRAPH_NODES: u32 = 150;
/// How many pages and stubs the whole-wiki graph shows at most unless configured otherwise.
pub const DEFAULT_GRAPH_NODES: usize = 300;

/// What the web handlers share.
pub struct WebState {
    pub knowledge: Arc<Knowledge>,
    /// `None`: no login, everyone is the owner (development on loopback only).
    pub login: Option<LoginSettings>,
    pending_logins: PendingLogins,
    /// The most pages and stubs the whole-wiki graph shows.
    graph_max_nodes: usize,
    /// Changes whenever agents write.
    changes: watch::Receiver<u64>,
    /// Ends the live update streams when the server shuts down.
    shutdown: CancellationToken,
}

/// The web UI's routes.
pub fn router(
    knowledge: Arc<Knowledge>,
    login: Option<LoginSettings>,
    graph_max_nodes: usize,
    changes: watch::Receiver<u64>,
    shutdown: CancellationToken,
) -> Router {
    let state = Arc::new(WebState {
        knowledge,
        login,
        pending_logins: PendingLogins::default(),
        graph_max_nodes,
        changes,
        shutdown,
    });
    let owner_only = Router::new()
        .route("/", get(index))
        .route("/search", get(search))
        .route("/graph", get(graph))
        .route("/stats", get(stats))
        .route("/pages/{title}", get(page))
        .route("/pages/{title}/history", get(history))
        .route("/pages/{title}/revisions/{number}", get(revision))
        .route("/live", get(live::live))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            login::require_owner,
        ));
    Router::new()
        .merge(owner_only)
        .route("/login", get(login::login))
        .route("/auth/callback", get(login::callback))
        .route("/logout", post(login::logout))
        .route("/logged-out", get(logged_out))
        .route("/assets/style.css", get(style))
        .route("/assets/mermaid.min.js", get(mermaid))
        .route("/assets/datastar.js", get(datastar))
        .route("/assets/cytoscape.min.js", get(cytoscape))
        .route("/assets/graph.js", get(graph_script))
        .with_state(state)
}

/// What every page's header shows.
struct Header {
    /// The logged-in owner, if login is enabled.
    owner_name: Option<String>,
    /// The search query, to keep it in the search box.
    query: String,
    /// The view's path, if it updates itself when agents write.
    live: Option<String>,
}

impl Header {
    fn new(state: &WebState, session: &Session) -> Self {
        Self {
            owner_name: state.login.as_ref().map(|_| session.owner_name.clone()),
            query: String::new(),
            live: None,
        }
    }

    /// A header without a session, for the login flow's pages.
    fn anonymous() -> Self {
        Self {
            owner_name: None,
            query: String::new(),
            live: None,
        }
    }

    fn live(mut self, view: String) -> Self {
        self.live = Some(view);
        self
    }

    /// The live update request for Datastar's `data-init`.
    fn live_request(&self) -> String {
        let view = self.live.as_deref().unwrap_or("/");
        format!(
            "@get('/live?view={}')",
            percent_encoding::utf8_percent_encode(view, percent_encoding::NON_ALPHANUMERIC)
        )
    }
}

struct PageRow {
    title: String,
    url: String,
    page_type: Option<String>,
    updated_at: String,
    updated_by: String,
}

struct LinkItem {
    title: String,
    url: String,
    stub: bool,
}

/// A graph for the `mnemo-graph` element: the data as JSON, and a legend of the page types
/// that occur in it.
struct GraphView {
    json: String,
    legend: Vec<LegendItem>,
    /// Number of pages and stubs in the graph.
    nodes: usize,
    /// Whether any of them is a stub.
    stubs: bool,
}

struct LegendItem {
    page_type: String,
    /// Index of the type's color (`--type-N` in the stylesheet).
    color: usize,
}

/// Colors defined for page types in the stylesheet; further types reuse them.
const TYPE_COLORS: usize = 8;

impl GraphView {
    /// A page's or stub's neighborhood; nodes carry their distance from it.
    fn neighborhood(graph: &Neighborhood, page_types: &[String]) -> Self {
        Self::build(
            graph.nodes.iter().map(|node| {
                let fields = serde_json::json!({ "distance": node.distance });
                (&node.title, node.page_type.as_deref(), node.stub, fields)
            }),
            &graph.edges,
            page_types,
        )
    }

    /// The whole wiki; nodes carry how many others they are linked with.
    fn wiki(graph: &WikiGraph, page_types: &[String]) -> Self {
        Self::build(
            graph.nodes.iter().map(|node| {
                let fields = serde_json::json!({ "degree": node.degree });
                (&node.title, node.page_type.as_deref(), node.stub, fields)
            }),
            &graph.edges,
            page_types,
        )
    }

    fn empty() -> Self {
        Self {
            json: r#"{"nodes":[],"edges":[]}"#.to_owned(),
            legend: Vec::new(),
            nodes: 0,
            stubs: false,
        }
    }

    /// Nodes as (title, page type, stub, further fields).
    fn build<'a>(
        nodes: impl Iterator<Item = (&'a String, Option<&'a str>, bool, serde_json::Value)>,
        edges: &[(usize, usize)],
        page_types: &[String],
    ) -> Self {
        let color = |page_type: &str| {
            page_types
                .iter()
                .position(|t| t == page_type)
                .map(|i| i % TYPE_COLORS)
        };
        let mut types = std::collections::BTreeSet::new();
        let mut stubs = false;
        let nodes: Vec<_> = nodes
            .map(|(title, page_type, stub, mut fields)| {
                types.extend(page_type);
                stubs |= stub;
                fields["label"] = title.as_str().into();
                fields["url"] = page_url(title).into();
                fields["stub"] = stub.into();
                fields["color"] = page_type.and_then(color).into();
                fields
            })
            .collect();
        let mut legend: Vec<_> = types
            .into_iter()
            .filter_map(|page_type| {
                Some(LegendItem {
                    color: color(page_type)?,
                    page_type: page_type.to_owned(),
                })
            })
            .collect();
        legend.sort_by_key(|item| item.color);
        Self {
            json: serde_json::json!({ "nodes": nodes, "edges": edges }).to_string(),
            legend,
            nodes: nodes.len(),
            stubs,
        }
    }
}

struct NoteItem {
    id: i64,
    created_at: String,
    author: String,
    html: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
struct IndexTemplate {
    header: Header,
    pages: Vec<PageRow>,
    notes: Vec<NoteItem>,
    has_mermaid: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "page.html")]
struct PageTemplate {
    header: Header,
    title: String,
    url: String,
    page_type: Option<String>,
    aliases: Vec<String>,
    revision: u32,
    created_at: String,
    updated_at: String,
    updated_by: String,
    body_html: String,
    links: Vec<LinkItem>,
    backlinks: Vec<LinkItem>,
    notes: Vec<NoteItem>,
    graph: GraphView,
    has_mermaid: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "stub.html")]
struct StubTemplate {
    header: Header,
    title: String,
    backlinks: Vec<LinkItem>,
    notes: Vec<NoteItem>,
    graph: GraphView,
    has_mermaid: bool,
}

struct HitItem {
    /// "page" or "note".
    kind: &'static str,
    title: String,
    url: String,
    page_type: Option<String>,
    meta: String,
    snippet_html: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "search.html")]
struct SearchTemplate {
    header: Header,
    page_type: String,
    page_types: Vec<String>,
    searched: bool,
    hits: Vec<HitItem>,
    error: Option<String>,
    has_mermaid: bool,
}

struct RevisionRow {
    number: u32,
    url: String,
    title: String,
    page_type: Option<String>,
    created_at: String,
    author: String,
    bytes: usize,
}

#[derive(Template, WebTemplate)]
#[template(path = "history.html")]
struct HistoryTemplate {
    header: Header,
    title: String,
    url: String,
    rows: Vec<RevisionRow>,
    has_mermaid: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "revision.html")]
struct RevisionTemplate {
    header: Header,
    /// The page's current title.
    title: String,
    url: String,
    number: u32,
    latest: u32,
    revision_title: String,
    page_type: Option<String>,
    aliases: Vec<String>,
    created_at: String,
    author: String,
    body_html: String,
    has_mermaid: bool,
}

/// How many most-linked pages the dashboard lists.
const MOST_LINKED: usize = 10;

struct LinkedPage {
    title: String,
    url: String,
    links: u32,
}

#[derive(Template, WebTemplate)]
#[template(path = "stats.html")]
struct StatsTemplate {
    header: Header,
    totals: mnemograph_core::Totals,
    growth_html: Option<String>,
    most_linked: Vec<LinkedPage>,
    orphans: Vec<LinkItem>,
    has_mermaid: bool,
}

struct TypeChoice {
    page_type: String,
    color: usize,
    checked: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "graph.html")]
struct GraphTemplate {
    header: Header,
    types: Vec<TypeChoice>,
    filtered: bool,
    graph: GraphView,
    hidden: usize,
    has_mermaid: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "message.html")]
struct MessageTemplate {
    header: Header,
    title: String,
    message: String,
    login_link: bool,
    has_mermaid: bool,
}

async fn index(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
) -> Response {
    let knowledge = state.knowledge.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let pages = knowledge.pages()?;
        let mut notes = knowledge.notes()?;
        notes.reverse();
        notes.truncate(RECENT_NOTES as usize);
        let stubs = stub_keys_of_notes(&notes);
        mnemograph_core::Result::Ok((pages, notes, stubs))
    })
    .await;
    let Ok(Ok((pages, notes, stubs))) = loaded else {
        return internal_error(&state, &session);
    };
    let (notes, has_mermaid) = note_items(notes, &stubs);
    IndexTemplate {
        header: Header::new(&state, &session).live("/".to_owned()),
        pages: pages
            .into_iter()
            .map(|page| PageRow {
                url: page_url(&page.title),
                updated_by: author_label(page.author.as_ref()),
                updated_at: timestamp(&page.updated_at),
                page_type: page.page_type,
                title: page.title,
            })
            .collect(),
        notes,
        has_mermaid,
    }
    .into_response()
}

async fn page(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
    Path(title): Path<String>,
) -> Response {
    let knowledge = state.knowledge.clone();
    let requested = title.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let graph = match knowledge.neighborhood(&requested, MAX_HOPS, GRAPH_NODES) {
            Ok(graph) => Some(graph),
            Err(mnemograph_core::Error::PageNotFound(_)) => None,
            Err(error) => return Err(error),
        };
        Ok::<_, mnemograph_core::Error>((
            knowledge.page(&requested)?,
            knowledge.links(&requested)?,
            knowledge.notes_about(&requested, RECENT_NOTES)?,
            graph,
        ))
    })
    .await;
    let Ok(Ok((page, links, notes, graph))) = loaded else {
        return internal_error(&state, &session);
    };
    let graph = graph
        .map(|graph| GraphView::neighborhood(&graph, state.knowledge.page_types()))
        .unwrap_or_else(GraphView::empty);
    let header = Header::new(&state, &session);
    let backlinks = links
        .backlinks
        .iter()
        .map(|title| LinkItem {
            title: title.clone(),
            url: page_url(title),
            stub: false,
        })
        .collect();
    let mut stubs = stub_keys_of_notes(&notes);

    let Some(page) = page else {
        if links.backlinks.is_empty() && notes.is_empty() {
            return message(
                StatusCode::NOT_FOUND,
                header,
                "Not found",
                &format!("Nothing is known about {title:?} yet."),
            );
        }
        let (notes, has_mermaid) = note_items(notes, &stubs);
        return StubTemplate {
            header: header.live(page_url(title.trim())),
            title: title.trim().to_owned(),
            backlinks,
            notes,
            graph,
            has_mermaid,
        }
        .into_response();
    };
    // A page reached by an alias or another spelling lives under its title.
    if page.title != title {
        return Redirect::permanent(&page_url(&page.title)).into_response();
    }

    stubs.extend(
        links
            .outgoing
            .iter()
            .filter(|link| link.is_stub())
            .map(|link| title_key(&link.title)),
    );
    let body = render(&page.body, &stubs);
    let (notes, notes_have_mermaid) = note_items(notes, &stubs);
    PageTemplate {
        header: header.live(page_url(&page.title)),
        url: page_url(&page.title),
        links: links
            .outgoing
            .iter()
            .map(|link| LinkItem {
                title: link.page.clone().unwrap_or_else(|| link.title.clone()),
                url: page_url(link.page.as_deref().unwrap_or(&link.title)),
                stub: link.is_stub(),
            })
            .collect(),
        backlinks,
        notes,
        graph,
        has_mermaid: body.has_mermaid || notes_have_mermaid,
        body_html: body.html,
        title: page.title,
        page_type: page.page_type,
        aliases: page.aliases,
        revision: page.revision,
        created_at: timestamp(&page.created_at),
        updated_at: timestamp(&page.updated_at),
        updated_by: author_label(page.author.as_ref()),
    }
    .into_response()
}

/// The statistics dashboard.
async fn stats(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
) -> Response {
    let knowledge = state.knowledge.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        Ok::<_, mnemograph_core::Error>((
            knowledge.totals()?,
            knowledge.weekly_growth()?,
            knowledge.most_linked_pages(MOST_LINKED)?,
            knowledge.orphaned_pages()?,
        ))
    })
    .await;
    let Ok(Ok((totals, weeks, most_linked, orphans))) = loaded else {
        return internal_error(&state, &session);
    };
    let growth_html = (!weeks.is_empty()).then(|| {
        let labels: Vec<String> = weeks.iter().map(|w| w.start.clone()).collect();
        chart::line_chart(
            "Pages and notes per week",
            &labels,
            &[
                chart::Series {
                    label: "pages".into(),
                    color: "var(--accent)".into(),
                    values: weeks.iter().map(|w| w.pages).collect(),
                },
                chart::Series {
                    label: "notes".into(),
                    color: "var(--type-2)".into(),
                    values: weeks.iter().map(|w| w.notes).collect(),
                },
            ],
        )
    });
    StatsTemplate {
        header: Header::new(&state, &session).live("/stats".to_owned()),
        totals,
        growth_html,
        most_linked: most_linked
            .into_iter()
            .map(|node| LinkedPage {
                url: page_url(&node.title),
                title: node.title,
                links: node.degree,
            })
            .collect(),
        orphans: orphans
            .into_iter()
            .map(|node| LinkItem {
                url: page_url(&node.title),
                title: node.title,
                stub: false,
            })
            .collect(),
        has_mermaid: false,
    }
    .into_response()
}

/// The whole wiki as a graph, optionally only pages of some types (`?type=a&type=b`).
async fn graph(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
    RawQuery(query): RawQuery,
) -> Response {
    let page_types = state.knowledge.page_types().to_vec();
    let chosen = chosen_types(query.as_deref().unwrap_or(""), &page_types);
    let knowledge = state.knowledge.clone();
    let max_nodes = state.graph_max_nodes;
    let filter = chosen.clone();
    let loaded =
        tokio::task::spawn_blocking(move || knowledge.wiki_graph(&filter, max_nodes)).await;
    let Ok(Ok(graph)) = loaded else {
        return internal_error(&state, &session);
    };
    let view = match graph_query(&chosen).as_str() {
        "" => "/graph".to_owned(),
        query => format!("/graph?{query}"),
    };
    GraphTemplate {
        header: Header::new(&state, &session).live(view),
        types: page_types
            .iter()
            .enumerate()
            .map(|(i, page_type)| TypeChoice {
                checked: chosen.contains(page_type),
                page_type: page_type.clone(),
                color: i % TYPE_COLORS,
            })
            .collect(),
        filtered: !chosen.is_empty(),
        hidden: graph.hidden,
        graph: GraphView::wiki(&graph, &page_types),
        has_mermaid: false,
    }
    .into_response()
}

/// The known page types among a query's `type` parameters, in the configured order.
fn chosen_types(query: &str, page_types: &[String]) -> Vec<String> {
    let given: HashSet<String> = query
        .split('&')
        .filter_map(|pair| pair.strip_prefix("type="))
        .filter_map(|value| {
            percent_encoding::percent_decode_str(&value.replace('+', " "))
                .decode_utf8()
                .ok()
                .map(|t| t.trim().to_lowercase())
        })
        .collect();
    page_types
        .iter()
        .filter(|t| given.contains(*t))
        .cloned()
        .collect()
}

/// The query string that chooses these page types.
fn graph_query(page_types: &[String]) -> String {
    page_types
        .iter()
        .map(|t| {
            format!(
                "type={}",
                percent_encoding::utf8_percent_encode(t, percent_encoding::NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default, rename = "type")]
    page_type: String,
}

async fn search(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let mut header = Header::new(&state, &session);
    header.query = query.q.trim().to_owned();
    let page_type = query.page_type.trim().to_owned();
    let mut template = SearchTemplate {
        header,
        page_type: page_type.clone(),
        page_types: state.knowledge.page_types().to_vec(),
        searched: !query.q.trim().is_empty(),
        hits: Vec::new(),
        error: None,
        has_mermaid: false,
    };
    if !template.searched {
        return template.into_response();
    }
    let knowledge = state.knowledge.clone();
    let text = query.q.clone();
    let found = tokio::task::spawn_blocking(move || {
        let page_type = Some(page_type).filter(|t| !t.is_empty());
        knowledge.search(&text, page_type.as_deref(), SEARCH_HITS)
    })
    .await;
    match found {
        Ok(Ok(hits)) => {
            template.hits = hits
                .into_iter()
                .map(|hit| {
                    let snippet_html = snippet_html(&hit.snippet);
                    match hit.found {
                        Found::Page { title, page_type } => HitItem {
                            kind: "page",
                            url: page_url(&title),
                            title,
                            page_type,
                            meta: String::new(),
                            snippet_html,
                        },
                        Found::Note(note) => HitItem {
                            kind: "note",
                            url: format!("/#note-{}", note.id),
                            title: format!("Note #{}", note.id),
                            page_type: None,
                            meta: format!(
                                "{} · {}",
                                timestamp(&note.created_at),
                                author_label(note.author.as_ref())
                            ),
                            snippet_html,
                        },
                    }
                })
                .collect();
        }
        Ok(Err(err)) => template.error = Some(err.to_string()),
        Err(_) => return internal_error(&state, &session),
    }
    template.into_response()
}

async fn history(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
    Path(title): Path<String>,
) -> Response {
    let knowledge = state.knowledge.clone();
    let requested = title.clone();
    let loaded = tokio::task::spawn_blocking(move || knowledge.revisions(&requested)).await;
    let Ok(Ok(revisions)) = loaded else {
        return internal_error(&state, &session);
    };
    let header = Header::new(&state, &session);
    let Some(revisions) = revisions.filter(|r| !r.is_empty()) else {
        return message(
            StatusCode::NOT_FOUND,
            header,
            "Not found",
            &format!("There is no page titled {title:?}."),
        );
    };
    let current = revisions[0].title.clone();
    if current != title {
        return Redirect::permanent(&format!("{}/history", page_url(&current))).into_response();
    }
    let url = page_url(&current);
    HistoryTemplate {
        header: header.live(format!("{url}/history")),
        rows: revisions
            .into_iter()
            .map(|r| RevisionRow {
                url: format!("{url}/revisions/{}", r.number),
                number: r.number,
                title: r.title,
                page_type: r.page_type,
                created_at: timestamp(&r.created_at),
                author: author_label(r.author.as_ref()),
                bytes: r.body_bytes,
            })
            .collect(),
        title: current,
        url,
        has_mermaid: false,
    }
    .into_response()
}

async fn revision(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
    Path((title, number)): Path<(String, u32)>,
) -> Response {
    let knowledge = state.knowledge.clone();
    let requested = title.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        Ok::<_, mnemograph_core::Error>((
            knowledge.page(&requested)?,
            knowledge.page_revision(&requested, number)?,
        ))
    })
    .await;
    let Ok(Ok((current, revision))) = loaded else {
        return internal_error(&state, &session);
    };
    let header = Header::new(&state, &session);
    let (Some(current), Some(revision)) = (current, revision) else {
        return message(
            StatusCode::NOT_FOUND,
            header,
            "Not found",
            &format!("{title:?} has no revision {number}."),
        );
    };
    let url = page_url(&current.title);
    if current.title != title {
        return Redirect::permanent(&format!("{url}/revisions/{number}")).into_response();
    }
    let body = render(&revision.body, &HashSet::new());
    RevisionTemplate {
        header,
        title: current.title,
        url,
        number,
        latest: current.revision,
        revision_title: revision.title,
        page_type: revision.page_type,
        aliases: revision.aliases,
        created_at: timestamp(&revision.updated_at),
        author: author_label(revision.author.as_ref()),
        body_html: body.html,
        has_mermaid: body.has_mermaid,
    }
    .into_response()
}

/// Search snippets mark matches with `**`; everything else is plain text.
fn snippet_html(snippet: &str) -> String {
    let escaped = snippet
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let mut html = String::new();
    for (i, part) in escaped.split("**").enumerate() {
        if i % 2 == 1 {
            html += "<mark>";
            html += part;
            html += "</mark>";
        } else {
            html += part;
        }
    }
    html
}

async fn logged_out() -> Response {
    message(
        StatusCode::OK,
        Header::anonymous(),
        "Logged out",
        "You are logged out of mnemograph.",
    )
}

async fn style() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../assets/style.css"),
    )
        .into_response()
}

async fn datastar() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=604800"),
        ],
        include_str!("../../assets/datastar-1.0.4.js"),
    )
        .into_response()
}

async fn cytoscape() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CONTENT_ENCODING, "gzip"),
            (header::CACHE_CONTROL, "public, max-age=604800"),
        ],
        include_bytes!("../../assets/cytoscape-3.34.4.min.js.gz").as_slice(),
    )
        .into_response()
}

async fn graph_script() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../assets/graph.js"),
    )
        .into_response()
}

async fn mermaid() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CONTENT_ENCODING, "gzip"),
            (header::CACHE_CONTROL, "public, max-age=604800"),
        ],
        include_bytes!("../../assets/mermaid-12.2.0.min.js.gz").as_slice(),
    )
        .into_response()
}

/// Title keys of stubs the notes link to, so their links can be marked.
fn stub_keys_of_notes(notes: &[Note]) -> HashSet<String> {
    notes
        .iter()
        .flat_map(|note| &note.links)
        .filter(|link| link.is_stub())
        .map(|link| title_key(&link.title))
        .collect()
}

fn note_items(notes: Vec<Note>, stubs: &HashSet<String>) -> (Vec<NoteItem>, bool) {
    let mut has_mermaid = false;
    let items = notes
        .into_iter()
        .map(|note| {
            let rendered = render(&note.body, stubs);
            has_mermaid |= rendered.has_mermaid;
            NoteItem {
                id: note.id,
                created_at: timestamp(&note.created_at),
                author: author_label(note.author.as_ref()),
                html: rendered.html,
            }
        })
        .collect();
    (items, has_mermaid)
}

fn author_label(author: Option<&Author>) -> String {
    match author {
        Some(Author::Agent(client)) => format!("{client} (agent)"),
        Some(Author::Owner(_)) => "owner".to_owned(),
        Some(Author::Anonymous) => "anonymous".to_owned(),
        None => "unknown".to_owned(),
    }
}

/// `2026-10-18T12:05:11.141Z` → `2026-10-18 12:05 UTC`.
fn timestamp(rfc3339: &str) -> String {
    match (rfc3339.get(..10), rfc3339.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time} UTC"),
        _ => rfc3339.to_owned(),
    }
}

fn message(status: StatusCode, header: Header, title: &str, text: &str) -> Response {
    (
        status,
        MessageTemplate {
            header,
            title: title.to_owned(),
            message: text.to_owned(),
            login_link: title == "Logged out",
            has_mermaid: false,
        },
    )
        .into_response()
}

/// An error page for the login flow, shown without a session.
pub fn error_page(status: StatusCode, text: &str) -> Response {
    message(status, Header::anonymous(), "Login failed", text)
}

fn internal_error(state: &WebState, session: &Session) -> Response {
    tracing::error!("loading data for the web UI failed");
    message(
        StatusCode::INTERNAL_SERVER_ERROR,
        Header::new(state, session),
        "Error",
        "Something went wrong while loading this page.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_timestamps() {
        assert_eq!(
            author_label(Some(&Author::Agent("claude-code".into()))),
            "claude-code (agent)"
        );
        assert_eq!(author_label(None), "unknown");
        assert_eq!(
            timestamp("2026-10-18T12:05:11.141Z"),
            "2026-10-18 12:05 UTC"
        );
    }

    #[test]
    fn snippets_are_escaped_and_matches_marked() {
        assert_eq!(
            snippet_html("use **Caddy** <script>x</script> & **TLS**"),
            "use <mark>Caddy</mark> &lt;script&gt;x&lt;/script&gt; &amp; <mark>TLS</mark>"
        );
    }
}
