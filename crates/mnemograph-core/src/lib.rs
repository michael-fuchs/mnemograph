//! Domain model and storage of mnemograph.
//!
//! [`Knowledge`] is the single entry point: it owns the SQLite database, which is the only
//! source of truth (ADR 0003). The API is synchronous; async callers should run it on a
//! blocking thread.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};

const MIGRATIONS_SLICE: &[M<'_>] = &[M::up(include_str!("../migrations/0001-pages.sql"))];
const MIGRATIONS: Migrations<'_> = Migrations::from_slice(MIGRATIONS_SLICE);

/// Characters that would break the `[[Title|label]]` link syntax.
const FORBIDDEN_TITLE_CHARS: &[char] = &['[', ']', '|', '\n', '\r'];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid title {0:?}: titles must be non-empty and must not contain [ ] | or line breaks"
    )]
    InvalidTitle(String),
    #[error("a page titled {0:?} already exists")]
    TitleTaken(String),
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A curated, evolving unit of knowledge about one subject, identified by its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub body: String,
    /// RFC 3339 timestamp (UTC).
    pub created_at: String,
    /// RFC 3339 timestamp (UTC).
    pub updated_at: String,
}

/// The knowledge base: pages stored in a SQLite database.
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
        MIGRATIONS.to_latest(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Creates a new page. Fails if a page with the same title already exists.
    pub fn create_page(&self, title: &str, body: &str) -> Result<Page> {
        let title = validate_title(title)?;
        let conn = self.conn();
        let inserted = conn.execute(
            "INSERT INTO page (title, body) VALUES (?1, ?2) ON CONFLICT (title) DO NOTHING",
            params![title, body],
        )?;
        if inserted == 0 {
            return Err(Error::TitleTaken(title.to_owned()));
        }
        Ok(query_page(&conn, title)?.expect("page was just inserted"))
    }

    /// Returns the page with exactly this title, if it exists.
    pub fn page(&self, title: &str) -> Result<Option<Page>> {
        query_page(&self.conn(), title.trim())
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave SQLite in an inconsistent state, because
        // every statement is atomic, so a poisoned lock is safe to reuse.
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

fn query_page(conn: &Connection, title: &str) -> Result<Option<Page>> {
    Ok(conn
        .query_row(
            "SELECT title, body, created_at, updated_at FROM page WHERE title = ?1",
            [title],
            |row| {
                Ok(Page {
                    title: row.get(0)?,
                    body: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }

    #[test]
    fn created_page_can_be_read_back() {
        let knowledge = Knowledge::open_in_memory().unwrap();
        let created = knowledge
            .create_page("Rauthy", "An OIDC provider.")
            .unwrap();
        assert_eq!(created.title, "Rauthy");

        let read = knowledge.page("Rauthy").unwrap().unwrap();
        assert_eq!(read, created);
    }

    #[test]
    fn missing_page_is_none() {
        let knowledge = Knowledge::open_in_memory().unwrap();
        assert_eq!(knowledge.page("Nothing").unwrap(), None);
    }

    #[test]
    fn title_is_trimmed() {
        let knowledge = Knowledge::open_in_memory().unwrap();
        knowledge.create_page("  Rust ", "").unwrap();
        assert!(knowledge.page("Rust").unwrap().is_some());
    }

    #[test]
    fn duplicate_title_is_rejected() {
        let knowledge = Knowledge::open_in_memory().unwrap();
        knowledge.create_page("Rust", "first").unwrap();
        let err = knowledge.create_page("Rust", "second").unwrap_err();
        assert!(matches!(err, Error::TitleTaken(t) if t == "Rust"));
    }

    #[test]
    fn titles_breaking_link_syntax_are_rejected() {
        let knowledge = Knowledge::open_in_memory().unwrap();
        for title in ["", "   ", "a|b", "[[x]]", "two\nlines"] {
            let err = knowledge.create_page(title, "").unwrap_err();
            assert!(matches!(err, Error::InvalidTitle(_)), "{title:?}");
        }
    }

    #[test]
    fn data_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mnemograph.db");
        Knowledge::open(&path)
            .unwrap()
            .create_page("Caddy", "A reverse proxy.")
            .unwrap();
        let reopened = Knowledge::open(&path).unwrap();
        assert_eq!(
            reopened.page("Caddy").unwrap().unwrap().body,
            "A reverse proxy."
        );
    }
}
