//! Domain model and storage of mnemograph.
//!
//! [`Knowledge`] is the single entry point: it owns the SQLite database, which is the only
//! source of truth (ADR 0003). The API is synchronous; async callers should run it on a
//! blocking thread.

mod author;
mod delete;
mod export;
mod links;
mod notes;
mod related;
mod schema;
mod search;
mod sessions;
mod title;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub use author::Author;
pub use delete::DeletedPage;
pub use export::ExportSummary;
pub use notes::Note;
pub use related::{MAX_HOPS, Related};
pub use sessions::Session;

use search::{IndexedText, Item, SearchIndex};
use title::clean_title;
/// The key under which titles and aliases are compared (case-insensitive, whitespace collapsed).
pub use title::title_key;

/// Page types allowed when none are configured.
pub const DEFAULT_PAGE_TYPES: &[&str] = &[
    "project",
    "technology",
    "decision",
    "person",
    "tool",
    "concept",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid title {0:?}: titles must be non-empty and must not contain [ ] | or line breaks"
    )]
    InvalidTitle(String),
    #[error("no page titled {0:?}")]
    PageNotFound(String),
    #[error("no note with ID {0}")]
    NoteNotFound(i64),
    /// The write did not name the page's current revision as its base. The caller should merge
    /// its change into `current` and retry with `current.revision` as the base.
    #[error("{}", conflict_message(.base, .current))]
    Conflict {
        base: Option<u32>,
        current: Box<Page>,
    },
    #[error("{name:?} is already the title or an alias of the page {page:?}")]
    NameTaken { name: String, page: String },
    #[error("unknown page type {given:?}; allowed types: {}", .allowed.join(", "))]
    UnknownPageType { given: String, allowed: Vec<String> },
    #[error("a note must not be empty")]
    EmptyNote,
    #[error("the export directory {0} is not empty")]
    ExportTargetNotEmpty(String),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error(
        "the search index {0} is in use by another process; is `mnemograph serve` running on this database?"
    )]
    IndexInUse(String),
    #[error("search index error: {0}")]
    Search(#[from] tantivy::TantivyError),
}

fn conflict_message(base: &Option<u32>, current: &Page) -> String {
    match base {
        None => format!(
            "a page titled {:?} already exists (revision {}); to change it, pass its current revision as the base",
            current.title, current.revision
        ),
        Some(base) => format!(
            "page {:?} has changed: the base revision is {base}, but the current revision is {}",
            current.title, current.revision
        ),
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A page in one of its revisions: a curated, evolving unit of knowledge about one subject,
/// identified by its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub page_type: Option<String>,
    /// Alternative titles that lead to this page, sorted.
    pub aliases: Vec<String>,
    pub body: String,
    /// Number of this revision; the first revision of a page is 1.
    pub revision: u32,
    /// When the page was created, as an RFC 3339 timestamp (UTC).
    pub created_at: String,
    /// When this revision was created, as an RFC 3339 timestamp (UTC).
    pub updated_at: String,
    /// Who created this revision; `None` for revisions from before authors were recorded.
    pub author: Option<Author>,
}

/// What a write changes besides the body. `None` fields keep their current value (for a new
/// page: no type, no aliases).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageChanges {
    /// Renames the page; the old title stays reachable as an alias.
    pub new_title: Option<String>,
    /// `Some(None)` removes the page type.
    pub page_type: Option<Option<String>>,
    /// Replaces all aliases.
    pub aliases: Option<Vec<String>>,
}

/// One entry of a page's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionSummary {
    pub number: u32,
    /// The page's title in this revision.
    pub title: String,
    pub page_type: Option<String>,
    /// When this revision was created, as an RFC 3339 timestamp (UTC).
    pub created_at: String,
    pub author: Option<Author>,
    /// Length of the body in bytes.
    pub body_bytes: usize,
}

/// A link from a page to a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// The title as written in the link.
    pub title: String,
    /// The title of the page the link leads to; `None` if the link leads to a stub.
    pub page: Option<String>,
}

impl LinkTarget {
    /// True if no page has this title (or alias) yet.
    pub fn is_stub(&self) -> bool {
        self.page.is_none()
    }
}

/// The links around a page or stub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    /// Links in the page's current revision, in order of first appearance. Empty for a stub.
    pub outgoing: Vec<LinkTarget>,
    /// Titles of the pages linking here, under its title or any alias, sorted.
    pub backlinks: Vec<String>,
}

/// Something a search found.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub found: Found,
    /// The matching part of the text, with matched words in **bold**.
    pub snippet: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Page {
        title: String,
        page_type: Option<String>,
    },
    Note(Note),
}

