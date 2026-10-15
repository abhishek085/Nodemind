//! The mind map's storage: nodes and edges.
//!
//! Nodes are the items people said (idea / learning / question / decision,
//! one node each), the entities those are about (topic / person / goal,
//! fuzzy-merged by vector so "bank call" and "call the bank" are one node),
//! and fog patterns (one hub per kind).

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

use crate::error::Result;
use crate::store::db::{GraphEdge, GraphNode};
use crate::store::vec_;

/// Cosine threshold above which two entity labels are considered the same.
const MERGE_SIMILARITY: f32 = 0.80;

/// A node plus everything the UI needs to render and act on it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub mentions: i64,
    pub sentiment: String,
    /// RFC 3339; for item nodes, when it was said.
    pub last_seen: Option<String>,
    pub degree: i64,
    /// For item nodes: the recording it came from.
    pub thought_id: Option<String>,
}


#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphData {
    pub nodes: Vec<NodeView>,
    pub edges: Vec<GraphEdge>,
}

fn slug(label: &str) -> String {
    label
        .trim()
        .to_lowercase()
        .replace('&', " and ")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-")
}

fn node_id(kind: &str, label: &str) -> String {
    format!("{kind}-{}", slug(label))
}

fn row_to_node(row: &rusqlite::Row) -> SqlResult<GraphNode> {
    Ok(GraphNode {
        id: row.get(0)?,
        node_type: row.get(1)?,
        label: row.get(2)?,
        mentions: row.get(3)?,
        last_seen: row.get(4)?,
        sentiment: row.get(5)?,
    })
}

pub fn get_nodes(conn: &Connection) -> Result<Vec<GraphNode>> {
    let mut stmt = conn
        .prepare("SELECT id, node_type, label, mentions, last_seen, sentiment FROM graph_nodes")
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], row_to_node)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    rows.collect::<SqlResult<Vec<_>>>()
        .map_err(|e| crate::error::Error::Store(e.to_string()))
}

