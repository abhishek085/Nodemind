//! Memory search: "what did I say about…" over every transcript, by meaning.
//!
//! Each transcript is split into short passages (a few sentences each) and
//! every passage gets an EmbeddingGemma 2 vector, so a detail buried in a
//! ten-minute ramble is still findable — and "car maintenance" finds "the car
//! needs servicing" with no shared words.
//!
//! Ranking is semantic first, with a keyword boost:
//!
//!   rank = (cosine − median cosine) + KEYWORD_BOOST × (weighted share of query words present)
//!
//! EmbeddingGemma 2's cosine scores sit high even for unrelated text (~0.6)
//! and the baseline shifts per query, so meaning is measured as the *lift*
//! over the median passage for this query. Measured lifts: unrelated ≤ +0.07,
//! relevant +0.08 … +0.23. The boost is on the same scale, so an exact word
//! (a name like "Priya") moves a passage up without overriding meaning.
//! Words are weighted by rarity across your history (names count most), and
//! words you've never said don't count against the share at all.
//! Only embedded passages are searched — no vectors, no results. When nothing
//! clears the bar, the closest few come back flagged `weak` rather than none.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::engine::llm;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{db, vec_};

pub const EMBED_MODEL: &str = "embeddinggemma-2";
const PASSAGE_MAX_CHARS: usize = 320;
/// Rank needed to show up at all…
const MIN_LIFT: f64 = 0.05;
/// …and to be flagged as a strong match.
const STRONG_LIFT: f64 = 0.10;
/// Added when every query word appears in the passage (scaled by the share).
const KEYWORD_BOOST: f64 = 0.08;
/// Closest passages returned (as `weak`) when nothing clears MIN_LIFT.
const WEAK_FALLBACK: usize = 3;

/// One search result: the best-matching passage of a thought.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub thought_id: String,
    pub snippet: String,
    pub summary: Option<String>,
    pub day: String,
    pub started_at: String,
    /// Semantic lift plus keyword boost (see module docs).
    pub score: f64,
    /// Clearly above the rest of your history (vs. merely the closest).
    pub strong: bool,
    /// Nothing really matched; this is just among the closest.
    pub weak: bool,
    /// Query words found verbatim in the snippet, for highlighting.
    pub matched: Vec<String>,
}

/// Split a transcript into passages of whole sentences, ≤ ~320 chars each.
pub fn split_passages(text: &str) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        if word.ends_with(['.', '?', '!']) || cur.len() >= PASSAGE_MAX_CHARS {
            sentences.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        sentences.push(cur);
    }

    let mut passages: Vec<String> = Vec::new();
    let mut cur = String::new();
    for s in sentences {
        if !cur.is_empty() && cur.len() + 1 + s.len() > PASSAGE_MAX_CHARS {
            passages.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&s);
    }
    if !cur.is_empty() {
        passages.push(cur);
    }
    passages
}

/// Split, embed and store one thought's passages. Replaces earlier ones.
pub async fn index_thought(state: &Arc<AppState>, thought_id: &str, text: &str) -> Result<usize> {
    let passages = split_passages(text);
    if passages.is_empty() {
        return Ok(0);
    }
    // Vectors are optional: passages are still keyword-searchable without them.
    let vectors = llm::embed_documents(&passages).await.unwrap_or_default();
    let conn = state.db.lock().map_err(|_| Error::Store("db poisoned".into()))?;
    let ids = db::replace_passages(&conn, thought_id, &passages)?;
    for (id, v) in ids.iter().zip(vectors.iter()) {
        vec_::insert(&conn, "passage", id, EMBED_MODEL, v)?;
    }
    Ok(ids.len())
}

/// Index any thought that has no passages yet (older data, or captures made
/// while the embedding server was down). Runs in the background at boot.
pub fn backfill_in_background(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let todo = match state.db.lock() {
            Ok(conn) => db::thoughts_missing_passage_vectors(&conn, EMBED_MODEL).unwrap_or_default(),
            Err(_) => return,
        };
        let mut n = 0;
        for (id, text) in todo {
            if index_thought(&state, &id, &text).await.is_ok() {
                n += 1;
            }
        }
        if n > 0 {
            eprintln!("[search] indexed {n} thoughts");
        }
    });
}

