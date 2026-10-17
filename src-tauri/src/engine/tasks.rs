//! The checklist's single door: every task (voice, meeting, typed) is added
//! through here so Today never shows the same thing twice.
//!
//! Two tasks are the same when either
//!   1. they match after stripping filler ("Remind me to add to policy" =
//!      "Add to policy"), or
//!   2. EmbeddingGemma 2 puts them at cosine ≥ DUPLICATE_COSINE
//!      ("Book a car service" = "Get the car serviced").
//!
//! The threshold was measured on real pairs: paraphrases scored 0.89–0.98,
//! while look-alike but different tasks topped out at 0.87 ("Buy milk" vs
//! "Buy eggs"). Without the embedding server, only rule 1 applies.

use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use crate::engine::llm;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::db;

const DUPLICATE_COSINE: f32 = 0.89;

/// Leading phrases that don't change what the task is.
const FILLER: &[&str] = &[
    "remind me to ", "remind to ", "reminder to ", "don't forget to ", "dont forget to ",
    "remember to ", "make sure to ", "make sure i ", "i need to ", "need to ", "i have to ",
    "have to ", "i should ", "should ", "i must ", "must ", "i want to ", "try to ", "please ",
];

/// A task created by a capture — enough for the UI to show and undo it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRef {
    pub id: String,
    pub title: String,
}