/// Find an existing node whose label is near-identical (case-insensitive
/// exact match first, then vector similarity).
fn resolve_existing(
    conn: &Connection,
    embed_model: &str,
    kind: &str,
    label: &str,
    label_vec: Option<&[f32]>,
) -> Option<GraphNode> {
    // 1) exact, case-insensitive
    let mut stmt = conn
        .prepare("SELECT id, node_type, label, mentions, last_seen, sentiment FROM graph_nodes WHERE node_type = ?1 AND label = ?2 COLLATE NOCASE")
        .map_err(|e| crate::error::Error::Store(e.to_string()))
        .ok()?;
    let row: Option<GraphNode> = stmt
        .query_row(params![kind, label], row_to_node)
        .optional()
        .ok()?;
    if row.is_some() {
        return row;
    }
    // 2) vector similarity within the same kind. Note: `ref_id` in the vector
    //    index stores the node id, so the hit *is* the node id.
    if let Some(v) = label_vec {
        if let Ok(hits) = vec_::knn(conn, embed_model, Some(kind), v, 3) {
            if let Some(top) = hits.first().filter(|h| h.score >= MERGE_SIMILARITY as f64) {
                let mut stmt2 = conn
                    .prepare("SELECT id, node_type, label, mentions, last_seen, sentiment FROM graph_nodes WHERE id = ?1")
                    .ok()?;
                if let Ok(Some(n)) = stmt2.query_row(params![top.ref_id], row_to_node).optional()
                {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Insert or merge an entity node. Returns the (possibly existing) node id.
pub fn upsert_entity(
    conn: &Connection,
    embed_model: &str,
    kind: &str,
    label: &str,
    sentiment: &str,
    label_vec: Option<&[f32]>,
) -> Result<Option<String>> {
    let label = label.trim();
    if label.len() < 2 {
        return Ok(None);
    }
    let now = crate::time::now_secs().to_string();

    if let Some(existing) = resolve_existing(conn, embed_model, kind, label, label_vec) {
        conn.execute(
            "UPDATE graph_nodes SET mentions = mentions + 1, last_seen = ?2, sentiment = ?3 WHERE id = ?1",
            params![existing.id, now, sentiment],
        )
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        if let Some(v) = label_vec {
            let _ = vec_::insert(conn, kind, &existing.id, embed_model, v);
        }
        return Ok(Some(existing.id));
    }

    let id = node_id(kind, label);
    conn.execute(
        "INSERT OR IGNORE INTO graph_nodes (id, node_type, label, mentions, last_seen, sentiment)
         VALUES (?1, ?2, ?3, 1, ?4, ?5)",
        params![id, kind, label, now, sentiment],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    if let Some(v) = label_vec {
        let _ = vec_::insert(conn, kind, &id, embed_model, v);
    }
    Ok(Some(id))
}

/// Insert an item node (idea / learning / question / decision), keyed by
/// the item's id. Items are never merged by label — each is something said.
pub fn upsert_item_node(conn: &Connection, item_id: &str, kind: &str, text: &str, said_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO graph_nodes (id, node_type, label, mentions, last_seen, sentiment)
         VALUES (?1, ?2, ?3, 1, ?4, 'neutral')
         ON CONFLICT(id) DO UPDATE SET label = excluded.label",
        params![item_id, kind, text.trim(), said_at],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

/// Add (or strengthen) an edge.
pub fn upsert_edge(conn: &Connection, from: &str, to: &str, edge_type: &str) -> Result<()> {
    if from == to || from.is_empty() || to.is_empty() {
        return Ok(());
    }
    let id = format!("{from}|{to}|{edge_type}");
    conn.execute(
        "INSERT INTO graph_edges (id, from_node, to_node, edge_type, weight)
         VALUES (?1, ?2, ?3, ?4, 1.0)
         ON CONFLICT(id) DO UPDATE SET weight = MIN(10.0, weight + 0.5)",
        params![id, from, to, edge_type],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

/// Insert or overwrite an edge with an explicit weight (computed links,
/// e.g. EmbeddingGemma similarity — unlike `upsert_edge`, which counts).
pub fn upsert_weighted_edge(conn: &Connection, from: &str, to: &str, edge_type: &str, weight: f64) -> Result<()> {
    if from == to || from.is_empty() || to.is_empty() {
        return Ok(());
    }
    // Undirected link types are stored once, in a stable order.
    let (a, b) = if edge_type == "similar" && to < from { (to, from) } else { (from, to) };
    conn.execute(
        "INSERT INTO graph_edges (id, from_node, to_node, edge_type, weight)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET weight = MAX(weight, excluded.weight)",
        params![format!("{a}|{b}|{edge_type}"), a, b, edge_type, weight.clamp(0.1, 10.0)],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

/// Delete a node, its edges, and its vector.
pub fn delete_node(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM graph_nodes WHERE id = ?1", params![id])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    conn.execute(
        "DELETE FROM graph_edges WHERE from_node = ?1 OR to_node = ?1",
        params![id],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    if let Some(kind) = id.split('-').next() {
        let _ = vec_::delete_ref(conn, kind, id);
    }
    Ok(())
}

/// Full graph for the UI: all nodes with degree + edges, newest first.
pub fn get_graph(conn: &Connection, limit: i64) -> Result<GraphData> {
    let nodes = get_nodes(conn)?;
    let edges: Vec<GraphEdge> = {
        let mut stmt = conn
            .prepare("SELECT id, from_node, to_node, edge_type, weight FROM graph_edges")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(GraphEdge {
                    id: r.get(0)?,
                    from_node: r.get(1)?,
                    to_node: r.get(2)?,
                    edge_type: r.get(3)?,
                    weight: r.get(4)?,
                })
            })
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|e| crate::error::Error::Store(e.to_string()))?
    };

    let degree: std::collections::HashMap<&str, i64> = edges
        .iter()
        .flat_map(|e| vec![e.from_node.as_str(), e.to_node.as_str()])
        .fold(std::collections::HashMap::new(), |mut m, id| {
            *m.entry(id).or_insert(0) += 1;
            m
        });

    // Items know which recording they came from.
    let mut sources: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, thought_id FROM items")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        sources.extend(rows.flatten());
    }
    let mut out: Vec<NodeView> = nodes
        .iter()
        .map(|n| NodeView {
            id: n.id.clone(),
            node_type: n.node_type.clone(),
            label: n.label.clone(),
            mentions: n.mentions,
            sentiment: n.sentiment.clone(),
            last_seen: n.last_seen.as_deref().map(rfc3339),
            degree: degree.get(n.id.as_str()).copied().unwrap_or(0),
            thought_id: sources.get(&n.id).cloned(),
        })
        .collect();

    out.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    out.truncate(limit as usize);
    let keep: std::collections::HashSet<&str> =
        out.iter().map(|n| n.id.as_str()).collect();
    let edges = edges
        .into_iter()
        .filter(|e| keep.contains(e.from_node.as_str()) && keep.contains(e.to_node.as_str()))
        .collect();

    Ok(GraphData { nodes: out, edges })
}

/// Ids of every node that has at least one edge.
pub fn all_edge_endpoints(conn: &Connection) -> Result<std::collections::HashSet<String>> {
    let mut stmt = conn
        .prepare("SELECT from_node FROM graph_edges UNION SELECT to_node FROM graph_edges")
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// Remove the item nodes of a recording (and their edges).
pub fn unrecord_items(conn: &Connection, item_ids: &[String]) -> Result<()> {
    for id in item_ids {
        delete_node(conn, id)?;
    }
    Ok(())
}

/// Older builds stored `last_seen` as unix seconds; the UI wants RFC 3339.
fn rfc3339(v: &str) -> String {
    match v.parse::<i64>() {
        Ok(secs) => chrono::DateTime::from_timestamp(secs, 0).map(|d| d.to_rfc3339()).unwrap_or_else(|| v.to_string()),
        Err(_) => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        super::super::db::init_schema(&mut c).unwrap();
        c
    }

    #[test]
    fn upsert_merges_case_insensitively() {
        let c = conn();
        let a = upsert_entity(&c, "m", "topic", "Bank Call", "neutral", None).unwrap();
        let b = upsert_entity(&c, "m", "topic", "bank call", "neutral", None).unwrap();
        assert_eq!(a, b);
        let nodes = get_nodes(&c).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].mentions, 2);
    }

    #[test]
    fn edges_dedup_and_strengthen() {
        let c = conn();
        upsert_entity(&c, "m", "topic", "Alpha", "neutral", None).unwrap();
        upsert_entity(&c, "m", "topic", "Beta", "neutral", None).unwrap();
        upsert_edge(&c, "topic-alpha", "topic-beta", "relates_to").unwrap();
        upsert_edge(&c, "topic-alpha", "topic-beta", "relates_to").unwrap();
        let g = get_graph(&c, 50).unwrap();
        assert_eq!(g.edges.len(), 1);
        assert!((g.edges[0].weight - 1.5).abs() < 1e-9);
    }
}
