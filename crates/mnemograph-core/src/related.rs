//! A page's link neighborhood.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{Connection, params};

use crate::{Error, Knowledge, Result, Visibility, names, page_spheres, split_spheres};

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
    /// The page's spheres, sorted; empty for a stub.
    pub spheres: Vec<String>,
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

/// A page or stub in the whole-wiki graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub title: String,
    pub page_type: Option<String>,
    /// True if no page has this title yet.
    pub stub: bool,
    /// Number of other nodes in the graph it is linked with, in either direction.
    pub degree: u32,
    /// The page's spheres, sorted; empty for a stub.
    pub spheres: Vec<String>,
}

/// The whole wiki as a graph, or the part of it that fits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiGraph {
    /// Most-linked first, then by title.
    pub nodes: Vec<GraphNode>,
    /// Links between nodes as index pairs into `nodes`, regardless of direction, each pair once
    /// with the smaller index first.
    pub edges: Vec<(usize, usize)>,
    /// How many pages and stubs matched but were left out to stay within the limit.
    pub hidden: usize,
}

impl Knowledge {
    /// Returns every visible page, including unlinked ones, and every stub, with the links
    /// between them; as everywhere, a page that is not visible is a stub. If `page_types` is
    /// not empty, only pages of those types are included, and no stubs (they have no type). Of
    /// more than `max_nodes` matches, only the most-linked are returned.
    pub fn wiki_graph(
        &self,
        page_types: &[String],
        max_nodes: usize,
        visibility: Visibility<'_>,
    ) -> Result<WikiGraph> {
        let conn = self.conn();
        let mut nodes: HashMap<String, (String, Option<String>, bool, Vec<String>)> =
            HashMap::new();
        let mut pages = conn.prepare(&format!(
            "SELECT p.id, p.title, p.page_type, r.spheres FROM page p \
             JOIN revision r ON r.page_id = p.id \
             WHERE r.number = (SELECT max(number) FROM revision WHERE page_id = p.id) AND {}",
            visibility.pages("p.id")
        ))?;
        let mut rows = pages.query([])?;
        while let Some(row) = rows.next()? {
            let page_type: Option<String> = row.get(2)?;
            if page_types.is_empty() || page_type.as_ref().is_some_and(|t| page_types.contains(t)) {
                nodes.insert(
                    format!("p:{}", row.get::<_, i64>(0)?),
                    (row.get(1)?, page_type, false, split_spheres(row.get(3)?)),
                );
            }
        }
        if page_types.is_empty() {
            // A stub shows as the first visible link to it wrote it, as in `related`.
            let mut stubs = conn.prepare(&format!(
                "SELECT l.target_key, l.target_title FROM link l \
                 WHERE l.target_key NOT IN (SELECT key FROM ({})) AND {} \
                 ORDER BY l.rowid",
                visibility.resolved_keys(),
                visibility.pages("l.source_page_id")
            ))?;
            let mut rows = stubs.query([])?;
            while let Some(row) = rows.next()? {
                nodes
                    .entry(format!("s:{}", row.get::<_, String>(0)?))
                    .or_insert((row.get(1)?, None, true, Vec::new()));
            }
        }

        let mut pairs = BTreeSet::new();
        let mut query = conn.prepare(&format!(
            "WITH {} SELECT a, b FROM edge WHERE a <> b",
            edges(visibility)
        ))?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let (a, b): (String, String) = (row.get(0)?, row.get(1)?);
            if nodes.contains_key(&a) && nodes.contains_key(&b) {
                pairs.insert(if a < b { (a, b) } else { (b, a) });
            }
        }
        let mut degree: HashMap<&str, u32> = HashMap::new();
        for (a, b) in &pairs {
            *degree.entry(a).or_default() += 1;
            *degree.entry(b).or_default() += 1;
        }

