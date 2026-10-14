//! Thoughts, tasks, graph, and semantic-search commands.

use std::sync::Arc;

use tauri::State;

use crate::error::Result;
use crate::state::AppState;
use crate::store::db::{self, Task, Thought};
use crate::store::graph::{self, GraphData};

/// Recent thoughts, newest first (default 20).
#[tauri::command]
pub fn get_thoughts(state: State<'_, Arc<AppState>>, limit: Option<u32>) -> Result<Vec<Thought>> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::get_thoughts(&conn, limit.unwrap_or(20) as i64)
}

#[tauri::command]
pub fn get_thought(state: State<'_, Arc<AppState>>, id: String) -> Result<Option<Thought>> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::get_thought(&conn, &id)
}

/// Delete a thought (and its vectors + graph node). Tasks are kept.
#[tauri::command]
pub fn delete_thought(state: State<'_, Arc<AppState>>, id: String) -> Result<()> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::delete_thought(&conn, &id)?;
    graph::unrecord_thought(&conn, &id)?;
    Ok(())
}

// ── Tasks ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_tasks(state: State<'_, Arc<AppState>>, include_done: Option<bool>) -> Result<Vec<Task>> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::get_tasks(&conn, include_done.unwrap_or(false))
}

/// Manual task (keyboard path for "I'll just type it").
#[tauri::command]
pub fn add_task(
    state: State<'_, Arc<AppState>>,
    title: String,
    project: Option<String>,
) -> Result<Task> {
    let title = title.trim().to_string();
    if title.len() < 2 {
        return Err(crate::error::Error::Capture("task too short".into()));
    }
    let task = Task {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.clone(),
        project,
        thought_id: None,
        done: false,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::insert_task(&conn, &task)?;
    state.emit("tasks-updated", &serde_json::json!({}));
    Ok(task)
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