/// Canonical form for exact-duplicate checks: lowercase, no punctuation,
/// filler prefixes removed.
pub fn task_key(title: &str) -> String {
    let mut s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    s.push(' ');
    loop {
        let before = s.len();
        for f in FILLER {
            if let Some(rest) = s.strip_prefix(f) {
                s = rest.to_string();
            }
        }
        if s.len() == before {
            break;
        }
    }
    s.trim().to_string()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Index of the first earlier entry that `i` duplicates, if any.
fn duplicate_of(i: usize, keys: &[String], vecs: &[Vec<f32>]) -> Option<usize> {
    (0..i).find(|&j| {
        keys[j] == keys[i]
            || (vecs.len() == keys.len() && cosine(&vecs[i], &vecs[j]) >= DUPLICATE_COSINE)
    })
}

/// Add tasks to Today, skipping any that duplicate an open task (or each
/// other). Returns the tasks actually added, in order.
pub async fn add_unique(
    state: &Arc<AppState>,
    titles: Vec<String>,
    thought_id: Option<String>,
) -> Result<Vec<TaskRef>> {
    let titles: Vec<String> = titles
        .iter()
        .map(|t| clean_title(t))
        .filter(|t| t.chars().count() >= 2)
        .collect();
    if titles.is_empty() {
        return Ok(Vec::new());
    }
    let open = db::get_tasks(&*lock(state)?, false)?;

    // Existing open tasks first, then the candidates, so candidates are
    // checked against everything before them.
    let all: Vec<&str> = open.iter().map(|t| t.title.as_str()).chain(titles.iter().map(String::as_str)).collect();
    let keys: Vec<String> = all.iter().map(|t| task_key(t)).collect();
    let texts: Vec<String> = all.iter().map(|t| t.to_string()).collect();
    let vecs = llm::embed(&texts).await.unwrap_or_default();

    let conn = lock(state)?;
    let mut added = Vec::new();
    for (n, title) in titles.into_iter().enumerate() {
        let i = open.len() + n;
        if keys[i].is_empty() || duplicate_of(i, &keys, &vecs).is_some() {
            continue;
        }
        let task = db::Task {
            id: Uuid::new_v4().to_string(),
            title: title.clone(),
            project: None,
            thought_id: thought_id.clone(),
            done: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        db::insert_task(&conn, &task)?;
        added.push(TaskRef { id: task.id, title: task.title });
    }
    Ok(added)
}

/// Remove open tasks that duplicate an earlier open task (keeps the oldest).
/// Returns how many were removed.
pub async fn dedupe_open(state: &Arc<AppState>) -> Result<usize> {
    let mut open = db::get_tasks(&*lock(state)?, false)?;
    // Oldest first, so the oldest of a duplicate group is the one kept.
    open.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let keys: Vec<String> = open.iter().map(|t| task_key(&t.title)).collect();
    let texts: Vec<String> = open.iter().map(|t| t.title.clone()).collect();
    let vecs = llm::embed(&texts).await.unwrap_or_default();

    let conn = lock(state)?;
    let mut removed = 0;
    for (i, dup) in later_duplicates(&keys, &vecs).into_iter().enumerate() {
        if dup {
            db::delete_task(&conn, &open[i].id)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// For tasks in creation order, which ones repeat an earlier kept task.
fn later_duplicates(keys: &[String], vecs: &[Vec<f32>]) -> Vec<bool> {
    let mut gone = vec![false; keys.len()];
    for i in 0..keys.len() {
        gone[i] = (0..i).filter(|&j| !gone[j]).any(|j| {
            keys[j] == keys[i] || (vecs.len() == keys.len() && cosine(&vecs[i], &vecs[j]) >= DUPLICATE_COSINE)
        });
    }
    gone
}

/// Startup pass, after the embedding server is up.
pub fn dedupe_in_background(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        match dedupe_open(&state).await {
            Ok(0) => {}
            Ok(n) => {
                crate::applog::write(&state, "app.log", &format!("merged {n} duplicate tasks"));
                state.emit("tasks-updated", &serde_json::json!({}));
            }
            Err(e) => eprintln!("[tasks] dedupe failed: {e}"),
        }
    });
}

/// The title as shown: filler prefix removed, no trailing period, capitalized
/// ("remind me to call Mom." → "Call Mom").
fn clean_title(title: &str) -> String {
    let mut s = title.trim();
    // FILLER is lowercase ASCII, so the byte offset is the same in `s`.
    while let Some(f) = FILLER.iter().find(|f| s.to_lowercase().starts_with(*f)) {
        s = s[f.len()..].trim_start();
    }
    capitalize(s.trim_end_matches('.').trim())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>> {
    state.db.lock().map_err(|_| Error::Store("db poisoned".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filler_is_ignored() {
        assert_eq!(task_key("Remind to add to policy"), task_key("Add to policy."));
        assert_eq!(task_key("Don't forget to call Mom!"), task_key("call mom"));
        assert_eq!(task_key("I need to remember to buy milk"), "buy milk");
        assert_ne!(task_key("Add to policy"), task_key("Go back to policy"));
    }

    #[test]
    fn semantic_duplicates_need_high_similarity() {
        let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vecs = vec![vec![1.0, 0.0], vec![0.95, 0.31], vec![0.0, 1.0]]; // b≈a (0.95); c unlike both
        assert_eq!(duplicate_of(1, &keys, &vecs), Some(0));
        assert_eq!(duplicate_of(2, &keys, &vecs), None);
    }

    #[test]
    fn titles_lose_filler() {
        assert_eq!(clean_title("remind me to call Mom."), "Call Mom");
        assert_eq!(clean_title("Don't forget to book the demo room"), "Book the demo room");
        assert_eq!(clean_title("Water the plants"), "Water the plants");
    }

    #[test]
    fn capitalizes_first_letter() {
        assert_eq!(capitalize("call priya"), "Call priya");
    }

    /// Your real checklist through the live embedding server (port 8643):
    /// `cargo test dedupe_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn dedupe_live() {
        let titles: Vec<String> = [
            "Take my notes", "Think about", "Add to policy", "Remind to go back to policy",
            "Go back to policy", "Remind to add to policy", "Answer questions right in", "Relax",
            "Read linear regression", "Study from other sources", "Ask why errors are squared in OLS",
            "Book a car service", "Get the car serviced", "Buy milk", "Buy eggs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let keys: Vec<String> = titles.iter().map(|t| task_key(t)).collect();
        let vecs = llm::embed(&titles).await.expect("embed server on 8643");
        let gone = later_duplicates(&keys, &vecs);
        for (t, g) in titles.iter().zip(&gone) {
            println!("{} {t}", if *g { "MERGED " } else { "  kept " });
        }
        let merged: Vec<&str> = titles.iter().zip(&gone).filter(|(_, g)| **g).map(|(t, _)| t.as_str()).collect();
        assert_eq!(merged, vec!["Go back to policy", "Remind to add to policy", "Get the car serviced"]);
    }
}
