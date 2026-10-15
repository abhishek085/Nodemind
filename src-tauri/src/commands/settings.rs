//! Settings commands: app info, language, data access, reindexing.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::error::Result;
use crate::state::AppState;
use crate::store::db;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub version: String,
    pub arch: String,
    pub language: String,
    pub data_dir: String,
    pub models_dir: String,
    pub prompts_path: Option<String>,
    pub counts: Counts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub thoughts: i64,
    pub tasks_open: i64,
    pub vectors: i64,
    pub nodes: i64,
}

#[tauri::command]
pub fn get_settings(state: State<'_, Arc<AppState>>) -> Result<Settings> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    let language = db::get_meta(&conn, "language")
        .ok()
        .flatten()
        .unwrap_or_else(|| "en".to_string());
    let counts = Counts {
        thoughts: count(&conn, "SELECT COUNT(*) FROM thoughts"),
        tasks_open: count(&conn, "SELECT COUNT(*) FROM tasks WHERE done = 0"),
        vectors: crate::store::vec_::count(&conn),
        nodes: count(&conn, "SELECT COUNT(*) FROM graph_nodes"),
    };
    let prompts_path = state
        .resources_dir
        .join("resources/prompts.toml")
        .to_string_lossy()
        .into_owned();
    Ok(Settings {
        version: db::app_version(),
        arch: crate::models::host_arch(),
        language,
        data_dir: state.models_dir.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        models_dir: state.models_dir.display().to_string(),
        prompts_path: Some(prompts_path),
        counts,
    })
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0)
}

/// Lock the shared connection, converting a poisoned mutex into our error.
fn lock_db<'s>(s: &'s Arc<AppState>) -> Result<std::sync::MutexGuard<'s, rusqlite::Connection>> {
    s.db.lock().map_err(|_| crate::error::Error::Store("db".into()))
}

/// A webview reports an error (render crash, unhandled rejection).
#[tauri::command]
pub fn log_ui_error(state: State<'_, Arc<AppState>>, message: String) {
    let message: String = message.chars().take(4_000).collect();
    crate::applog::write(&state, "ui.log", &message);
}

/// The capture language the user picked ("en" | "hi"), defaulting to English.
pub fn current_language(state: &AppState) -> String {
    state
        .db
        .lock()
        .ok()
        .and_then(|conn| db::get_meta(&conn, "language").ok().flatten())
        .unwrap_or_else(|| "en".to_string())
}

#[tauri::command]
pub fn set_language(state: State<'_, Arc<AppState>>, language: String) -> Result<()> {
    let language = if language == "hi" { "hi" } else { "en" }.to_string();
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::set_meta(&conn, "language", &language)?;
    state.emit("settings-updated", &serde_json::json!({ "language": language }));
    Ok(())
}

/// Reveal the app-data folder in Finder.
#[tauri::command]
pub fn open_data_dir(state: State<'_, Arc<AppState>>) -> Result<()> {
    let dir = state.models_dir.parent().unwrap_or(&state.models_dir);
    let path = dir.to_string_lossy().to_string();
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|e| crate::error::Error::Store(format!("cannot open data dir: {e}")))
}

/// Re-embed all stored thoughts + entity labels (after an embedding-model
/// upgrade). Returns the number of vectors written.
///
/// Fully async: the embed calls are loopback HTTP, and SQLite steps on a
/// personal-scale database take microseconds — no blocking thread needed.
#[tauri::command]
pub async fn reindex_embeddings(state: State<'_, Arc<AppState>>) -> Result<u32> {
    let state = state.inner().clone();
    let model = crate::engine::search::EMBED_MODEL;

    // Drop every vector, then re-embed what search and the map use: items and
    // tasks (document prefix), and topic / person / goal labels (plain).
    let (statements, labels) = {
        let conn = lock_db(&state)?;
        conn.execute("DELETE FROM embeddings WHERE model = ?1", rusqlite::params![model])
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let mut statements: Vec<(String, String, String)> = Vec::new();
        for (kind, sql) in [("item", "SELECT id, text FROM items"), ("task", "SELECT id, title FROM tasks")] {
            let mut stmt = conn.prepare(sql).map_err(|e| crate::error::Error::Store(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| crate::error::Error::Store(e.to_string()))?;
            statements.extend(rows.flatten().map(|(id, t)| (kind.to_string(), id, t)));
        }
        let mut stmt = conn
            .prepare("SELECT node_type, id, label FROM graph_nodes WHERE node_type IN ('topic', 'person', 'goal')")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let labels: Vec<(String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| crate::error::Error::Store(e.to_string()))?
            .flatten()
            .collect();
        (statements, labels)
    };

    let mut written = 0u32;
    for chunk in statements.chunks(16) {
        if crate::engine::search::index(&state, chunk).await.is_ok() {
            written += chunk.len() as u32;
        }
    }
    for chunk in labels.chunks(16) {
        let texts: Vec<String> = chunk.iter().map(|c| c.2.clone()).collect();
        if let Ok(vecs) = crate::engine::llm::embed(&texts).await {
            let conn = lock_db(&state)?;
            for ((kind, id, _), v) in chunk.iter().zip(vecs.iter()) {
                let _ = crate::store::vec_::insert(&conn, kind, id, model, v);
                written += 1;
            }
        }
    }
    crate::engine::links::refresh_in_background(state);
    Ok(written)
}


