//! Deletion, which only the owner may do. Agents can never delete (there is no MCP tool).

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};

use crate::search::Item;
use crate::{Error, Knowledge, Result, current_page, increase_change_counter, resolve};

/// What deleting a page removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedPage {
    pub title: String,
    pub aliases: Vec<String>,
    pub revisions: u32,
}

impl Knowledge {
    /// Permanently deletes the page with this title or alias, with all its revisions and
    /// aliases. Links pointing to it stay in the linking pages and notes, so its title (and
    /// aliases) become stubs.
    pub fn delete_page(&self, title: &str) -> Result<DeletedPage> {
        let mut conn = self.conn();
        let tx = Transaction::new(&mut conn, TransactionBehavior::Immediate)?;
        let Some(page_id) = resolve(&tx, &crate::title::title_key(title))? else {
            return Err(Error::PageNotFound(title.trim().to_owned()));
        };
        let page = current_page(&tx, page_id)?;
        // Revisions, aliases and outgoing links go with the page (ON DELETE CASCADE).
        tx.execute("DELETE FROM page WHERE id = ?1", [page_id])?;
        increase_change_counter(&tx)?;
        tx.commit()?;
        self.index_after_commit(&conn, &[(Item::Page(page_id), None)]);
        Ok(DeletedPage {
            title: page.title,
            aliases: page.aliases,
            revisions: page.revision,
        })
    }

    /// Permanently deletes a note.
    pub fn delete_note(&self, note_id: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = Transaction::new(&mut conn, TransactionBehavior::Immediate)?;
        let exists = tx
            .query_row("SELECT 1 FROM note WHERE id = ?1", [note_id], |_| Ok(()))
            .optional()?;
        if exists.is_none() {
            return Err(Error::NoteNotFound(note_id));
        }
        tx.execute("DELETE FROM note WHERE id = ?1", [note_id])?;
        increase_change_counter(&tx)?;
        tx.commit()?;
        self.index_after_commit(&conn, &[(Item::Note(note_id), None)]);
        Ok(())
    }
}
