//! Statistics about the knowledge, for the owner's dashboard.

use crate::{GraphNode, Knowledge, RESOLVED_KEYS, Result};

/// How much knowledge there is now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub pages: u32,
    /// Titles that links point to but no page has.
    pub stubs: u32,
    /// Links from pages and notes, not counting a page's links to itself.
    pub links: u32,
    pub notes: u32,
    pub revisions: u32,
}

/// How many pages and notes existed at the end of a week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Week {
    /// The Monday the week starts on, `YYYY-MM-DD`.
    pub start: String,
    pub pages: u32,
    pub notes: u32,
}

impl Knowledge {
    /// Counts pages, stubs, links, notes and revisions.
    pub fn totals(&self) -> Result<Totals> {
        let conn = self.conn();
        let count = |sql: &str| -> Result<u32> { Ok(conn.query_row(sql, [], |row| row.get(0))?) };
        Ok(Totals {
            pages: count("SELECT count(*) FROM page")?,
            stubs: count(&format!(
                "SELECT count(DISTINCT target_key) FROM \
                   (SELECT target_key FROM link UNION ALL SELECT target_key FROM note_link) \
                 WHERE target_key NOT IN (SELECT key FROM ({RESOLVED_KEYS}))"
            ))?,
            links: count(&format!(
                "SELECT count(*) FROM link l LEFT JOIN ({RESOLVED_KEYS}) k ON k.key = l.target_key \
                 WHERE k.page_id IS NOT l.source_page_id"
            ))? + count("SELECT count(*) FROM note_link")?,
            notes: count("SELECT count(*) FROM note")?,
            revisions: count("SELECT count(*) FROM revision")?,
        })
    }

    /// The number of pages and notes at the end of every week, from the week of the first one
    /// written to the current week; empty if there are none. Deleted pages and notes are not
    /// counted in any week.
    pub fn weekly_growth(&self) -> Result<Vec<Week>> {
        self.weekly_growth_until("now")
    }

    /// [`Knowledge::weekly_growth`] up to the week of `until` (an SQLite date such as `now`).
    pub(crate) fn weekly_growth_until(&self, until: &str) -> Result<Vec<Week>> {
        let conn = self.conn();
        // A week is named by its Monday: the date moved to the next Sunday (or kept, if it is
        // one), minus six days.
        let mut query = conn.prepare(
            "WITH RECURSIVE \
               item (week, page, note) AS ( \
                 SELECT date(created_at, 'weekday 0', '-6 days'), 1, 0 FROM page \
                 UNION ALL \
                 SELECT date(created_at, 'weekday 0', '-6 days'), 0, 1 FROM note), \
               last (until) AS (SELECT date(?1, 'weekday 0', '-6 days')), \
               weeks (week) AS ( \
                 SELECT min(week) FROM item \
                 UNION ALL \
                 SELECT date(weeks.week, '+7 days') FROM weeks, last \
                 WHERE weeks.week < max((SELECT max(week) FROM item), last.until)) \
             SELECT w.week, \
               (SELECT count(*) FROM item WHERE item.week <= w.week AND page = 1), \
               (SELECT count(*) FROM item WHERE item.week <= w.week AND note = 1) \
             FROM weeks w WHERE w.week IS NOT NULL ORDER BY w.week",
        )?;
        let weeks = query
            .query_map([until], |row| {
                Ok(Week {
                    start: row.get(0)?,
                    pages: row.get(1)?,
                    notes: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(weeks)
    }

    /// The pages linked with the most other pages and stubs, in either direction, most-linked
    /// first; at most `limit`, and only pages with links.
    pub fn most_linked_pages(&self, limit: usize) -> Result<Vec<GraphNode>> {
        let graph = self.wiki_graph(&[], usize::MAX)?;
        Ok(graph
            .nodes
            .into_iter()
            .filter(|node| !node.stub && node.degree > 0)
            .take(limit)
            .collect())
    }

    /// Pages without links to other pages or stubs and without links from other pages, by
    /// title. Notes linking to them do not count.
    pub fn orphaned_pages(&self) -> Result<Vec<GraphNode>> {
        let graph = self.wiki_graph(&[], usize::MAX)?;
        let mut orphans: Vec<_> = graph
            .nodes
            .into_iter()
            .filter(|node| !node.stub && node.degree == 0)
            .collect();
        orphans.sort_by_cached_key(|node| node.title.to_lowercase());
        Ok(orphans)
    }
}
