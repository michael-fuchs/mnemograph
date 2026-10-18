//! Domain model and storage of mnemograph.
//!
//! [`Knowledge`] is the single entry point: it owns the SQLite database, which is the only
//! source of truth (ADR 0003). The API is synchronous; async callers should run it on a
//! blocking thread.

mod links;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use rusqlite_migration::{M, Migrations};

const MIGRATIONS_SLICE: &[M<'_>] = &[
    M::up(include_str!("../migrations/0001-pages.sql")),
    M::up(include_str!("../migrations/0002-revisions.sql")),
    M::up(include_str!("../migrations/0003-links.sql")),
];
/// Schema version that introduced the `link` table; older databases need their links extracted.
const LINKS_SCHEMA_VERSION: i64 = 3;
const MIGRATIONS: Migrations<'_> = Migrations::from_slice(MIGRATIONS_SLICE);

/// Characters that would break the `[[Title|label]]` link syntax.
const FORBIDDEN_TITLE_CHARS: &[char] = &['[', ']', '|', '\n', '\r'];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid title {0:?}: titles must be non-empty and must not contain [ ] | or line breaks"
    )]
    InvalidTitle(String),
    #[error("no page titled {0:?}")]
    PageNotFound(String),
    /// The write did not name the page's current revision as its base. The caller should merge
    /// its change into `current` and retry with `current.revision` as the base.
    #[error("{}", conflict_message(.base, .current))]
    Conflict {
        base: Option<u32>,
        current: Box<Page>,
    },
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
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
    pub body: String,
    /// Number of this revision; the first revision of a page is 1.
    pub revision: u32,
    /// When the page was created, as an RFC 3339 timestamp (UTC).
    pub created_at: String,
    /// When this revision was created, as an RFC 3339 timestamp (UTC).
    pub updated_at: String,
}

/// A link from a page to a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    pub title: String,
    /// True if no page has this title yet.
    pub stub: bool,
}

/// The links around a page or stub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    /// Links in the page's current revision, in order of first appearance. Empty for a stub.
    pub outgoing: Vec<LinkTarget>,
    /// Titles of the pages linking here, sorted.
    pub backlinks: Vec<String>,
}

/// The knowledge base, stored in a SQLite database.
pub struct Knowledge {
    conn: Mutex<Connection>,
}

