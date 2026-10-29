//! A page's link neighborhood.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{Connection, params};

use crate::{Error, Knowledge, RESOLVED_KEYS, Result, names};

/// The most hops [`Knowledge::related`] walks.
pub const MAX_HOPS: u32 = 3;

/// A page or stub near another one in the link graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    pub title: String,
    pub page_type: Option<String>,
    /// True if no page has this title yet.
    pub stub: bool,
    /// Number of links between the two, following links in either direction.
    pub distance: u32,
}

/// A page or stub with the pages and stubs around it, and the links between all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighborhood {
    /// The page or stub itself first (distance 0), then the others as [`Knowledge::related`]
    /// reports them.
    pub nodes: Vec<Related>,
    /// Links between nodes as index pairs into `nodes`, regardless of direction, each pair once
    /// with the smaller index first.
    pub edges: Vec<(usize, usize)>,
}

impl Knowledge {
    /// Returns the neighborhood of this title or alias: the same pages and stubs as
    /// [`Knowledge::related`] plus the title itself, and the links between them.
    pub fn neighborhood(&self, title: &str, hops: u32, limit: u32) -> Result<Neighborhood> {
        let conn = self.conn();
        let (page_id, keys) = names(&conn, title)?;
        let start = match page_id {
            Some(id) => format!("p:{id}"),
            None => format!("s:{}", keys[0]),
        };
        let found = walk(&conn, &start, hops.clamp(1, MAX_HOPS), limit)?;
        if page_id.is_none() && found.is_empty() {
            return Err(Error::PageNotFound(title.trim().to_owned()));
        }
        let mut found = found
            .into_iter()
            .map(|(node, distance)| Ok((describe(&conn, &node, distance)?, node)))
            .collect::<Result<Vec<_>>>()?;
        found.sort_by_cached_key(|(r, _)| (r.distance, r.title.to_lowercase()));
        let mut nodes = vec![describe(&conn, &start, 0)?];
        let mut index = HashMap::from([(start, 0)]);
        for (related, node) in found {
            index.insert(node, nodes.len());
            nodes.push(related);
        }

        let mut query = conn.prepare(&format!(
            "WITH {} SELECT a, b FROM edge WHERE a <> b",
            edges()
        ))?;
        let mut edges = BTreeSet::new();
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let (a, b): (String, String) = (row.get(0)?, row.get(1)?);
            if let (Some(&a), Some(&b)) = (index.get(&a), index.get(&b)) {
                edges.insert((a.min(b), a.max(b)));
            }
        }
        Ok(Neighborhood {
            nodes,
            edges: edges.into_iter().collect(),
        })
    }

    /// Returns the pages and stubs within `hops` links of this title or alias (clamped to
    /// 1..=[`MAX_HOPS`]), following links in both directions; nearest first, then by title, at
    /// most `limit` entries.
    pub fn related(&self, title: &str, hops: u32, limit: u32) -> Result<Vec<Related>> {
        let conn = self.conn();
        let (page_id, keys) = names(&conn, title)?;
        let start = match page_id {
            Some(id) => format!("p:{id}"),
            None => format!("s:{}", keys[0]),
        };
        let found = walk(&conn, &start, hops.clamp(1, MAX_HOPS), limit)?;
        if page_id.is_none() && found.is_empty() {
            return Err(Error::PageNotFound(title.trim().to_owned()));
        }
        let mut related = found
            .into_iter()
            .map(|(node, distance)| describe(&conn, &node, distance))
            .collect::<Result<Vec<_>>>()?;
        related.sort_by_cached_key(|r| (r.distance, r.title.to_lowercase()));
        Ok(related)
    }
}

/// The link graph as `edge (a, b)` from linking to linked node, for a `WITH` clause.
fn edges() -> String {
    format!(
        "edge (a, b) AS ( \
           SELECT 'p:' || l.source_page_id, coalesce('p:' || k.page_id, 's:' || l.target_key) \
           FROM link l LEFT JOIN ({RESOLVED_KEYS}) k ON k.key = l.target_key)"
    )
}

/// Walks the undirected link graph from `start`. Nodes are `p:<page id>` for pages and
/// `s:<title key>` for stubs, so links to a page under any title or alias meet in one node.
fn walk(conn: &Connection, start: &str, hops: u32, limit: u32) -> Result<Vec<(String, u32)>> {
    let mut query = conn.prepare(&format!(
        "WITH RECURSIVE {}, \
           neighbor (a, b) AS ( \
             SELECT a, b FROM edge WHERE a <> b UNION SELECT b, a FROM edge WHERE a <> b), \
           reached (node, distance) AS ( \
             SELECT ?1, 0 \
             UNION \
             SELECT n.b, r.distance + 1 FROM reached r JOIN neighbor n ON n.a = r.node \
             WHERE r.distance < ?2) \
         SELECT node, min(distance) AS distance FROM reached WHERE node <> ?1 \
         GROUP BY node ORDER BY distance LIMIT ?3",
        edges()
    ))?;
    let found = query
        .query_map(params![start, hops, limit], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(found)
}

fn describe(conn: &Connection, node: &str, distance: u32) -> Result<Related> {
    if let Some(id) = node.strip_prefix("p:") {
        let (title, page_type) = conn.query_row(
            "SELECT title, page_type FROM page WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(Related {
            title,
            page_type,
            stub: false,
            distance,
        })
    } else {
        let key = node.strip_prefix("s:").expect("node is a page or a stub");
        // A stub has no title of its own; show it as the first link to it wrote it.
        let title = conn.query_row(
            "SELECT target_title FROM link WHERE target_key = ?1 ORDER BY rowid LIMIT 1",
            [key],
            |row| row.get(0),
        )?;
        Ok(Related {
            title,
            page_type: None,
            stub: true,
            distance,
        })
    }
}