/// Semantic search over all passages; one hit per thought, best first.
pub async fn search(state: &Arc<AppState>, query: &str, k: usize) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let qvec = llm::embed_query(query).await.map_err(|_| {
        Error::Engine("search runs on EmbeddingGemma 2, which is still starting — try again in a moment".into())
    })?;

    let conn = state.db.lock().map_err(|_| Error::Store("db poisoned".into()))?;
    rank(&conn, query, &qvec, k)
}

/// Rank every embedded passage against an embedded query (see module docs).
fn rank(conn: &rusqlite::Connection, query: &str, qvec: &[f32], k: usize) -> Result<Vec<SearchHit>> {
    let scored = vec_::knn(conn, EMBED_MODEL, Some("passage"), qvec, usize::MAX)?;
    let Some(median) = median_score(&scored) else {
        return Ok(Vec::new());
    };
    let cosine: HashMap<String, f64> = scored.into_iter().map(|h| (h.ref_id, h.score)).collect();
    let passages: Vec<db::Passage> = db::all_passages(conn)?
        .into_iter()
        .filter(|p| cosine.contains_key(&p.id)) // not embedded yet — can't judge it
        .collect();
    let lowered: Vec<String> = passages.iter().map(|p| p.text.to_lowercase()).collect();
    let weights = term_weights(&query_terms(query), &lowered);
    let total_weight: f64 = weights.iter().map(|(_, w)| w).sum();

    // Best passage per thought: (rank, passage, matched words).
    let mut best: HashMap<String, (f64, db::Passage, Vec<String>)> = HashMap::new();
    for (p, lower) in passages.into_iter().zip(lowered) {
        let matched: Vec<(String, f64)> =
            weights.iter().filter(|(t, _)| lower.contains(t.as_str())).cloned().collect();
        let share = if total_weight > 0.0 {
            matched.iter().map(|(_, w)| w).sum::<f64>() / total_weight
        } else {
            0.0
        };
        let rank = (cosine[&p.id] - median) + KEYWORD_BOOST * share;
        if best.get(&p.thought_id).is_none_or(|(r, _, _)| *r < rank) {
            let words = matched.into_iter().map(|(t, _)| t).collect();
            best.insert(p.thought_id.clone(), (rank, p, words));
        }
    }

    let mut ranked: Vec<_> = best.into_values().collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let clear = ranked.iter().take_while(|r| r.0 >= MIN_LIFT).count();
    let weak = clear == 0;
    ranked.truncate(if weak { WEAK_FALLBACK.min(k) } else { clear.min(k) });

    let mut hits = Vec::with_capacity(ranked.len());
    for (rank, p, matched) in ranked {
        let Some(t) = db::get_thought(conn, &p.thought_id)? else {
            continue;
        };
        hits.push(SearchHit {
            thought_id: t.id,
            snippet: p.text,
            summary: t.summary,
            day: t.day,
            started_at: t.started_at,
            score: rank,
            strong: rank >= STRONG_LIFT,
            weak,
            matched,
        });
    }
    Ok(hits)
}

