//! Search: "what did I say about…", over the things worth keeping.
//!
//! The index holds short, clean statements — items (ideas, learnings,
//! questions, decisions) and tasks — not raw transcripts, so results read
//! like answers instead of fragments of rambling. Each statement gets an
//! EmbeddingGemma 2 vector (document prefix); queries use the query prefix.
//!
//! Ranking is semantic first, with a keyword boost:
//!
//!   rank = (cosine − baseline) + KEYWORD_BOOST × (rarity-weighted share of query words present)
//!
//! EmbeddingGemma's cosine runs high even for unrelated text (~0.6) and the
//! level shifts per query, so the baseline is the median statement's score
//! (or a fixed 0.62 while history is too small for a median). Below MIN_LIFT
//! nothing is returned — an honest "nothing yet" beats random matches. (At a
//! lower bar, "dinner recipes" matched "draft the pricing page tonight".)

use std::sync::Arc;

use rusqlite::Connection;
use serde::Serialize;

use crate::engine::llm;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{db, vec_};

pub const EMBED_MODEL: &str = "embeddinggemma-2";
/// Rank needed to be a result. Measured: real matches ≥ 0.10, near-misses below.
const MIN_LIFT: f64 = 0.10;
/// Added when every query word appears in the statement (scaled by share).
const KEYWORD_BOOST: f64 = 0.08;
/// Below this many statements there's no stable median; use a fixed baseline.
const SMALL_INDEX: usize = 6;
const SMALL_INDEX_BASELINE: f64 = 0.62;

/// One search result: a statement you made.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// Item or task id (item ids are also mind-map node ids).
    pub id: String,
    /// task | idea | learning | question | decision
    pub kind: String,
    pub text: String,
    pub thought_id: Option<String>,
    pub said_at: String,
    pub score: f64,
    /// Query words found verbatim, for highlighting.
    pub matched: Vec<String>,
}

/// Embed and store statements: (ref_kind "item" | "task", id, text).
pub async fn index(state: &Arc<AppState>, statements: &[(String, String, String)]) -> Result<()> {
    if statements.is_empty() {
        return Ok(());
    }
    let texts: Vec<String> = statements.iter().map(|(_, _, t)| t.clone()).collect();
    let vectors = llm::embed_documents(&texts).await?;
    let conn = state.db.lock().map_err(|_| Error::Store("db poisoned".into()))?;
    for ((kind, id, _), v) in statements.iter().zip(vectors.iter()) {
        vec_::insert(&conn, kind, id, EMBED_MODEL, v)?;
    }
    Ok(())
}

/// Index anything saved while the embedding server was down. Runs at boot.
pub fn backfill_in_background(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let todo = match state.db.lock() {
            Ok(conn) => db::unindexed_statements(&conn, EMBED_MODEL).unwrap_or_default(),
            Err(_) => return,
        };
        if let Err(e) = index(&state, &todo).await {
            eprintln!("[search] backfill failed: {e}");
        }
    });
}

/// A searchable statement with its vector and metadata.
struct Statement {
    id: String,
    kind: String,
    text: String,
    thought_id: Option<String>,
    said_at: String,
    vec: Vec<f32>,
}

fn statements(conn: &Connection) -> Result<Vec<Statement>> {
    let mut out = Vec::new();
    for (id, vec) in vec_::all_vectors(conn, EMBED_MODEL, "item")? {
        if let Some(i) = db::get_item(conn, &id)? {
            out.push(Statement { id, kind: i.kind, text: i.text, thought_id: Some(i.thought_id), said_at: i.created_at, vec });
        }
    }
    let tasks: std::collections::HashMap<String, Vec<f32>> =
        vec_::all_vectors(conn, EMBED_MODEL, "task")?.into_iter().collect();
    for t in db::get_tasks(conn, true)? {
        if let Some(vec) = tasks.get(&t.id) {
            out.push(Statement {
                id: t.id,
                kind: "task".into(),
                text: t.title,
                thought_id: t.thought_id,
                said_at: t.created_at,
                vec: vec.clone(),
            });
        }
    }
    Ok(out)
}

/// Search everything you've said that was worth keeping, best first.
pub async fn search(state: &Arc<AppState>, query: &str, k: usize) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let qvec = llm::embed_query(query).await.map_err(|_| {
        Error::Engine("search runs on EmbeddingGemma 2, which is still starting — try again in a moment".into())
    })?;
    let conn = state.db.lock().map_err(|_| Error::Store("db poisoned".into()))?;
    Ok(rank(statements(&conn)?, query, &qvec, k))
}

