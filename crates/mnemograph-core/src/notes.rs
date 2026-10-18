//! Notes: short, timestamped, append-only entries that link to the pages they concern.

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::search::{IndexedText, Item};
use crate::title::title_key;
use crate::{
    Error, Knowledge, LinkTarget, RESOLVED_KEYS, Result, increase_change_counter, links, names,
};

/// A short, timestamped, append-only entry recording something worth remembering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub body: String,
    /// RFC 3339 timestamp (UTC).
    pub created_at: String,
    /// The titles the note links to, in order of first appearance.
    pub links: Vec<LinkTarget>,
}

impl Knowledge {
    /// Records a note. Its `[[links]]` attach it to the pages (or stubs) it concerns.
    pub fn add_note(&self, body: &str) -> Result<Note> {
        if body.trim().is_empty() {
            return Err(Error::EmptyNote);
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO note (body) VALUES (?1)", [body])?;
        let note_id = tx.last_insert_rowid();
        let mut insert = tx.prepare(
            "INSERT INTO note_link (note_id, target_title, target_key, position) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (position, target) in (0_i64..).zip(links::link_targets(body)) {
            insert.execute(params![note_id, target, title_key(&target), position])?;
        }
        drop(insert);
        increase_change_counter(&tx)?;
        let note = note(&tx, note_id)?;
        tx.commit()?;
        self.index_after_commit(
            &conn,
            &[(
                Item::Note(note_id),
                Some(IndexedText {
                    item: Item::Note(note_id),
                    names: &[],
                    page_type: None,
                    body: &note.body,
                }),
            )],
        );
        Ok(note)
    }

    /// Returns the most recent notes linking to this title or alias (of a page or a stub),
    /// newest first.
    pub fn notes_about(&self, title: &str, limit: u32) -> Result<Vec<Note>> {
        let conn = self.conn();
        let (_, keys) = names(&conn, title)?;
        let placeholders = vec!["?"; keys.len()].join(", ");
        let ids: Vec<i64> = conn
            .prepare(&format!(
                "SELECT DISTINCT note_id FROM note_link WHERE target_key IN ({placeholders}) \
                 ORDER BY note_id DESC LIMIT {limit}"
            ))?
            .query_map(params_from_iter(&keys), |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter().map(|id| note(&conn, id)).collect()
    }
}

/// Returns the note with this ID, if it exists.
pub(crate) fn find_note(conn: &Connection, note_id: i64) -> Result<Option<Note>> {
    let exists = conn
        .query_row("SELECT 1 FROM note WHERE id = ?1", [note_id], |_| Ok(()))
        .optional()?;
    exists.map(|()| note(conn, note_id)).transpose()
}

fn note(conn: &Connection, note_id: i64) -> Result<Note> {
    let (body, created_at) = conn.query_row(
        "SELECT body, created_at FROM note WHERE id = ?1",
        [note_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let links = conn
        .prepare(&format!(
            "SELECT l.target_title, t.title FROM note_link l \
             LEFT JOIN ({RESOLVED_KEYS}) k ON k.key = l.target_key \
             LEFT JOIN page t ON t.id = k.page_id \
             WHERE l.note_id = ?1 ORDER BY l.position"
        ))?
        .query_map([note_id], |row| {
            Ok(LinkTarget {
                title: row.get(0)?,
                page: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Note {
        id: note_id,
        body,
        created_at,
        links,
    })
}
