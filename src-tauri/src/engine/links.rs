//! Mind-map links, decided by EmbeddingGemma 2 rather than guessed by the LLM.
//!
//! - **similar** (item ↔ item): an idea / learning / question / decision
//!   links to the few earlier items that stand out as close in meaning, so the
//!   same theme connects across days. EmbeddingGemma's cosine runs high even
//!   for unrelated text (~0.6–0.8), so "close" means a lift above this item's
//!   *median* similarity, never a fixed number.
//! - **fog**: one hub per fog kind, so recurring overthinking shows as a hub.
//! - Junk entities ("you", "the speaker", "notes") never enter the graph.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::error::Result;
use crate::store::{graph, vec_};

const EMBED_MODEL: &str = "embeddinggemma-2";
/// How far above the median similarity a neighbour must be to link.
const MIN_LIFT: f64 = 0.06;
/// With fewer thoughts than this there is no stable median; require a high
/// absolute similarity instead.
const SMALL_HISTORY: usize = 6;
const SMALL_HISTORY_COSINE: f64 = 0.86;
const MAX_LINKS: usize = 3;

/// Entities that carry no meaning in a personal mind map.
pub fn is_junk_entity(kind: &str, label: &str) -> bool {
    let l = label.trim().to_lowercase();
    let words = l.split_whitespace().count();
    let generic: &[&str] = match kind {
        "person" => &[
            "you", "me", "i", "myself", "we", "us", "they", "them", "he", "she", "someone", "somebody",
            "everyone", "everybody", "guys", "hey guys", "narrator", "the narrator", "speaker", "the speaker",
            "user", "the user", "people", "team", "the team",
        ],
        "topic" => &[
            "notes", "note", "meeting", "meetings", "thoughts", "thought", "thinking", "tasks", "task", "things",
            "stuff", "work", "life", "today", "recording", "transcript", "brain-dump", "brain dump",
        ],
        "goal" => &["remind", "a task", "task", "tasks", "take notes", "notes", "do it", "get it done"],
        _ => &[],
    };
    l.len() < 2 || generic.contains(&l.as_str()) || (kind == "goal" && words < 2)
}

/// Link an item to its closest other items. Returns the linked ids.
pub fn link_similar(conn: &Connection, item_id: &str) -> Result<Vec<String>> {
    let vectors = item_vectors(conn)?;
    let Some(me) = vectors.get(item_id) else {
        return Ok(Vec::new());
    };
    let neighbours = pick_neighbours(me, vectors.iter().filter(|(id, _)| id.as_str() != item_id));
    for (id, lift) in &neighbours {
        graph::upsert_weighted_edge(conn, item_id, id, "similar", 1.0 + lift * 10.0)?;
    }
    Ok(neighbours.into_iter().map(|(id, _)| id).collect())
}

/// Recompute every "similar" link (startup), dropping link types older
/// builds made.
pub fn rebuild_similar(conn: &Connection) -> Result<usize> {
    conn.execute("DELETE FROM graph_edges WHERE edge_type IN ('similar', 'relates_to')", [])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let vectors = item_vectors(conn)?;
    let mut n = 0;
    for (id, v) in &vectors {
        for (other, lift) in pick_neighbours(v, vectors.iter().filter(|(o, _)| *o != id)) {
            graph::upsert_weighted_edge(conn, id, &other, "similar", 1.0 + lift * 10.0)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Remove junk entity nodes (and their edges) left by older builds.
pub fn prune_junk(conn: &Connection) -> Result<usize> {
    let junk: Vec<String> = graph::get_nodes(conn)?
        .into_iter()
        .filter(|n| matches!(n.node_type.as_str(), "topic" | "person" | "goal") && is_junk_entity(&n.node_type, &n.label))
        .map(|n| n.id)
        .collect();
    for id in &junk {
        graph::delete_node(conn, id)?;
    }
    Ok(junk.len())
}

/// Startup pass: prune junk entities and recompute similarity links.
pub fn refresh_in_background(state: std::sync::Arc<crate::state::AppState>) {
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(conn) = state.db.lock() else {
            return;
        };
        let pruned = prune_junk(&conn).unwrap_or(0);
        let linked = rebuild_similar(&conn).unwrap_or(0);
        drop(conn);
        crate::applog::write(&state, "app.log", &format!("graph refresh: {pruned} junk entities removed, {linked} similarity links"));
    });
}

fn item_vectors(conn: &Connection) -> Result<HashMap<String, Vec<f32>>> {
    Ok(vec_::all_vectors(conn, EMBED_MODEL, "item")?.into_iter().collect())
}

/// Neighbours that clear the bar, closest first: (id, lift over median).
fn pick_neighbours<'a>(
    me: &[f32],
    others: impl Iterator<Item = (&'a String, &'a Vec<f32>)>,
) -> Vec<(String, f64)> {
    let mut sims: Vec<(String, f64)> = others.map(|(id, v)| (id.clone(), cosine(me, v))).collect();
    if sims.is_empty() {
        return Vec::new();
    }
    sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let small = sims.len() < SMALL_HISTORY;
    let median = sims[sims.len() / 2].1;
    sims.into_iter()
        .filter(|(_, s)| if small { *s >= SMALL_HISTORY_COSINE } else { *s - median >= MIN_LIFT })
        .take(MAX_LINKS)
        .map(|(id, s)| (id, if small { s - SMALL_HISTORY_COSINE + MIN_LIFT } else { s - median }))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        (dot / (na * nb)) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_entities() {
        assert!(is_junk_entity("person", "the narrator"));
        assert!(is_junk_entity("person", "You"));
        assert!(!is_junk_entity("person", "Priya"));
        assert!(is_junk_entity("topic", "Meeting"));
        assert!(!is_junk_entity("topic", "beta launch"));
        assert!(is_junk_entity("goal", "remind"));
        assert!(!is_junk_entity("goal", "get fit this year"));
    }

    #[test]
    fn neighbours_must_stand_out_from_the_median() {
        // Six others: two clearly closer than the pack.
        let me = vec![1.0, 0.0];
        let ids: Vec<String> = (0..6).map(|i| format!("t{i}")).collect();
        let vecs: Vec<Vec<f32>> = vec![
            vec![0.99, 0.14], // 0.99
            vec![0.95, 0.31], // 0.95
            vec![0.80, 0.60], // 0.80
            vec![0.79, 0.61],
            vec![0.78, 0.63],
            vec![0.77, 0.64],
        ];
        let got = pick_neighbours(&me, ids.iter().zip(vecs.iter()));
        let got: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(got, vec!["t0", "t1"]);
    }

    #[test]
    fn small_history_needs_high_similarity() {
        let me = vec![1.0, 0.0];
        let ids = vec!["a".to_string(), "b".to_string()];
        let vecs = vec![vec![0.78, 0.63], vec![0.70, 0.71]]; // ~0.78, ~0.70
        assert!(pick_neighbours(&me, ids.iter().zip(vecs.iter())).is_empty());
    }
}