        let mut ranked: Vec<_> = nodes
            .iter()
            .map(|(id, (title, page_type, stub, spheres))| {
                let node = GraphNode {
                    title: title.clone(),
                    page_type: page_type.clone(),
                    stub: *stub,
                    degree: degree.get(id.as_str()).copied().unwrap_or(0),
                    spheres: spheres.clone(),
                };
                (id.as_str(), node)
            })
            .collect();
        ranked.sort_by_cached_key(|(_, n)| (std::cmp::Reverse(n.degree), n.title.to_lowercase()));
        let hidden = ranked.len().saturating_sub(max_nodes);
        ranked.truncate(max_nodes);
        let index: HashMap<&str, usize> = ranked
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();
        let mut edges: Vec<_> = pairs
            .iter()
            .filter_map(|(a, b)| Some((*index.get(a.as_str())?, *index.get(b.as_str())?)))
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        edges.sort_unstable();
        Ok(WikiGraph {
            nodes: ranked.into_iter().map(|(_, node)| node).collect(),
            edges,
            hidden,
        })
    }

    /// Returns the neighborhood of this title or alias: the same pages and stubs as
    /// [`Knowledge::related`] plus the title itself, and the links between them.
    pub fn neighborhood(
        &self,
        title: &str,
        hops: u32,
        limit: u32,
        visibility: Visibility<'_>,
    ) -> Result<Neighborhood> {
        let conn = self.conn();
        let (page_id, keys) = names(&conn, title, visibility)?;
        let start = match page_id {
            Some(id) => format!("p:{id}"),
            None => format!("s:{}", keys[0]),
        };
        let found = walk(&conn, &start, hops.clamp(1, MAX_HOPS), limit, visibility)?;
        if page_id.is_none() && found.is_empty() {
            return Err(Error::PageNotFound(title.trim().to_owned()));
        }
        let mut found = found
            .into_iter()
            .map(|(node, distance)| Ok((describe(&conn, &node, distance, visibility)?, node)))
            .collect::<Result<Vec<_>>>()?;
        found.sort_by_cached_key(|(r, _)| (r.distance, r.title.to_lowercase()));
        let mut nodes = vec![describe(&conn, &start, 0, visibility)?];
        let mut index = HashMap::from([(start, 0)]);
        for (related, node) in found {
            index.insert(node, nodes.len());
            nodes.push(related);
        }

        let mut query = conn.prepare(&format!(
            "WITH {} SELECT a, b FROM edge WHERE a <> b",
            edges(visibility)
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
    /// most `limit` entries. Only links of visible pages are followed; a page that is not
    /// visible is a stub.
    pub fn related(
        &self,
        title: &str,
        hops: u32,
        limit: u32,
        visibility: Visibility<'_>,
    ) -> Result<Vec<Related>> {
        let conn = self.conn();
        let (page_id, keys) = names(&conn, title, visibility)?;
        let start = match page_id {
            Some(id) => format!("p:{id}"),
            None => format!("s:{}", keys[0]),
        };
        let found = walk(&conn, &start, hops.clamp(1, MAX_HOPS), limit, visibility)?;
        if page_id.is_none() && found.is_empty() {
            return Err(Error::PageNotFound(title.trim().to_owned()));
        }
        let mut related = found
            .into_iter()
            .map(|(node, distance)| describe(&conn, &node, distance, visibility))
            .collect::<Result<Vec<_>>>()?;
        related.sort_by_cached_key(|r| (r.distance, r.title.to_lowercase()));
        Ok(related)
    }
}

/// The link graph as `edge (a, b)` from linking to linked node, for a `WITH` clause: the links
/// of visible pages, where a link to a page that is not visible leads to a stub.
fn edges(visibility: Visibility<'_>) -> String {
    format!(
        "edge (a, b) AS ( \
           SELECT 'p:' || l.source_page_id, coalesce('p:' || k.page_id, 's:' || l.target_key) \
           FROM link l LEFT JOIN ({}) k ON k.key = l.target_key WHERE {})",
        visibility.resolved_keys(),
        visibility.pages("l.source_page_id")
    )
}

/// Walks the undirected link graph from `start`. Nodes are `p:<page id>` for pages and
/// `s:<title key>` for stubs, so links to a page under any title or alias meet in one node.
fn walk(
    conn: &Connection,
    start: &str,
    hops: u32,
    limit: u32,
    visibility: Visibility<'_>,
) -> Result<Vec<(String, u32)>> {
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
        edges(visibility)
    ))?;
    let found = query
        .query_map(params![start, hops, limit], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(found)
}

fn describe(
    conn: &Connection,
    node: &str,
    distance: u32,
    visibility: Visibility<'_>,
) -> Result<Related> {
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
            spheres: page_spheres(conn, id.parse().expect("page IDs are numbers"))?,
        })
    } else {
        let key = node.strip_prefix("s:").expect("node is a page or a stub");
        // A stub has no title of its own; show it as the first visible link to it wrote it.
        let title = conn.query_row(
            &format!(
                "SELECT target_title FROM link WHERE target_key = ?1 AND {} \
                 ORDER BY rowid LIMIT 1",
                visibility.pages("source_page_id")
            ),
            [key],
            |row| row.get(0),
        )?;
        Ok(Related {
            title,
            page_type: None,
            stub: true,
            distance,
            spheres: Vec::new(),
        })
    }
}
