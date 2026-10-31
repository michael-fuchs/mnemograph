//! Live updates: an open view keeps a server-sent-events stream to `/live`. Whenever agents
//! write, the view is rendered again and its `<main>` is sent as a Datastar patch.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, RawQuery, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Extension, body};
use datastar::prelude::{ExecuteScript, PatchElements};
use futures::stream::{self, StreamExt};
use mnemograph_core::Session;
use serde::Deserialize;

use super::WebState;
use super::spheres::SphereFilter;

/// Writes that arrive within this time after another one cause a single update.
const COALESCE: Duration = Duration::from_millis(200);
/// Rendered views are small; this only guards against surprises.
const MAX_VIEW_BYTES: usize = 16 * 1024 * 1024;

/// The views that update themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
enum View {
    Index,
    Page(String),
    History(String),
    /// The whole-wiki graph, with its query string (the chosen page types).
    Graph(String),
    Stats,
}

impl View {
    /// Parses a view path such as `/`, `/pages/Rust` or `/pages/Rust/history`.
    fn parse(path: &str) -> Option<Self> {
        if path == "/" {
            return Some(View::Index);
        }
        if path == "/stats" {
            return Some(View::Stats);
        }
        if path == "/graph" {
            return Some(View::Graph(String::new()));
        }
        if let Some(query) = path.strip_prefix("/graph?") {
            return Some(View::Graph(query.to_owned()));
        }
        let rest = path.strip_prefix("/pages/")?;
        let (encoded, history) = match rest.strip_suffix("/history") {
            Some(encoded) => (encoded, true),
            None => (rest, false),
        };
        if encoded.is_empty() || encoded.contains('/') {
            return None;
        }
        let title = percent_encoding::percent_decode_str(encoded)
            .decode_utf8()
            .ok()?
            .into_owned();
        Some(if history {
            View::History(title)
        } else {
            View::Page(title)
        })
    }
}

#[derive(Deserialize)]
pub struct LiveQuery {
    view: String,
}

pub async fn live(
    State(state): State<Arc<WebState>>,
    Extension(session): Extension<Session>,
    filter: SphereFilter,
    Query(query): Query<LiveQuery>,
) -> Response {
    let Some(view) = View::parse(&query.view) else {
        return (StatusCode::BAD_REQUEST, "unknown view").into_response();
    };
    let mut changes = state.changes.clone();
    changes.mark_unchanged();
    let shutdown = state.shutdown.clone();
    let updates = stream::unfold(
        (changes, state, session, filter, view),
        |(mut changes, state, session, filter, view)| async move {
            changes.changed().await.ok()?;
            tokio::time::sleep(COALESCE).await;
            changes.mark_unchanged();
            let events = render_events(&state, &session, &filter, &view).await;
            Some((
                stream::iter(events),
                (changes, state, session, filter, view),
            ))
        },
    )
    .flatten()
    .map(Ok::<Event, Infallible>)
    .take_until(shutdown.cancelled_owned());
    Sse::new(updates)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Renders the view again, with the owner's sphere filter, and returns the events that update
/// it in the browser.
async fn render_events(
    state: &Arc<WebState>,
    session: &Session,
    filter: &SphereFilter,
    view: &View,
) -> Vec<Event> {
    let state = State(state.clone());
    let session = Extension(session.clone());
    let filter = filter.clone();
    let response = match view {
        View::Index => super::index(state, session, filter).await,
        View::Page(title) => super::page(state, session, filter, Path(title.clone())).await,
        View::History(title) => super::history(state, session, filter, Path(title.clone())).await,
        View::Graph(query) => {
            super::graph(state, session, filter, RawQuery(Some(query.clone()))).await
        }
        View::Stats => super::stats(state, session, filter).await,
    };
    if response.status() != StatusCode::OK {
        // The page was renamed or deleted; a reload shows where it went.
        return Vec::new();
    }
    let Ok(bytes) = body::to_bytes(response.into_body(), MAX_VIEW_BYTES).await else {
        return Vec::new();
    };
    let html = String::from_utf8_lossy(&bytes);
    let Some(main) = main_element(&html) else {
        return Vec::new();
    };
    let mut events = vec![PatchElements::new(main).write_as_axum_sse_event()];
    if main.contains("class=\"mermaid\"") {
        events.push(ExecuteScript::new("mermaid.run()").write_as_axum_sse_event());
    }
    events
}

/// The `<main id="content">…</main>` element of a rendered page.
fn main_element(html: &str) -> Option<&str> {
    let start = html.find("<main id=\"content\"")?;
    let end = html.rfind("</main>")? + "</main>".len();
    html.get(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_are_parsed_from_paths() {
        assert_eq!(View::parse("/"), Some(View::Index));
        assert_eq!(
            View::parse("/pages/C%2B%2B%20%2F%20Rust"),
            Some(View::Page("C++ / Rust".into()))
        );
        assert_eq!(
            View::parse("/pages/Rust/history"),
            Some(View::History("Rust".into()))
        );
        assert_eq!(View::parse("/pages/Rust/revisions/1"), None);
        assert_eq!(View::parse("/search"), None);
        assert_eq!(View::parse("/graph"), Some(View::Graph(String::new())));
        assert_eq!(View::parse("/stats"), Some(View::Stats));
        assert_eq!(
            View::parse("/graph?type=tool&type=project"),
            Some(View::Graph("type=tool&type=project".into()))
        );
        assert_eq!(View::parse("https://evil.example/"), None);
    }

    #[test]
    fn the_main_element_is_cut_out() {
        let html = "<html><body><header>x</header><main id=\"content\"><h1>A</h1></main><script></script></body></html>";
        assert_eq!(
            main_element(html),
            Some("<main id=\"content\"><h1>A</h1></main>")
        );
    }
}
