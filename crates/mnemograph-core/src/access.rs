//! The access log: what agents read, searched and explored, kept for a while for the owner's
//! statistics.

use crate::stats::weekly_per_author;
use rusqlite::params;

use crate::{Author, Knowledge, Result, Spheres, Visibility, WeeklyActivity, title_key};

/// What an agent did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Read a page (or tried to: then there are no hits).
    ReadPage,
    Search,
    /// Explored the pages around a title.
    Related,
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Access::ReadPage => "read_page",
            Access::Search => "search",
            Access::Related => "related",
        }
    }
}

/// How much one agent read and searched within the log's retention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAccess {
    pub author: Option<Author>,
    pub reads: u32,
    pub searches: u32,
    pub related: u32,
    /// When the agent last did any of these.
    pub last: String,
}

/// A page and how often agents read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageReads {
    /// The page's current title.
    pub title: String,
    pub reads: u32,
}

/// Search terms that found nothing: something agents looked for that the wiki does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeGap {
    /// The terms as last searched.
    pub terms: String,
    /// How often the same terms (ignoring case and spacing) found nothing.
    pub times: u32,
    /// Who searched them last, and when.
    pub author: Option<Author>,
    pub last: String,
}

/// Access log entries as events for [`weekly_per_author`].
const ACCESSES: &str = "event (author, created_at) AS (SELECT author, at FROM access)";

impl Knowledge {
    /// Records that `author`, through a registration with `spheres`, read, searched or
    /// explored `target` (a title, or search terms) with `hits` results.
    pub fn record_access(
        &self,
        author: &Author,
        spheres: &Spheres,
        access: Access,
        target: &str,
        hits: u32,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO access (author, tool, target, target_key, hits) VALUES (?1, ?2, ?3, ?4, ?5)",
            (author.to_string(), access.as_str(), target.trim(), title_key(target), hits),
        )?;
        let access_id = tx.last_insert_rowid();
        for sphere in spheres.names() {
            tx.execute(
                "INSERT INTO access_sphere (access_id, sphere) VALUES (?1, ?2)",
                params![access_id, sphere],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes access log entries older than `days` days and returns how many.
    pub fn prune_access_log(&self, days: u32) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM access WHERE at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [format!("-{days} days")],
        )?)
    }

    /// Per agent, how often it read, searched and explored; most active first.
    pub fn agent_access(&self) -> Result<Vec<AgentAccess>> {
        let conn = self.conn();
        let mut query = conn.prepare(
            "SELECT author, sum(tool = 'read_page'), sum(tool = 'search'), sum(tool = 'related'), \
               max(at) \
             FROM access GROUP BY author ORDER BY count(*) DESC, max(at) DESC",
        )?;
        let access = query
            .query_map([], |row| {
                let author: String = row.get(0)?;
                Ok(AgentAccess {
                    author: Author::parse(&author),
                    reads: row.get(1)?,
                    searches: row.get(2)?,
                    related: row.get(3)?,
                    last: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(access)
    }

    /// Reads, searches and explorations per week and agent, up to the current week.
    pub fn weekly_access(&self) -> Result<WeeklyActivity> {
        weekly_per_author(&self.conn(), ACCESSES, "now")
    }

    /// The pages agents read most, under their current titles (reads under an alias or an
    /// earlier title count for the page); pages deleted since are left out. With
    /// [`Visibility::Within`], only visible pages and reads through registrations sharing a
    /// sphere count.
    pub fn most_read_pages(
        &self,
        limit: usize,
        visibility: Visibility<'_>,
    ) -> Result<Vec<PageReads>> {
        let conn = self.conn();
        let mut query = conn.prepare(&format!(
            "SELECT p.title, count(*) FROM access a \
             JOIN ({}) k ON k.key = a.target_key JOIN page p ON p.id = k.page_id \
             WHERE a.tool = 'read_page' AND a.hits > 0 AND {} \
             GROUP BY p.id ORDER BY count(*) DESC, max(a.at) DESC LIMIT ?1",
            visibility.resolved_keys(),
            visibility.accesses("a.id")
        ))?;
        let pages = query
            .query_map([limit as i64], |row| {
                Ok(PageReads {
                    title: row.get(0)?,
                    reads: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(pages)
    }

    /// Searches that found nothing, the most recent first; the same terms (ignoring case and
    /// spacing) count as one. With [`Visibility::Within`], only searches through registrations
    /// sharing a sphere count.
    pub fn knowledge_gaps(
        &self,
        limit: usize,
        visibility: Visibility<'_>,
    ) -> Result<Vec<KnowledgeGap>> {
        let conn = self.conn();
        // The latest search of each terms supplies how they were written, by whom and when.
        let mut query = conn.prepare(&format!(
            "SELECT a.target, g.times, a.author, a.at FROM \
               (SELECT target_key, count(*) AS times, max(id) AS latest FROM access \
                WHERE tool = 'search' AND hits = 0 AND {} GROUP BY target_key) g \
             JOIN access a ON a.id = g.latest ORDER BY a.at DESC, a.id DESC LIMIT ?1",
            visibility.accesses("id")
        ))?;
        let gaps = query
            .query_map([limit as i64], |row| {
                let author: String = row.get(2)?;
                Ok(KnowledgeGap {
                    terms: row.get(0)?,
                    times: row.get(1)?,
                    author: Author::parse(&author),
                    last: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(gaps)
    }
}
