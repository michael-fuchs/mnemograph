//! Statistics about the knowledge, for the owner's dashboard.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::{Author, GraphNode, Knowledge, RESOLVED_KEYS, Result};

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

/// What one author wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorActivity {
    /// `None` for revisions and notes written before authors were recorded.
    pub author: Option<Author>,
    pub revisions: u32,
    pub notes: u32,
    /// When the author's first and latest revision or note was written.
    pub first: String,
    pub last: String,
}

/// Revisions plus notes written per week, per author.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeeklyActivity {
    /// The Mondays the weeks start on, `YYYY-MM-DD`, from the first write to the current week.
    pub weeks: Vec<String>,
    /// Each author with one count per week, in the order of [`Knowledge::author_activity`].
    pub authors: Vec<(Option<Author>, Vec<u32>)>,
}

/// Revisions and notes with their author and time, for a `WITH` clause.
const WRITES: &str = "event (author, created_at, revision, note) AS ( \
       SELECT author, created_at, 1, 0 FROM revision \
       UNION ALL SELECT author, created_at, 0, 1 FROM note)";

impl Knowledge {
    /// Everyone who wrote revisions or notes, most writes first, then by latest write.
    pub fn author_activity(&self) -> Result<Vec<AuthorActivity>> {
        let conn = self.conn();
        let mut query = conn.prepare(&format!(
            "WITH {WRITES} \
             SELECT author, sum(revision), sum(note), min(created_at), max(created_at) \
             FROM event GROUP BY author \
             ORDER BY count(*) DESC, max(created_at) DESC"
        ))?;
        let activity = query
            .query_map([], |row| {
                let author: Option<String> = row.get(0)?;
                Ok(AuthorActivity {
                    author: author.as_deref().and_then(Author::parse),
                    revisions: row.get(1)?,
                    notes: row.get(2)?,
                    first: row.get(3)?,
                    last: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(activity)
    }

    /// Revisions plus notes per week and author, from the week of the first write to the
    /// current week; empty if nothing was written.
    pub fn weekly_activity(&self) -> Result<WeeklyActivity> {
        self.weekly_activity_until("now")
    }

    /// [`Knowledge::weekly_activity`] up to the week of `until` (an SQLite date such as `now`).
    pub(crate) fn weekly_activity_until(&self, until: &str) -> Result<WeeklyActivity> {
        weekly_per_author(&self.conn(), WRITES, until)
    }

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

/// Counts events per week and author. `events` defines `event (author, created_at, …)` for a
/// `WITH` clause. Weeks run from the first event's to the week of `until` (an SQLite date such
/// as `now`); authors come most events first, then by latest event.
pub(crate) fn weekly_per_author(
    conn: &Connection,
    events: &str,
    until: &str,
) -> Result<WeeklyActivity> {
    let mut weeks_query = conn.prepare(&format!(
        "WITH RECURSIVE {events}, \
           bound (first, last) AS ( \
             SELECT min(date(created_at, 'weekday 0', '-6 days')), \
                    max(max(date(created_at, 'weekday 0', '-6 days')), \
                        date(?1, 'weekday 0', '-6 days')) FROM event), \
           weeks (week) AS ( \
             SELECT first FROM bound \
             UNION ALL SELECT date(week, '+7 days') FROM weeks, bound WHERE week < bound.last) \
         SELECT week FROM weeks WHERE week IS NOT NULL"
    ))?;
    let weeks: Vec<String> = weeks_query
        .query_map([until], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if weeks.is_empty() {
        return Ok(WeeklyActivity::default());
    }
    let order: Vec<Option<String>> = conn
        .prepare(&format!(
            "WITH {events} SELECT author FROM event GROUP BY author \
             ORDER BY count(*) DESC, max(created_at) DESC"
        ))?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let position: HashMap<&str, usize> = weeks
        .iter()
        .enumerate()
        .map(|(i, w)| (w.as_str(), i))
        .collect();
    let row_of: HashMap<&Option<String>, usize> =
        order.iter().enumerate().map(|(i, a)| (a, i)).collect();
    let mut counts = vec![vec![0u32; weeks.len()]; order.len()];
    let mut query = conn.prepare(&format!(
        "WITH {events} \
         SELECT author, date(created_at, 'weekday 0', '-6 days'), count(*) FROM event \
         GROUP BY 1, 2"
    ))?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let author: Option<String> = row.get(0)?;
        let week: String = row.get(1)?;
        if let (Some(&a), Some(&w)) = (row_of.get(&author), position.get(week.as_str())) {
            counts[a][w] = row.get(2)?;
        }
    }
    Ok(WeeklyActivity {
        authors: order
            .iter()
            .map(|author| author.as_deref().and_then(Author::parse))
            .zip(counts)
            .collect(),
        weeks,
    })
}