/// The knowledge base, stored in a SQLite database with a derived search index.
pub struct Knowledge {
    conn: Mutex<Connection>,
    search: SearchIndex,
    page_types: Vec<String>,
}

impl Knowledge {
    /// Opens (or creates) the database at `path` and migrates it to the latest schema. The
    /// search index lives next to it in `<path>.search/` and is rebuilt if it is missing or
    /// outdated.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut index_dir = path.as_os_str().to_owned();
        index_dir.push(".search");
        let index_dir = Path::new(&index_dir);
        let search = SearchIndex::open(index_dir).map_err(|err| match err {
            tantivy::TantivyError::LockFailure(..) => {
                Error::IndexInUse(index_dir.display().to_string())
            }
            err => err.into(),
        })?;
        Self::init(Connection::open(path)?, search)
    }

    /// Opens a fresh in-memory database, mainly for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, SearchIndex::in_memory()?)
    }

    fn init(mut conn: Connection, search: SearchIndex) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrations().to_latest(&mut conn)?;
        let knowledge = Self {
            conn: Mutex::new(conn),
            search,
            page_types: DEFAULT_PAGE_TYPES.iter().map(|t| t.to_string()).collect(),
        };
        let counter = change_counter(&knowledge.conn())?;
        if knowledge.search.change_counter() != Some(counter) {
            tracing::info!("search index is missing or outdated; rebuilding it");
            knowledge.rebuild_search_index()?;
        }
        Ok(knowledge)
    }

    /// Rebuilds the search index from the database.
    pub fn rebuild_search_index(&self) -> Result<()> {
        let conn = self.conn();
        let pages: Vec<(i64, Page)> = conn
            .prepare(&format!(
                "{PAGE_SELECT_WITH_ID} WHERE r.number = \
                 (SELECT max(number) FROM revision WHERE page_id = p.id)"
            ))?
            .query_map([], |row| Ok((row.get(0)?, row_to_page_from(row, 1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let notes: Vec<(i64, String)> = conn
            .prepare("SELECT id, body FROM note")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let names: Vec<Vec<&str>> = pages.iter().map(|(_, page)| page_names(page)).collect();
        let texts = pages
            .iter()
            .zip(&names)
            .map(|((id, page), names)| IndexedText {
                item: Item::Page(*id),
                names,
                page_type: page.page_type.as_deref(),
                body: &page.body,
            })
            .chain(notes.iter().map(|(id, body)| IndexedText {
                item: Item::Note(*id),
                names: &[],
                page_type: None,
                body,
            }));
        self.search.rebuild(texts, change_counter(&conn)?)?;
        Ok(())
    }

    /// Searches pages and notes by full text, best matches first. With `page_type`, only pages
    /// of that type are searched.
    pub fn search(
        &self,
        query: &str,
        page_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let page_type = self.check_page_type(page_type.map(str::to_owned))?;
        let hits = self.search.search(query, page_type.as_deref(), limit)?;
        let conn = self.conn();
        let mut found = Vec::with_capacity(hits.len());
        for hit in hits {
            let item = match hit.item {
                Item::Page(id) => conn
                    .query_row(
                        "SELECT title, page_type FROM page WHERE id = ?1",
                        [id],
                        |row| {
                            Ok(Found::Page {
                                title: row.get(0)?,
                                page_type: row.get(1)?,
                            })
                        },
                    )
                    .optional()?,
                Item::Note(id) => notes::find_note(&conn, id)?.map(Found::Note),
            };
            // A hit without a row means the index is ahead of a deletion; skip it.
            if let Some(item) = item {
                found.push(SearchHit {
                    found: item,
                    snippet: hit.snippet,
                    score: hit.score,
                });
            }
        }
        Ok(found)
    }

    /// Updates the search index after a committed write, while the caller still holds the
    /// database lock so index commits happen in the same order as database commits. A failure
    /// leaves the index outdated, which the next start detects and repairs.
    fn index_after_commit(&self, conn: &Connection, texts: &[(Item, Option<IndexedText<'_>>)]) {
        let result =
            change_counter(conn).and_then(|counter| Ok(self.search.update(texts, counter)?));
        if let Err(err) = result {
            tracing::warn!(%err, "updating the search index failed; it will be rebuilt on the next start");
        }
    }

    /// Replaces the allowed page types. Types are compared case-insensitively.
    pub fn with_page_types(mut self, page_types: impl IntoIterator<Item = String>) -> Self {
        self.page_types = page_types
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        self
    }

    /// The allowed page types.
    pub fn page_types(&self) -> &[String] {
        &self.page_types
    }

    /// Creates or changes the page with this title or alias.
    ///
    /// Without `base_revision` this only creates a new page. Changing an existing page requires
    /// its current revision as `base_revision`, so that a writer never overwrites a change it
    /// has not seen; otherwise [`Error::Conflict`] returns the current revision. A write that
    /// changes nothing creates no new revision.
    pub fn write_page(
        &self,
        title: &str,
        body: &str,
        base_revision: Option<u32>,
        changes: PageChanges,
        author: &Author,
    ) -> Result<Page> {
        let title = clean_title(title)?;
        let page_type = match changes.page_type {
            Some(page_type) => Some(self.check_page_type(page_type)?),
            None => None,
        };
        let mut conn = self.conn();
        let tx = Transaction::new(&mut conn, TransactionBehavior::Immediate)?;

        let (page_id, next) = match resolve(&tx, &title_key(&title))? {
            None => {
                if base_revision.is_some() {
                    return Err(Error::PageNotFound(title));
                }
                let new_title = match changes.new_title {
                    Some(new_title) => clean_title(&new_title)?,
                    None => title,
                };
                let snapshot = Snapshot {
                    title: new_title,
                    page_type: page_type.flatten(),
                    aliases: changes.aliases.unwrap_or_default(),
                    body: body.to_owned(),
                }
                .normalized(&tx, NO_PAGE)?;
                tx.execute(
                    "INSERT INTO page (title, title_key) VALUES (?1, ?2)",
                    params![snapshot.title, title_key(&snapshot.title)],
                )?;
                (tx.last_insert_rowid(), snapshot)
            }
            Some(page_id) => {
                let current = current_page(&tx, page_id)?;
                if base_revision != Some(current.revision) {
                    return Err(Error::Conflict {
                        base: base_revision,
                        current: Box::new(current),
                    });
                }
                let mut aliases = changes.aliases.unwrap_or_else(|| current.aliases.clone());
                let new_title = match changes.new_title {
                    Some(new_title) => {
                        aliases.push(current.title.clone());
                        clean_title(&new_title)?
                    }
                    None => current.title.clone(),
                };
                let snapshot = Snapshot {
                    title: new_title,
                    page_type: page_type.unwrap_or_else(|| current.page_type.clone()),
                    aliases,
                    body: body.to_owned(),
                }
                .normalized(&tx, page_id)?;
                if snapshot.matches(&current) {
                    return Ok(current);
                }
                (page_id, snapshot)
            }
        };

        next.store(&tx, page_id, author)?;
        increase_change_counter(&tx)?;
        let page = current_page(&tx, page_id)?;
        tx.commit()?;
        self.index_after_commit(
            &conn,
            &[(
                Item::Page(page_id),
                Some(IndexedText {
                    item: Item::Page(page_id),
                    names: &page_names(&page),
                    page_type: page.page_type.as_deref(),
                    body: &page.body,
                }),
            )],
        );
        Ok(page)
    }

    /// Returns the current revision of the page with this title or alias, if it exists.
    pub fn page(&self, title: &str) -> Result<Option<Page>> {
        let conn = self.conn();
        match resolve(&conn, &title_key(title))? {
            Some(page_id) => Ok(Some(current_page(&conn, page_id)?)),
            None => Ok(None),
        }
    }

    /// Returns the current revision of every page, sorted by title.
    pub fn pages(&self) -> Result<Vec<Page>> {
        let mut pages = self.pages_in_creation_order()?;
        pages.sort_by_cached_key(|page| page.title.to_lowercase());
        Ok(pages)
    }

    /// Returns the current revision of every page, oldest page first.
    pub(crate) fn pages_in_creation_order(&self) -> Result<Vec<Page>> {
        Ok(self
            .conn()
            .prepare(&format!(
                "{PAGE_SELECT} WHERE r.number = \
                 (SELECT max(number) FROM revision WHERE page_id = p.id) ORDER BY p.id"
            ))?
            .query_map([], row_to_page)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Returns a specific revision of the page with this title or alias, if both exist.
    pub fn page_revision(&self, title: &str, revision: u32) -> Result<Option<Page>> {
        let conn = self.conn();
        let Some(page_id) = resolve(&conn, &title_key(title))? else {
            return Ok(None);
        };
        Ok(conn
            .query_row(
                &format!("{PAGE_SELECT} WHERE p.id = ?1 AND r.number = ?2"),
                params![page_id, revision],
                row_to_page,
            )
            .optional()?)
    }

    /// Returns the history of the page with this title or alias, newest revision first, or
    /// `None` if there is no such page.
    pub fn revisions(&self, title: &str) -> Result<Option<Vec<RevisionSummary>>> {
        let conn = self.conn();
        let Some(page_id) = resolve(&conn, &title_key(title))? else {
            return Ok(None);
        };
        let revisions = conn
            .prepare(
                "SELECT number, title, page_type, created_at, author, length(CAST(body AS BLOB)) \
                 FROM revision WHERE page_id = ?1 ORDER BY number DESC",
            )?
            .query_map([page_id], |row| {
                Ok(RevisionSummary {
                    number: row.get(0)?,
                    title: row.get(1)?,
                    page_type: row.get(2)?,
                    created_at: row.get(3)?,
                    author: row
                        .get::<_, Option<String>>(4)?
                        .as_deref()
                        .and_then(Author::parse),
                    body_bytes: row.get::<_, i64>(5)? as usize,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(revisions))
    }

    /// Returns the links around a title or alias, whether it names a page or a stub.
    pub fn links(&self, title: &str) -> Result<Links> {
        let conn = self.conn();
        let (page_id, keys) = names(&conn, title)?;
        let Some(page_id) = page_id else {
            return Ok(Links {
                outgoing: Vec::new(),
                backlinks: backlinks(&conn, None, &keys)?,
            });
        };

        let outgoing = conn
            .prepare(&format!(
                "SELECT l.target_title, t.title FROM link l \
                 LEFT JOIN ({RESOLVED_KEYS}) k ON k.key = l.target_key \
                 LEFT JOIN page t ON t.id = k.page_id \
                 WHERE l.source_page_id = ?1 AND k.page_id IS NOT ?1 \
                 ORDER BY l.position"
            ))?
            .query_map([page_id], |row| {
                Ok(LinkTarget {
                    title: row.get(0)?,
                    page: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Links {
            outgoing,
            backlinks: backlinks(&conn, Some(page_id), &keys)?,
        })
    }

    fn check_page_type(&self, page_type: Option<String>) -> Result<Option<String>> {
        let Some(given) = page_type else {
            return Ok(None);
        };
        let normalized = given.trim().to_lowercase();
        if self.page_types.contains(&normalized) {
            Ok(Some(normalized))
        } else {
            Err(Error::UnknownPageType {
                given,
                allowed: self.page_types.clone(),
            })
        }
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave SQLite in an inconsistent state, because
        // writes happen in transactions, so a poisoned lock is safe to reuse.
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A page ID no page has (SQLite row IDs start at 1).
const NO_PAGE: i64 = 0;

/// Every key that leads to a page: its title key and its aliases' keys.
const RESOLVED_KEYS: &str =
    "SELECT title_key AS key, id AS page_id FROM page UNION ALL SELECT key, page_id FROM alias";

/// Returns the ID of the page whose title or alias has this key.
fn resolve(conn: &Connection, key: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            &format!("SELECT page_id FROM ({RESOLVED_KEYS}) WHERE key = ?1"),
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

/// Resolves a title or alias: returns the page it names (if any) and every key leading there —
/// the page's title and alias keys, or just the title's own key for a stub.
fn names(conn: &Connection, title: &str) -> Result<(Option<i64>, Vec<String>)> {
    let key = title_key(title);
    let Some(page_id) = resolve(conn, &key)? else {
        return Ok((None, vec![key]));
    };
    let keys = conn
        .prepare(&format!(
            "SELECT key FROM ({RESOLVED_KEYS}) WHERE page_id = ?1"
        ))?
        .query_map([page_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((Some(page_id), keys))
}

/// Titles of the pages linking to any of `keys`, except `page_id` itself.
fn backlinks(conn: &Connection, page_id: Option<i64>, keys: &[String]) -> Result<Vec<String>> {
    let mut backlinks = Vec::new();
    let mut query = conn.prepare(
        "SELECT p.title FROM link l JOIN page p ON p.id = l.source_page_id \
         WHERE l.target_key = ?1 AND p.id IS NOT ?2",
    )?;
    for key in keys {
        for title in query.query_map(params![key, page_id], |row| row.get::<_, String>(0))? {
            backlinks.push(title?);
        }
    }
    backlinks.sort();
    backlinks.dedup();
    Ok(backlinks)
}

/// Everything a revision records about a page.
struct Snapshot {
    title: String,
    page_type: Option<String>,
    aliases: Vec<String>,
    body: String,
}

impl Snapshot {
    /// Cleans and sorts the aliases, drops those equal to the title, and checks that neither
    /// title nor aliases belong to another page.
    fn normalized(mut self, conn: &Connection, page_id: i64) -> Result<Self> {
        let mut aliases: Vec<String> = Vec::new();
        for alias in &self.aliases {
            let alias = clean_title(alias)?;
            if title_key(&alias) != title_key(&self.title)
                && !aliases.iter().any(|a| title_key(a) == title_key(&alias))
            {
                aliases.push(alias);
            }
        }
        aliases.sort();
        self.aliases = aliases;

        for name in std::iter::once(&self.title).chain(&self.aliases) {
            if let Some(owner) = resolve(conn, &title_key(name))?
                && owner != page_id
            {
                let page =
                    conn.query_row("SELECT title FROM page WHERE id = ?1", [owner], |row| {
                        row.get(0)
                    })?;
                return Err(Error::NameTaken {
                    name: name.clone(),
                    page,
                });
            }
        }
        Ok(self)
    }

    fn matches(&self, page: &Page) -> bool {
        self.title == page.title
            && self.page_type == page.page_type
            && self.aliases == page.aliases
            && self.body == page.body
    }

    /// Records this snapshot as the page's next revision and updates its title, type, aliases
    /// and links accordingly.
    fn store(&self, tx: &Transaction<'_>, page_id: i64, author: &Author) -> Result<()> {
        let number: u32 = tx.query_row(
            "SELECT coalesce(max(number), 0) + 1 FROM revision WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO revision (page_id, number, title, page_type, aliases, body, author) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                page_id,
                number,
                self.title,
                self.page_type,
                self.aliases.join("\n"),
                self.body,
                author.to_string()
            ],
        )?;
        tx.execute(
            "UPDATE page SET title = ?2, title_key = ?3, page_type = ?4 WHERE id = ?1",
            params![page_id, self.title, title_key(&self.title), self.page_type],
        )?;
        tx.execute("DELETE FROM alias WHERE page_id = ?1", [page_id])?;
        for alias in &self.aliases {
            tx.execute(
                "INSERT INTO alias (key, alias, page_id) VALUES (?1, ?2, ?3)",
                params![title_key(alias), alias, page_id],
            )?;
        }
        replace_links(tx, page_id, &self.body)
    }
}

/// The page's title followed by its aliases.
fn page_names(page: &Page) -> Vec<&str> {
    std::iter::once(page.title.as_str())
        .chain(page.aliases.iter().map(String::as_str))
        .collect()
}

fn change_counter(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT value FROM meta WHERE key = 'change_counter'",
        [],
        |row| row.get(0),
    )?)
}

/// Every write transaction calls this, so the search index can tell whether it missed one.
fn increase_change_counter(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE meta SET value = value + 1 WHERE key = 'change_counter'",
        [],
    )?;
    Ok(())
}

const PAGE_SELECT_WITH_ID: &str = "SELECT p.id, r.title, r.page_type, r.aliases, r.body, \
     r.number, p.created_at, r.created_at, r.author FROM page p JOIN revision r ON r.page_id = p.id";

const PAGE_SELECT: &str = "SELECT r.title, r.page_type, r.aliases, r.body, r.number, \
     p.created_at, r.created_at, r.author FROM page p JOIN revision r ON r.page_id = p.id";

fn row_to_page(row: &rusqlite::Row<'_>) -> rusqlite::Result<Page> {
    row_to_page_from(row, 0)
}

/// Reads a page from the columns of [`PAGE_SELECT`], starting at column `first`.
fn row_to_page_from(row: &rusqlite::Row<'_>, first: usize) -> rusqlite::Result<Page> {
    let aliases: String = row.get(first + 2)?;
    Ok(Page {
        title: row.get(first)?,
        page_type: row.get(first + 1)?,
        aliases: aliases
            .split('\n')
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .collect(),
        body: row.get(first + 3)?,
        revision: row.get(first + 4)?,
        created_at: row.get(first + 5)?,
        updated_at: row.get(first + 6)?,
        author: row
            .get::<_, Option<String>>(first + 7)?
            .as_deref()
            .and_then(Author::parse),
    })
}

fn current_page(conn: &Connection, page_id: i64) -> Result<Page> {
    Ok(conn.query_row(
        &format!("{PAGE_SELECT} WHERE p.id = ?1 ORDER BY r.number DESC LIMIT 1"),
        [page_id],
        row_to_page,
    )?)
}

/// Replaces the page's links with those in `body`.
pub(crate) fn replace_links(conn: &Connection, page_id: i64, body: &str) -> Result<()> {
    conn.execute("DELETE FROM link WHERE source_page_id = ?1", [page_id])?;
    let mut insert = conn.prepare(
        "INSERT INTO link (source_page_id, target_title, target_key, position) \
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (position, target) in (0_i64..).zip(links::link_targets(body)) {
        insert.execute(params![page_id, target, title_key(&target), position])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
