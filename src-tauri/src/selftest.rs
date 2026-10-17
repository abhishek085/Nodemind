//! Debug-builds-only end-to-end harness (`NM_SELFTEST=cases.json`).
//!
//! Once the engine is ready, runs each case through the same code path as a
//! real capture (wav → segmenter → whisper-server → `pipeline::organize`),
//! then dumps the resulting state and search results to `NM_SELFTEST_OUT`
//! and exits. Not compiled into release builds.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::audio::segmenter::Segmenter;
use crate::audio::session::CaptureMode;
use crate::engine::{pipeline, search, tasks, whisper};
use crate::state::AppState;
use crate::store::{db, graph};

pub fn run_if_requested(state: Arc<AppState>) {
    let Ok(path) = std::env::var("NM_SELFTEST") else {
        return;
    };
    let out = std::env::var("NM_SELFTEST_OUT").unwrap_or_else(|_| "/tmp/nm-selftest.json".into());
    tauri::async_runtime::spawn(async move {
        let report = match run(&state, &path).await {
            Ok(v) => v,
            Err(e) => json!({ "fatal": e }),
        };
        let _ = std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap_or_default());
        state.app.exit(0);
    });
}

async fn run(state: &Arc<AppState>, path: &str) -> Result<Value, String> {
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut captures = Vec::new();
    for case in spec["cases"].as_array().cloned().unwrap_or_default() {
        let t0 = std::time::Instant::now();
        let (transcript, mode) = match case["wav"].as_str() {
            Some(wav) => {
                let mut r = hound::WavReader::open(wav).map_err(|e| e.to_string())?;
                let samples: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap_or(0) as f32 / 32768.0).collect();
                let mut seg = Segmenter::new();
                let mut utterances = Vec::new();
                for chunk in samples.chunks(1_600) {
                    utterances.extend(seg.push(chunk));
                }
                utterances.extend(seg.finish());
                let mut text = String::new();
                for u in &utterances {
                    let t = whisper::transcribe_segment(state, u, "en", &text).await.map_err(|e| e.message())?;
                    if !t.is_empty() {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(&t);
                    }
                }
                (text, CaptureMode::Note)
            }
            None => (
                case["text"].as_str().unwrap_or("").to_string(),
                if case["mode"].as_str() == Some("meeting") { CaptureMode::Meeting } else { CaptureMode::Note },
            ),
        };
        let stt_ms = t0.elapsed().as_millis();
        let t1 = std::time::Instant::now();
        let r = pipeline::organize(state.clone(), transcript, "en".into(), crate::time::now_millis(), 20, mode)
            .await
            .map_err(|e| e.message())?;
        captures.push(json!({ "name": case["name"], "stt_ms": stt_ms, "organize_ms": t1.elapsed().as_millis(), "result": r }));
    }

    // Typed tasks go through the same duplicate check as spoken ones.
    let mut typed = Vec::new();
    for t in spec["typed"].as_array().cloned().unwrap_or_default() {
        let added = tasks::add_unique(state, vec![t.as_str().unwrap_or("").to_string()], None).await.map_err(|e| e.message())?;
        typed.push(json!({ "text": t, "added": !added.is_empty() }));
    }

    let mut searches = Vec::new();
    for q in spec["queries"].as_array().cloned().unwrap_or_default() {
        let hits = search::search(state, q.as_str().unwrap_or(""), 5).await.map_err(|e| e.message())?;
        searches.push(json!({ "query": q, "hits": hits }));
    }

    // Deleting a recording must take its tasks, items and map nodes with it.
    let mut deleted = Value::Null;
    if let Some(id) = captures.iter().find(|c| c["name"] == spec["delete"]).and_then(|c| c["result"]["thoughtId"].as_str().map(str::to_string)) {
        let conn = state.db.lock().map_err(|_| "db")?;
        let items: Vec<String> = db::items_for_thought(&conn, &id).map_err(|e| e.message())?.into_iter().map(|i| i.id).collect();
        let tasks_before = db::get_tasks(&conn, true).map_err(|e| e.message())?.iter().filter(|t| t.thought_id.as_deref() == Some(id.as_str())).count();
        graph::unrecord_items(&conn, &items).map_err(|e| e.message())?;
        db::delete_thought(&conn, &id).map_err(|e| e.message())?;
        let left = db::get_tasks(&conn, true).map_err(|e| e.message())?.iter().filter(|t| t.thought_id.as_deref() == Some(id.as_str())).count();
        deleted = json!({ "name": spec["delete"], "items": items.len(), "tasks_before": tasks_before, "tasks_after": left, "thought_gone": db::get_thought(&conn, &id).map_err(|e| e.message())?.is_none() });
    }

    let conn = state.db.lock().map_err(|_| "db")?;
    let g = graph::get_graph(&conn, 500).map_err(|e| e.message())?;
    Ok(json!({
        "captures": captures,
        "typed": typed,
        "searches": searches,
        "deleted": deleted,
        "open_tasks": db::get_tasks(&conn, false).map_err(|e| e.message())?.iter().map(|t| &t.title).collect::<Vec<_>>(),
        "graph": g,
    }))
}