/// Rarity weight per query word, ln(1 + N / df) over your passages; words
/// that never occur in your history are dropped (no evidence either way).
fn term_weights(terms: &[String], passages_lower: &[String]) -> Vec<(String, f64)> {
    let n = passages_lower.len() as f64;
    terms
        .iter()
        .filter_map(|t| {
            let df = passages_lower.iter().filter(|p| p.contains(t.as_str())).count();
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

/// Median similarity across all passages — the "unrelated" baseline. With
/// very little history there is no meaningful baseline, so everything counts.
fn median_score(scored: &[vec_::Hit]) -> Option<f64> {
    match scored.len() {
        0 => None,
        1..=4 => Some(scored.iter().map(|h| h.score).fold(f64::MAX, f64::min) - MIN_LIFT),
        n => {
            let mut v: Vec<f64> = scored.iter().map(|h| h.score).collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            Some(v[n / 2])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passages_keep_sentences_whole() {
        let text = "Call Priya about the invoice. ".repeat(20);
        let ps = split_passages(&text);
        assert!(ps.len() > 1);
        assert!(ps.iter().all(|p| p.len() <= PASSAGE_MAX_CHARS && p.ends_with('.')));
        assert_eq!(ps.join(" "), text.trim());
    }

    #[test]
    fn unpunctuated_text_still_splits() {
        let text = "word ".repeat(200);
        let ps = split_passages(&text);
        assert!(ps.len() >= 3);
        assert!(ps.iter().all(|p| p.len() <= PASSAGE_MAX_CHARS + 8));
    }

    #[test]
    fn rare_words_weigh_more_and_unknown_words_drop() {
        let ps: Vec<String> = ["call rahul today", "call mom", "call the bank", "gym today"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let w = term_weights(&["rahul".into(), "call".into(), "agreement".into()], &ps);
        assert_eq!(w.len(), 2, "never-said words are dropped");
        assert!(w[0].1 > w[1].1, "rahul (1 passage) outweighs call (3)");
    }

    #[test]
    fn query_terms_drop_function_words() {
        assert_eq!(query_terms("What did I say about Priya?"), vec!["priya"]);
        assert_eq!(query_terms("car servicing"), vec!["car", "servicing"]);
    }

    fn hit(score: f64) -> vec_::Hit {
        vec_::Hit { ref_kind: "passage".into(), ref_id: String::new(), score }
    }

    #[test]
    fn median_is_the_baseline() {
        let scored: Vec<_> = [0.81, 0.66, 0.61, 0.60, 0.58].into_iter().map(hit).collect();
        assert_eq!(median_score(&scored), Some(0.61));
    }

    #[test]
    fn tiny_history_keeps_everything() {
        let scored: Vec<_> = [0.70, 0.62].into_iter().map(hit).collect();
        let m = median_score(&scored).unwrap();
        assert!(scored.iter().all(|h| h.score - m >= MIN_LIFT));
    }

    /// End-to-end ranking against the live embedding server on EMBED_PORT:
    /// `cargo test search_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn search_live_quality() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_schema(&mut conn).unwrap();
        let notes = [
            "Okay so tomorrow I need to call Priya about the invoice because it is overdue.",
            "The slides for Monday's board meeting are still not done and I keep putting them off.",
            "Mom's birthday is next Thursday so I need to order a gift by Sunday.",
            "The car needs servicing and I have not booked it yet.",
            "I keep thinking about whether to move the Nodemind launch to next month.",
            "I want to read that paper on small language models this weekend.",
            "I should go to the gym three times a week, Monday Wednesday Friday mornings.",
            "Remind me to email Rahul about the contract renewal before Friday.",
        ];
        for (i, text) in notes.iter().enumerate() {
            let id = format!("t{i}");
            db::insert_thought(&conn, &db::Thought {
                id: id.clone(), text: text.to_string(), summary: None, language: "en".into(),
                day: "2026-10-14".into(), started_at: "2026-10-14T10:00:00Z".into(),
                duration_secs: 5, created_at: "2026-10-14T10:00:00Z".into(), kind: "note".into(), minutes: None,
            }).unwrap();
            let ps = split_passages(text);
            let vs = llm::embed_documents(&ps).await.expect("embed server on 8643");
            for (pid, v) in db::replace_passages(&conn, &id, &ps).unwrap().iter().zip(vs.iter()) {
                vec_::insert(&conn, "passage", pid, EMBED_MODEL, v).unwrap();
            }
        }
        let cases = [
            ("money I'm owed", "t0"), ("Priya", "t0"), ("presentation deck", "t1"),
            ("present for my mother", "t2"), ("vehicle maintenance", "t3"),
            ("product release timing", "t4"), ("research reading", "t5"),
            ("workout schedule", "t6"), ("agreement with Rahul", "t7"),
        ];
        let mut top1 = 0;
        for (q, want) in cases {
            let qv = llm::embed_query(q).await.unwrap();
            let hits = rank(&conn, q, &qv, 3).unwrap();
            let got: Vec<String> = hits.iter().map(|h| format!("{}{}", h.thought_id, if h.strong { "*" } else { "" })).collect();
            top1 += (hits.first().map(|h| h.thought_id.as_str()) == Some(want)) as usize;
            println!("{q:24} want {want} → {got:?}");
        }
        for q in ["quantum physics homework", "tax filing deadline"] {
            let qv = llm::embed_query(q).await.unwrap();
            let hits = rank(&conn, q, &qv, 3).unwrap();
            println!("{q:24} (unrelated) → {} hits, strong: {}", hits.len(), hits.iter().filter(|h| h.strong).count());
        }
        println!("top-1: {top1}/{}", cases.len());
        assert!(top1 >= cases.len() - 1);
    }
}
