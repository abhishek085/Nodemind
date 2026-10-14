//! Thoughts, tasks, graph, and semantic-search commands.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::error::Result;
use crate::state::AppState;
use crate::store::db::{self, Thought};
use crate::store::graph::{self, GraphData};

#[tauri::command]
pub fn get_thought(state: State<'_, Arc<AppState>>, id: String) -> Result<Option<Thought>> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::get_thought(&conn, &id)
}

/// Delete a thought with its tasks, vectors, passages and graph node.
#[tauri::command]
pub fn delete_thought(state: State<'_, Arc<AppState>>, id: String) -> Result<()> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::delete_thought(&conn, &id)?;
    graph::unrecord_thought(&conn, &id)?;
    Ok(())
}

// ── Tasks ───────────────────────────────────────────────────────────────────

/// Manual task (keyboard path for "I'll just type it"). Returns `None` when
/// it duplicates an open task, which is then left as is.
#[tauri::command]
pub async fn add_task(
    state: State<'_, Arc<AppState>>,
    title: String,
) -> Result<Option<crate::engine::tasks::TaskRef>> {
    let added = crate::engine::tasks::add_unique(state.inner(), vec![title], None).await?;
    state.emit("tasks-updated", &serde_json::json!({}));
    Ok(added.into_iter().next())
}

/// Rename and/or toggle a task.
#[tauri::command]
pub fn update_task(
    state: State<'_, Arc<AppState>>,
    id: String,
    title: Option<String>,
    done: Option<bool>,
) -> Result<()> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    if let Some(title) = &title {
        let title = title.trim();
        if !title.is_empty() {
            db::rename_task(&conn, &id, title)?;
        }
    }
    if let Some(done) = done {
        db::set_task_done(&conn, &id, done)?;
    }
    state.emit("tasks-updated", &serde_json::json!({}));
    Ok(())
}

#[tauri::command]
pub fn delete_task(state: State<'_, Arc<AppState>>, id: String) -> Result<()> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::delete_task(&conn, &id)?;
    state.emit("tasks-updated", &serde_json::json!({}));
    Ok(())
}

// ── Graph ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_graph(state: State<'_, Arc<AppState>>, limit: Option<u32>) -> Result<GraphData> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    graph::get_graph(&conn, limit.unwrap_or(300) as i64)
}

/// Semantic search (EmbeddingGemma 2) over everything you've said:
/// the best-matching passage per thought.
#[tauri::command]
pub async fn search_memory(
    state: State<'_, Arc<AppState>>,
    query: String,
    k: Option<u32>,
) -> Result<Vec<crate::engine::search::SearchHit>> {
    crate::engine::search::search(state.inner(), &query, k.unwrap_or(12) as usize).await
}


// ── Today ───────────────────────────────────────────────────────────────────

/// Everything the Today screen shows, in one call: the open checklist (each
/// task with where it came from) and today's captures as a timeline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Today {
    pub day: String,
    pub tasks: Vec<TodayTask>,
    pub thoughts: Vec<TodayThought>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayTask {
    pub id: String,
    pub title: String,
    pub created_at: String,
    /// The capture it came from, if any (typed tasks have none).
    pub source: Option<TaskSource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSource {
    pub thought_id: String,
    pub kind: String,
    pub title: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayThought {
    pub id: String,
    pub kind: String,
    pub started_at: String,
    pub duration_secs: i64,
    /// Meeting title, else the one-line summary, else the opening words.
    pub title: String,
    pub text: String,
    pub minutes: Option<serde_json::Value>,
    /// Where the thinking seemed stuck (engine::fog), if anywhere.
    pub fog: Option<serde_json::Value>,
    pub task_count: usize,
}

/// A short, human title for a capture.
fn thought_title(t: &Thought) -> String {
    let minutes_title = t
        .minutes
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v["title"].as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty());
    minutes_title
        .or_else(|| t.summary.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| {
            let words: Vec<&str> = t.text.split_whitespace().take(9).collect();
            format!("{}{}", words.join(" "), if t.text.split_whitespace().count() > 9 { "…" } else { "" })
        })
}

#[tauri::command]
pub fn get_today(state: State<'_, Arc<AppState>>) -> Result<Today> {
    let day = crate::time::local_day_key();
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    let open = db::get_tasks(&conn, false)?;
    let all_tasks = db::get_tasks(&conn, true)?;
    let todays = db::get_thoughts_for_day(&conn, &day)?;

    let mut sources: std::collections::HashMap<String, TaskSource> = std::collections::HashMap::new();
    let mut tasks = Vec::with_capacity(open.len());
    for t in open {
        let source = match &t.thought_id {
            Some(tid) => match sources.get(tid) {
                Some(s) => Some(s.clone()),
                None => db::get_thought(&conn, tid)?.map(|th| {
                    let s = TaskSource {
                        thought_id: th.id.clone(),
                        kind: th.kind.clone(),
                        title: thought_title(&th),
                        started_at: th.started_at.clone(),
                    };
                    sources.insert(tid.clone(), s.clone());
                    s
                }),
            },
            None => None,
        };
        tasks.push(TodayTask { id: t.id, title: t.title, created_at: t.created_at, source });
    }

    let thoughts = todays
        .iter()
        .map(|t| TodayThought {
            id: t.id.clone(),
            kind: t.kind.clone(),
            started_at: t.started_at.clone(),
            duration_secs: t.duration_secs,
            title: thought_title(t),
            text: t.text.clone(),
            minutes: t.minutes.as_deref().and_then(|m| serde_json::from_str(m).ok()),
            fog: t.fog.as_deref().and_then(|f| serde_json::from_str(f).ok()),
            task_count: all_tasks.iter().filter(|x| x.thought_id.as_deref() == Some(t.id.as_str())).count(),
        })
        .collect();

    Ok(Today { day, tasks, thoughts })
}