fn rank(all: Vec<Statement>, query: &str, qvec: &[f32], k: usize) -> Vec<SearchHit> {
    if all.is_empty() {
        return Vec::new();
    }
    let cos: Vec<f64> = all.iter().map(|s| cosine(qvec, &s.vec)).collect();
    let baseline = if all.len() < SMALL_INDEX {
        SMALL_INDEX_BASELINE
    } else {
        let mut sorted = cos.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };
    let lowered: Vec<String> = all.iter().map(|s| s.text.to_lowercase()).collect();
    let weights = term_weights(&query_terms(query), &lowered);
    let total: f64 = weights.iter().map(|(_, w)| w).sum();

    let mut hits: Vec<SearchHit> = all
        .into_iter()
        .zip(cos)
        .zip(lowered)
        .filter_map(|((s, c), lower)| {
            let matched: Vec<(String, f64)> = weights.iter().filter(|(t, _)| lower.contains(t.as_str())).cloned().collect();
            let share = if total > 0.0 { matched.iter().map(|(_, w)| w).sum::<f64>() / total } else { 0.0 };
            let rank = (c - baseline) + KEYWORD_BOOST * share;
            (rank >= MIN_LIFT).then(|| SearchHit {
                id: s.id,
                kind: s.kind,
                text: s.text,
                thought_id: s.thought_id,
                said_at: s.said_at,
                score: rank,
                matched: matched.into_iter().map(|(t, _)| t).collect(),
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(k);
    hits
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

/// Rarity weight per query word, ln(1 + N / df) over the statements; words
/// that never occur are dropped (no evidence either way).
fn term_weights(terms: &[String], lowered: &[String]) -> Vec<(String, f64)> {
    let n = lowered.len() as f64;
    terms
        .iter()
        .filter_map(|t| {
            let df = lowered.iter().filter(|p| p.contains(t.as_str())).count();
            (df > 0).then(|| (t.clone(), (1.0 + n / df as f64).ln()))
        })
        .collect()
}

/// Lowercased content words of the query (short function words dropped).
fn query_terms(query: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "a", "an", "the", "and", "or", "of", "to", "in", "on", "for", "about", "with", "what",
        "did", "i", "my", "me", "say", "said", "is", "was", "it", "that", "this", "do",
    ];
    let mut out: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2 && !STOP.contains(w))
        .map(str::to_string)
        .collect();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: &str, text: &str, vec: Vec<f32>) -> Statement {
        Statement { id: id.into(), kind: "idea".into(), text: text.into(), thought_id: None, said_at: String::new(), vec }
    }

    #[test]
    fn nothing_below_the_bar() {
        // Small index → fixed 0.62 baseline; 0.64 is not a match, 0.80 is.
        let q = vec![1.0, 0.0];
        let hits = rank(
            vec![st("a", "close", vec![0.80, 0.60]), st("b", "far", vec![0.64, 0.77])],
            "anything",
            &q,
            5,
        );
        assert_eq!(hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn keyword_boost_rescues_names() {
        let q = vec![1.0, 0.0];
        // Same semantic score just under the bar; the one naming "priya" passes.
        let hits = rank(
            vec![st("a", "Call Priya about the invoice", vec![0.70, 0.71]), st("b", "Book the car service", vec![0.70, 0.71])],
            "Priya",
            &q,
            5,
        );
        assert_eq!(hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(hits[0].matched, vec!["priya"]);
    }

    #[test]
    fn query_terms_drop_function_words() {
        assert_eq!(query_terms("What did I say about Priya?"), vec!["priya"]);
    }

    /// Real embeddings (embed server on 8643): `cargo test search_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn search_live() {
        let said = [
            "We decided to price it at nine dollars a month. I still don't know whether to offer a free tier though. Let me draft the pricing page tonight.",
            "Okay so random thought, I was walking and I realized that the reason people don't journal is because writing is slow, talking is fast. Oh I need to email Sarah about the contract by Friday. And maybe we could do a voice-only version of the app for iPhone? Also why does my back hurt every morning, I should look into that.",
            "Call with Priya went fine. She suggested we try onboarding emails for new users. I think I'll set up a three email sequence next week.",
        ];
        let items: Vec<crate::engine::sort::Item> = said.iter().flat_map(|t| crate::engine::sort::sort(t)).collect();
        let texts: Vec<String> = items.iter().map(|i| i.text.clone()).collect();
        let vecs = llm::embed_documents(&texts).await.expect("embed server on 8643");
        let all = || -> Vec<Statement> {
            items.iter().zip(vecs.iter()).enumerate()
                .map(|(n, (i, v))| Statement { id: n.to_string(), kind: i.kind.as_str().into(), text: i.text.clone(), thought_id: None, said_at: String::new(), vec: v.clone() })
                .collect()
        };
        for q in ["pricing", "what did I decide about price", "back pain", "journaling", "Sarah", "email marketing", "mobile app", "quantum physics homework", "dinner recipes"] {
            let qv = llm::embed_query(q).await.unwrap();
            let hits = rank(all(), q, &qv, 3);
            println!("{q:30} → {}", if hits.is_empty() { "(nothing yet)".to_string() } else {
                hits.iter().map(|h| format!("[{}] {}", h.kind, h.text)).collect::<Vec<_>>().join("  |  ")
            });
        }
    }
}