impl Knowledge {
    /// Opens (or creates) the database at `path` and migrates it to the latest schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// Opens a fresh in-memory database, mainly for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version_before: i64 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        MIGRATIONS.to_latest(&mut conn)?;
        if version_before < LINKS_SCHEMA_VERSION {
            extract_all_links(&mut conn)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Creates or changes a page.
    ///
    /// Without `base_revision` this only creates a new page. Changing an existing page requires
    /// its current revision as `base_revision`, so that a writer never overwrites a change it
    /// has not seen; otherwise [`Error::Conflict`] returns the current revision. Writing the
    /// body the page already has creates no new revision.
    pub fn write_page(&self, title: &str, body: &str, base_revision: Option<u32>) -> Result<Page> {
        let title = validate_title(title)?;
        let mut conn = self.conn();
        let tx = Transaction::new(&mut conn, TransactionBehavior::Immediate)?;

        let page = match current_page(&tx, title)? {
            None => {
                if base_revision.is_some() {
                    return Err(Error::PageNotFound(title.to_owned()));
                }
                tx.execute("INSERT INTO page (title) VALUES (?1)", [title])?;
                let page_id = tx.last_insert_rowid();
                insert_revision(&tx, page_id, 1, body)?;
                replace_links(&tx, page_id, body)?;
                current_page(&tx, title)?.expect("page was just inserted")
            }
            Some((page_id, current)) => {
                if base_revision != Some(current.revision) {
                    return Err(Error::Conflict {
                        base: base_revision,
                        current: Box::new(current),
                    });
                }
                if current.body == body {
                    return Ok(current);
                }
                insert_revision(&tx, page_id, current.revision + 1, body)?;
                replace_links(&tx, page_id, body)?;
                current_page(&tx, title)?.expect("page exists")
            }
        };
        tx.commit()?;
        Ok(page.1)
    }

    /// Returns the current revision of the page with exactly this title, if it exists.
    pub fn page(&self, title: &str) -> Result<Option<Page>> {
        Ok(current_page(&self.conn(), title.trim())?.map(|(_, page)| page))
    }

    /// Returns a specific revision of a page, if both exist.
    pub fn page_revision(&self, title: &str, revision: u32) -> Result<Option<Page>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{PAGE_SELECT} WHERE p.title = ?1 AND r.number = ?2"),
                params![title.trim(), revision],
                row_to_page,
            )
            .optional()?
            .map(|(_, page)| page))
    }

    /// Returns the links around a title, whether it is a page or a stub.
    pub fn links(&self, title: &str) -> Result<Links> {
        let conn = self.conn();
        let title = title.trim();
        let outgoing = conn
            .prepare(
                "SELECT l.target_title, NOT EXISTS (SELECT 1 FROM page t WHERE t.title = l.target_title) \
                 FROM link l JOIN page p ON p.id = l.source_page_id \
                 WHERE p.title = ?1 ORDER BY l.position",
            )?
            .query_map([title], |row| {
                Ok(LinkTarget {
                    title: row.get(0)?,
                    stub: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let backlinks = conn
            .prepare(
                "SELECT p.title FROM link l JOIN page p ON p.id = l.source_page_id \
                 WHERE l.target_title = ?1 ORDER BY p.title",
            )?
            .query_map([title], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Links {
            outgoing,
            backlinks,
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave SQLite in an inconsistent state, because
        // writes happen in transactions, so a poisoned lock is safe to reuse.
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn validate_title(title: &str) -> Result<&str> {
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed.contains(FORBIDDEN_TITLE_CHARS) {
        return Err(Error::InvalidTitle(title.to_owned()));
    }
    Ok(trimmed)
}

const PAGE_SELECT: &str = "SELECT p.id, p.title, r.body, r.number, p.created_at, r.created_at \
     FROM page p JOIN revision r ON r.page_id = p.id";

fn row_to_page(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, Page)> {
    Ok((
        row.get(0)?,
        Page {
            title: row.get(1)?,
            body: row.get(2)?,
            revision: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        },
    ))
}

/// Returns the page's ID and its current revision.
fn current_page(conn: &Connection, title: &str) -> Result<Option<(i64, Page)>> {
    Ok(conn
        .query_row(
            &format!("{PAGE_SELECT} WHERE p.title = ?1 ORDER BY r.number DESC LIMIT 1"),
            [title],
            row_to_page,
        )
        .optional()?)
}

/// Replaces the page's links with those in `body`. A page never links to itself.
fn replace_links(conn: &Connection, page_id: i64, body: &str) -> Result<()> {
    conn.execute("DELETE FROM link WHERE source_page_id = ?1", [page_id])?;
    let mut insert = conn.prepare(
        "INSERT INTO link (source_page_id, target_title, position) \
         SELECT ?1, ?2, ?3 WHERE ?2 <> (SELECT title FROM page WHERE id = ?1)",
    )?;
    for (position, target) in (0_i64..).zip(links::link_targets(body)) {
        insert.execute(params![page_id, target, position])?;
    }
    Ok(())
}

/// Extracts the links of every page's current revision, for databases created before links
/// existed.
fn extract_all_links(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    let pages: Vec<(i64, String)> = tx
        .prepare(
            "SELECT r.page_id, r.body FROM revision r \
             WHERE r.number = (SELECT max(number) FROM revision WHERE page_id = r.page_id)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (page_id, body) in pages {
        replace_links(&tx, page_id, &body)?;
    }
    tx.commit()?;
    Ok(())
}

fn insert_revision(conn: &Connection, page_id: i64, number: u32, body: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO revision (page_id, number, body) VALUES (?1, ?2, ?3)",
        params![page_id, number, body],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knowledge() -> Knowledge {
        Knowledge::open_in_memory().unwrap()
    }

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }

    #[test]
    fn created_page_can_be_read_back() {
        let knowledge = knowledge();
        let created = knowledge
            .write_page("Rauthy", "An OIDC provider.", None)
            .unwrap();
        assert_eq!(created.title, "Rauthy");
        assert_eq!(created.revision, 1);

        let read = knowledge.page("Rauthy").unwrap().unwrap();
        assert_eq!(read, created);
    }

    #[test]
    fn missing_page_is_none() {
        assert_eq!(knowledge().page("Nothing").unwrap(), None);
    }

    #[test]
    fn title_is_trimmed() {
        let knowledge = knowledge();
        knowledge.write_page("  Rust ", "", None).unwrap();
        assert!(knowledge.page("Rust").unwrap().is_some());
    }

    #[test]
    fn titles_breaking_link_syntax_are_rejected() {
        let knowledge = knowledge();
        for title in ["", "   ", "a|b", "[[x]]", "two\nlines"] {
            let err = knowledge.write_page(title, "", None).unwrap_err();
            assert!(matches!(err, Error::InvalidTitle(_)), "{title:?}");
        }
    }

    #[test]
    fn update_with_current_base_creates_a_new_revision_and_keeps_the_old_one() {
        let knowledge = knowledge();
        knowledge.write_page("Caddy", "v1", None).unwrap();
        let updated = knowledge.write_page("Caddy", "v2", Some(1)).unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.body, "v2");
        assert_eq!(knowledge.page("Caddy").unwrap().unwrap(), updated);

        let first = knowledge.page_revision("Caddy", 1).unwrap().unwrap();
        assert_eq!(first.body, "v1");
        assert_eq!(first.created_at, updated.created_at);
    }

    #[test]
    fn update_without_base_is_a_conflict() {
        let knowledge = knowledge();
        knowledge.write_page("Rust", "first", None).unwrap();
        let err = knowledge.write_page("Rust", "second", None).unwrap_err();
        let Error::Conflict { base, current } = err else {
            panic!("expected a conflict, got {err:?}");
        };
        assert_eq!(base, None);
        assert_eq!((current.revision, current.body.as_str()), (1, "first"));
    }

    #[test]
    fn update_with_stale_base_is_a_conflict_and_changes_nothing() {
        let knowledge = knowledge();
        knowledge.write_page("Rust", "v1", None).unwrap();
        knowledge
            .write_page("Rust", "v2 by agent A", Some(1))
            .unwrap();

        let err = knowledge
            .write_page("Rust", "v2 by agent B", Some(1))
            .unwrap_err();
        let Error::Conflict { base, current } = err else {
            panic!("expected a conflict, got {err:?}");
        };
        assert_eq!(base, Some(1));
        assert_eq!(
            (current.revision, current.body.as_str()),
            (2, "v2 by agent A")
        );
        assert_eq!(knowledge.page("Rust").unwrap().unwrap().revision, 2);
    }

    #[test]
    fn base_revision_for_a_missing_page_is_rejected() {
        let err = knowledge().write_page("Nothing", "x", Some(1)).unwrap_err();
        assert!(matches!(err, Error::PageNotFound(_)));
    }

    #[test]
    fn unchanged_body_creates_no_revision() {
        let knowledge = knowledge();
        knowledge.write_page("Rust", "same", None).unwrap();
        let page = knowledge.write_page("Rust", "same", Some(1)).unwrap();
        assert_eq!(page.revision, 1);
    }

    #[test]
    fn data_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mnemograph.db");
        Knowledge::open(&path)
            .unwrap()
            .write_page("Caddy", "A reverse proxy.", None)
            .unwrap();
        let reopened = Knowledge::open(&path).unwrap();
        assert_eq!(
            reopened.page("Caddy").unwrap().unwrap().body,
            "A reverse proxy."
        );
    }

    fn target(title: &str, stub: bool) -> LinkTarget {
        LinkTarget {
            title: title.to_owned(),
            stub,
        }
    }

    #[test]
    fn links_are_extracted_and_unwritten_titles_are_stubs() {
        let knowledge = knowledge();
        knowledge.write_page("Rust", "A language.", None).unwrap();
        knowledge
            .write_page(
                "mnemograph",
                "Written in [[Rust]], uses [[Tantivy|a search index]].",
                None,
            )
            .unwrap();

        let links = knowledge.links("mnemograph").unwrap();
        assert_eq!(
            links.outgoing,
            [target("Rust", false), target("Tantivy", true)]
        );
        assert_eq!(knowledge.links("Rust").unwrap().backlinks, ["mnemograph"]);
    }

    #[test]
    fn stub_has_backlinks_and_becomes_a_page_when_written() {
        let knowledge = knowledge();
        knowledge.write_page("A", "See [[Caddy]].", None).unwrap();
        knowledge.write_page("B", "Also [[Caddy]].", None).unwrap();

        let stub = knowledge.links("Caddy").unwrap();
        assert!(stub.outgoing.is_empty());
        assert_eq!(stub.backlinks, ["A", "B"]);

        knowledge
            .write_page("Caddy", "A reverse proxy.", None)
            .unwrap();
        assert_eq!(
            knowledge.links("A").unwrap().outgoing,
            [target("Caddy", false)]
        );
        assert_eq!(knowledge.links("Caddy").unwrap().backlinks, ["A", "B"]);
    }

    #[test]
    fn new_revision_replaces_links() {
        let knowledge = knowledge();
        knowledge
            .write_page("Page", "[[Old]] and [[Kept]]", None)
            .unwrap();
        knowledge
            .write_page("Page", "[[Kept]] and [[New]]", Some(1))
            .unwrap();

        let titles: Vec<_> = knowledge
            .links("Page")
            .unwrap()
            .outgoing
            .into_iter()
            .map(|l| l.title)
            .collect();
        assert_eq!(titles, ["Kept", "New"]);
        assert!(knowledge.links("Old").unwrap().backlinks.is_empty());
    }

    #[test]
    fn self_links_are_ignored() {
        let knowledge = knowledge();
        knowledge
            .write_page("Rust", "[[Rust]] is [[Rust]].", None)
            .unwrap();
        assert_eq!(knowledge.links("Rust").unwrap(), Links::default());
    }

    #[test]
    fn migration_extracts_links_of_existing_pages() {
        let mut conn = Connection::open_in_memory().unwrap();
        MIGRATIONS.to_version(&mut conn, 2).unwrap();
        conn.execute_batch(
            "INSERT INTO page (id, title) VALUES (1, 'Old');
             INSERT INTO revision (page_id, number, body) VALUES (1, 1, '[[Gone]]'), (1, 2, '[[Current]]');",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v2.db");
        conn.backup(rusqlite::MAIN_DB, &path, None).unwrap();
        drop(conn);

        let knowledge = Knowledge::open(&path).unwrap();
        assert_eq!(
            knowledge.links("Old").unwrap().outgoing,
            [target("Current", true)]
        );
    }

    #[test]
    fn migration_turns_existing_pages_into_first_revisions() {
        let mut conn = Connection::open_in_memory().unwrap();
        MIGRATIONS.to_version(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO page (title, body) VALUES ('Old', 'from v1 schema')",
            [],
        )
        .unwrap();
        MIGRATIONS.to_latest(&mut conn).unwrap();

        let knowledge = Knowledge {
            conn: Mutex::new(conn),
        };
        let page = knowledge.page("Old").unwrap().unwrap();
        assert_eq!((page.revision, page.body.as_str()), (1, "from v1 schema"));
    }
}
