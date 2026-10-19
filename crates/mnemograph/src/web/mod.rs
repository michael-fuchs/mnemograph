//! The owner's read-only web UI: browse pages, stubs and notes.

pub mod login;
pub mod render;

use std::collections::HashSet;
use std::sync::Arc;

use askama::Template;
use askama_web::WebTemplate;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Router, middleware};
use mnemograph_core::{Author, Knowledge, Note, Session, title_key};

use login::{LoginSettings, PendingLogins};
use render::{page_url, render};

/// How many recent notes the start page and a page show.
const RECENT_NOTES: u32 = 20;

/// What the web handlers share.
pub struct WebState {
    pub knowledge: Arc<Knowledge>,
    /// `None`: no login, everyone is the owner (development on loopback only).
    pub login: Option<LoginSettings>,
    pending_logins: PendingLogins,
}

/// The web UI's routes.
pub fn router(knowledge: Arc<Knowledge>, login: Option<LoginSettings>) -> Router {
    let state = Arc::new(WebState {
        knowledge,
        login,
        pending_logins: PendingLogins::default(),
    });
    let owner_only = Router::new()
        .route("/", get(index))
        .route("/pages/{title}", get(page))
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
        .with_state(state)
}

/// What every page's header shows.
struct Header {
    /// The logged-in owner, if login is enabled.
    owner_name: Option<String>,
}

impl Header {
    fn new(state: &WebState, session: &Session) -> Self {
        Self {
            owner_name: state.login.as_ref().map(|_| session.owner_name.clone()),
        }
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
    has_mermaid: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "stub.html")]
struct StubTemplate {
    header: Header,
    title: String,
    backlinks: Vec<LinkItem>,
    notes: Vec<NoteItem>,
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
        header: Header::new(&state, &session),
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
        Ok::<_, mnemograph_core::Error>((
            knowledge.page(&requested)?,
            knowledge.links(&requested)?,
            knowledge.notes_about(&requested, RECENT_NOTES)?,
        ))
    })
    .await;
    let Ok(Ok((page, links, notes))) = loaded else {
        return internal_error(&state, &session);
    };
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
            header,
            title: title.trim().to_owned(),
            backlinks,
            notes,
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
        header,
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

async fn logged_out() -> Response {
    message(
        StatusCode::OK,
        Header { owner_name: None },
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
    message(status, Header { owner_name: None }, "Login failed", text)
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
}
