//! Database schema and its migrations.

use rusqlite::Transaction;
use rusqlite_migration::{HookError, HookResult, M, Migrations};

use crate::title::title_key;

pub(crate) fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/0001-pages.sql")),
        M::up(include_str!("../migrations/0002-revisions.sql")),
        M::up(include_str!("../migrations/0003-links.sql")),
        M::up_with_hook(
            include_str!("../migrations/0004-titles-aliases-types.sql"),
            fill_title_keys_and_links,
        ),
        M::up(include_str!("../migrations/0005-notes.sql")),
        M::up(include_str!("../migrations/0006-change-counter.sql")),
        M::up(include_str!("../migrations/0007-authors.sql")),
        M::up(include_str!("../migrations/0008-sessions.sql")),
    ])
}

/// Fills the title keys of existing pages, makes them unique and extracts all links again.
///
/// Links are derived from the current revisions, so a migration that changes their shape
/// rebuilds them from the bodies rather than converting rows.
fn fill_title_keys_and_links(tx: &Transaction<'_>) -> HookResult {
    let pages: Vec<(i64, String, String)> = tx
        .prepare(
            "SELECT p.id, p.title, r.body FROM page p JOIN revision r ON r.page_id = p.id \
             WHERE r.number = (SELECT max(number) FROM revision WHERE page_id = p.id)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut keys: Vec<(String, &str)> = Vec::new();
    for (id, title, _) in &pages {
        let key = title_key(title);
        if let Some((_, other)) = keys.iter().find(|(k, _)| *k == key) {
            return Err(HookError::Hook(format!(
                "the pages {other:?} and {title:?} differ only in case or whitespace; rename one of them before upgrading"
            )));
        }
        tx.execute(
            "UPDATE page SET title_key = ?2 WHERE id = ?1",
            rusqlite::params![id, key],
        )?;
        keys.push((key, title));
    }
    tx.execute_batch("CREATE UNIQUE INDEX page_title_key ON page (title_key);")?;

    for (id, _, body) in &pages {
        crate::replace_links(tx, *id, body).map_err(|err| HookError::Hook(err.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrations_are_valid() {
        super::migrations().validate().unwrap();
    }
}
