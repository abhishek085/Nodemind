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
    use crate::store::vec_;

    let state = state.inner().clone();

    // 1) Collect all re-embeddable texts (thoughts + entity labels).
    let mut items: Vec<(String, String, String)> = Vec::new(); // kind, ref_id, text
    {
        let conn = lock_db(&state)?;
        let mut stmt = conn
            .prepare("SELECT id, text FROM thoughts ORDER BY created_at")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        for row in rows.flatten() {
            let text: String = row.1.chars().take(8_000).collect();
            items.push(("thought".into(), row.0, text));
        }
        for kind in ["topic", "person", "goal"] {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT id, label FROM graph_nodes WHERE node_type = '{kind}'"
                ))
                .map_err(|e| crate::error::Error::Store(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| crate::error::Error::Store(e.to_string()))?;
            for row in rows.flatten() {
                items.push((kind.to_string(), row.0, row.1.clone()));
            }
        }

        let model = "embeddinggemma-2";
        conn.execute(
            "DELETE FROM embeddings WHERE model = ?1",
            rusqlite::params![model],
        )
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    }

    // 2) Batch-embed in chunks of 16.
    let model = "embeddinggemma-2";
    let mut written = 0u32;
    for chunk in items.chunks(16) {
        let texts: Vec<String> = chunk.iter().map(|c| c.2.clone()).collect();
        let vecs = match crate::engine::llm::embed(&texts).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[reindex] embed batch failed: {e} — continuing");
                continue;
            }
        };
        let conn = lock_db(&state)?;
        for (i, (kind, ref_id, _)) in chunk.iter().enumerate() {
            if let Some(v) = vecs.get(i) {
                let _ = vec_::insert(&conn, kind, ref_id, model, v);
                written += 1;
            }
        }
        eprintln!("[reindex] {}/{} vectors", written, items.len());
    }
    Ok(written)
}


