//! Notes: short, timestamped, append-only entries that link to the pages they concern.

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::search::{IndexedText, Item};
use crate::title::title_key;
use crate::{
    Author, Error, Knowledge, LinkTarget, Result, Visibility, Writer, increase_change_counter,
    links, names, split_spheres,
};

/// A short, timestamped, append-only entry recording something worth remembering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub body: String,
    /// RFC 3339 timestamp (UTC).
    pub created_at: String,
    /// Who recorded the note; `None` for notes from before authors were recorded.
    pub author: Option<Author>,
    /// The titles the note links to, in order of first appearance.
    pub links: Vec<LinkTarget>,
    /// The spheres the note belongs to, sorted; never empty.
    pub spheres: Vec<String>,
}

impl Knowledge {
    /// Records a note in the sphere chosen from the writer's: `sphere`, which may be left out if
    /// the writer has only one. Its `[[links]]` attach it to the pages (or stubs) it concerns.
    pub fn add_note(&self, body: &str, sphere: Option<&str>, writer: &Writer) -> Result<Note> {
        if body.trim().is_empty() {
            return Err(Error::EmptyNote);
        }
        let sphere = writer.spheres.choose(sphere)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO note (body, author) VALUES (?1, ?2)",
            params![body, writer.author.to_string()],
        )?;
        let note_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO note_sphere (note_id, sphere) VALUES (?1, ?2)",
            params![note_id, sphere],
        )?;
        let mut insert = tx.prepare(
            "INSERT INTO note_link (note_id, target_title, target_key, position) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (position, target) in (0_i64..).zip(links::link_targets(body)) {
            insert.execute(params![note_id, target, title_key(&target), position])?;
        }
        drop(insert);
        increase_change_counter(&tx)?;
        let note = note(&tx, note_id, writer.visibility())?;
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

    /// Returns all notes, oldest first (for the owner).
    pub fn notes(&self) -> Result<Vec<Note>> {
        let conn = self.conn();
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM note ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter()
            .map(|id| note(&conn, id, Visibility::All))
            .collect()
    }

    /// Returns the most recent visible notes linking to this title or alias (of a page or a
    /// stub), newest first. A page that is not visible counts as a stub: only the notes linking
    /// to the title itself are returned, not those linking to its aliases.
    pub fn notes_about(
        &self,
        title: &str,
        limit: u32,
        visibility: Visibility<'_>,
    ) -> Result<Vec<Note>> {
        let conn = self.conn();
        let (_, keys) = names(&conn, title, visibility)?;
        let placeholders = vec!["?"; keys.len()].join(", ");
        let ids: Vec<i64> = conn
            .prepare(&format!(
                "SELECT DISTINCT note_id FROM note_link \
                 WHERE target_key IN ({placeholders}) AND {} \
                 ORDER BY note_id DESC LIMIT {limit}",
                visibility.notes("note_id")
            ))?
            .query_map(params_from_iter(&keys), |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter()
            .map(|id| note(&conn, id, visibility))
            .collect()
    }
}

/// Returns the note with this ID, if it exists and is visible.
pub(crate) fn find_note(
    conn: &Connection,
    note_id: i64,
    visibility: Visibility<'_>,
) -> Result<Option<Note>> {
    let exists = conn
        .query_row(
            &format!(
                "SELECT 1 FROM note WHERE id = ?1 AND {}",
                visibility.notes("id")
            ),
            [note_id],
            |_| Ok(()),
        )
        .optional()?;
    exists.map(|()| note(conn, note_id, visibility)).transpose()
}

/// The note with this ID; its links lead to pages only if they are visible, otherwise to stubs.
fn note(conn: &Connection, note_id: i64, visibility: Visibility<'_>) -> Result<Note> {
    let (body, created_at, author, spheres) = conn.query_row(
        "SELECT body, created_at, author, \
         (SELECT group_concat(sphere, char(10)) FROM note_sphere WHERE note_id = note.id) \
         FROM note WHERE id = ?1",
        [note_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get(3)?,
            ))
        },
    )?;
    let links = conn
        .prepare(&format!(
            "SELECT l.target_title, t.title FROM note_link l \
             LEFT JOIN ({}) k ON k.key = l.target_key \
             LEFT JOIN page t ON t.id = k.page_id \
             WHERE l.note_id = ?1 ORDER BY l.position",
            visibility.resolved_keys()
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
        author: author.as_deref().and_then(Author::parse),
        links,
        spheres: split_spheres(spheres),
    })
}
