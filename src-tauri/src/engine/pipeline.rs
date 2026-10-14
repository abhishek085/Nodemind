//! The capture pipeline: voice → text → checklist + graph memory.
//!
//! ```
//! PCM (frontend) → wav → whisper-cli ─→ transcript
//!                                         │
//!              ┌──────────────┬───────────┤
//!              ▼              ▼           ▼
//!          clean()       annotate()   embed()  (EmbeddingGemma 2)
//!        checklist      graph nodes   vector store
//!                             │           │
//!                             └─── merge (fuzzy, cosine ≥ 0.8)
//! ```
//!
//! Heavy steps (file IO, whisper subprocess, HTTP to the servers) run on
//! `spawn_blocking`; the command itself is async so the UI never freezes.

use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{db, graph, vec_};
use crate::time;
use uuid::Uuid;

/// A task extracted by the annotator.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDraft {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_hint: Option<String>,
}

/// Structured extraction from a transcript.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub tasks: Vec<TaskDraft>,
    pub topics: Vec<String>,
    pub people: Vec<String>,
    pub goals: Vec<String>,
    /// ids of earlier thoughts this one relates to
    pub related: Vec<String>,
    pub sentiment: String,
}

/// What the UI shows after a capture completes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub thought_id: String,
    pub transcript: String,
    pub summary: Option<String>,
    pub tasks: Vec<String>,
    pub topics: Vec<String>,
    pub people: Vec<String>,
    pub related: Vec<String>,
    /// Parsed direct command, if the transcript looked like one ("remind me…",
    /// "start focus…"). The UI renders it as an actionable chip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<crate::engine::llm::CommandIntent>,
    pub duration_secs: i64,
}

/// Cheap pre-filter for the command parser — most transcripts are ambient
/// brain-dumps and should not pay for an LLM classification call.
fn looks_like_command(t: &str) -> bool {
    let lower = t.to_lowercase();
    if lower.contains("remind me")
        || lower.contains("add a task")
        || lower.contains("add task")
        || lower.contains("start focus")
        || lower.contains("start a focus")
        || lower.contains("note that")
    {
        return true;
    }
    lower
        .split_whitespace()
        .take(3)
        .any(|w| matches!(w, "add" | "remind" | "note" | "jot" | "create" | "start"))
}

fn stage(state: &AppState, stage: &str) {
    state.emit(
        "capture-stage",
        &serde_json::json!({ "stage": stage, "ts": time::now_millis() }),
    );
}

/// 20 minutes of 16 kHz mono s16 — the capture ceiling.
pub const MAX_CAPTURE_BYTES: usize = 20 * 60 * 16_000 * 2;

/// Run the whole pipeline for the current capture buffer.
pub async fn process_capture(state: Arc<AppState>) -> Result<CaptureResult> {
    // 1) Pull the audio out of the shared buffer.
    let (pcm, language, started_at_ms) = {
        let mut cap = state
            .capture
            .lock()
            .map_err(|_| Error::Capture("capture buffer poisoned".into()))?;
        if !cap.active {
            return Err(Error::Capture("no active capture".into()));
        }
        let mut pcm = std::mem::take(&mut cap.pcm);
        let (pcm, started) = if pcm.len() > MAX_CAPTURE_BYTES {
            // keep the most recent 20 minutes
            let from = pcm.len() - MAX_CAPTURE_BYTES;
            (pcm.split_off(from), cap.started_at + from as i64 / 32)
        } else {
            (pcm, cap.started_at)
        };
        cap.active = false;
        (pcm, cap.language.clone(), started)
    };
    let _ = started_at_ms;
    let duration_secs = (pcm.len() as f64 / (16_000.0 * 2.0)).round() as i64;

    if pcm.len() < 16_000 * 2 {
        // Less than a second of audio — treat as a no-op.
        stage(&state, "done");
        return Ok(CaptureResult {
            thought_id: Uuid::new_v4().to_string(),
            transcript: "(no audio)".to_string(),
            summary: None,
            tasks: Vec::new(),
            topics: Vec::new(),
            people: Vec::new(),
            related: Vec::new(),
            command: None,
            duration_secs: 0,
        });
    }

    // 2) wav + whisper in one blocking job (tempdir must outlive both).
    stage(&state, "transcribing");
    let transcript = {
        let s = state.clone();
        let language = language.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let tmp = tempfile::Builder::new()
                .prefix("nm-audio-")
                .tempdir()
                .map_err(|e| Error::Capture(format!("tempdir: {e}")))?;
            let wav = tmp.path().join("capture.wav");
            crate::engine::whisper::write_wav(&pcm, &wav)?;
            let text = crate::engine::whisper::transcribe(&s, &wav, &language)?;
            drop(tmp);
            Ok::<String, Error>(text)
        })
        .await
        .map_err(|e| Error::Capture(format!("transcription task: {e}")))?
        .map_err(|e| Error::Capture(e.message()))?
    };
    let transcript = transcript.trim().to_string();

    if transcript.len() < 2 {
        stage(&state, "done");
        return Ok(CaptureResult {
            thought_id: Uuid::new_v4().to_string(),
            transcript: "(silence)".to_string(),
            summary: None,
            tasks: Vec::new(),
            topics: Vec::new(),
            people: Vec::new(),
            related: Vec::new(),
            command: None,
            duration_secs,
        });
    }

    // 3) Persist the thought first (memory survives even if the LLM step dies).
    let thought_id = Uuid::new_v4().to_string();
    let started_rfc = chrono::DateTime::from_timestamp_millis(started_at_ms)
        .map(|d| d.with_timezone(&chrono::Utc).to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let thought = db::Thought {
        id: thought_id.clone(),
        text: transcript.clone(),
        summary: None,
        language: language.clone(),
        day: time::local_day_key(),
        started_at: started_rfc,
        duration_secs,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    {
        let conn = state
            .db
            .lock()
            .map_err(|_| Error::Store("db poisoned".into()))?;
        if let Err(e) = db::insert_thought(&conn, &thought) {
            return Err(Error::Store(e.message()));
        }
    }
    state.emit("thoughts-updated", &serde_json::json!({}));

    // 4) Checklist cleanup — the flagship feature.
    stage(&state, "organizing");
    let tasks = crate::engine::llm::clean_checklist(&transcript).await.unwrap_or_default();

    // 5) Graph annotation (with recent thoughts as linking context).
    let pairs: Vec<(String, String)> = {
        let conn = state
            .db
            .lock()
            .map_err(|_| Error::Store("db poisoned".into()))?;
        db::get_thoughts(&conn, 8)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.id != thought_id)
            .map(|t| {
                (
                    t.id.clone(),
                    t.text.chars().take(140).collect::<String>(),
                )
            })
            .collect()
    };
    let annotation =
        crate::engine::llm::annotate(&transcript, &pairs).await.unwrap_or_default();

    // 5b) Direct-command pre-pass — only for transcripts that *look* like a
    // command, so ambient brain-dumps don't pay for the call.
    let command = if looks_like_command(&transcript) {
        crate::engine::llm::parse_command(&transcript)
            .await
            .ok()
            .filter(|c| c.intent != "unknown")
    } else {
        None
    };

    // 5c) Cognitive Mirror — friction signals, only for longer transcripts.
    let fog_signals = if transcript.chars().count() > 200 {
        crate::engine::llm::fog(&transcript).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    // 6) Embed: thought + entity labels in one batch call.
    let mut embed_texts: Vec<String> = Vec::new();
    embed_texts.push(transcript.chars().take(8_000).collect());
    embed_texts.extend(annotation.topics.iter().cloned().take(8));
    embed_texts.extend(annotation.people.iter().cloned().take(8));
    embed_texts.extend(annotation.goals.iter().cloned().take(8));
    let embed_model = "embeddinggemma-2";
    let vectors = crate::engine::llm::embed(&embed_texts).await.unwrap_or_default();

    // 7) Vector store + graph + tasks (all local).
    {
        let conn = state
            .db
            .lock()
            .map_err(|_| Error::Store("db poisoned".into()))?;

        // thought vector + node
        let thought_vec: Option<&[f32]> = vectors.first().map(|v| v.as_slice());
        if let Some(v) = thought_vec {
            let _ = vec_::insert(&conn, "thought", &thought_id, embed_model, v);
        }
        // Thought node is keyed by the thought's UUID (thoughts are unique
        // events; get_graph joins this id back to the thoughts table).
        let excerpt: String = transcript.split_whitespace().take(12).collect::<Vec<_>>().join(" ");
        let _ = graph::upsert_thought_node(&conn, &thought_id, excerpt.trim(), &annotation.sentiment);

        // entity nodes (topics / people / goals)
        let mut entity_ids: Vec<(String, String)> = Vec::new(); // (id, kind)
        let mut vi = 1usize;
        for (kind, labels) in [
            ("topic", &annotation.topics),
            ("person", &annotation.people),
            ("goal", &annotation.goals),
        ] {
            for label in labels.iter().take(8) {
                let vec = vectors.get(vi).cloned().unwrap_or_default();
                vi += 1;
                if !vec.is_empty() {
                    let _ = vec_::insert(&conn, kind, label, embed_model, &vec);
                }
                let vec_ref = if vec.is_empty() { None } else { Some(vec.as_slice()) };
                if let Some(id) =
                    graph::upsert_entity(&conn, embed_model, kind, label, &annotation.sentiment, vec_ref)?
                {
                    entity_ids.push((id, kind.to_string()));
                }
            }
        }

        // edges: thought → its entities; thought → related thoughts
        for (id, _) in &entity_ids {
            let _ = graph::upsert_edge(&conn, &thought_id, id, "involves");
        }
        for rel in &annotation.related {
            if rel != &thought_id {
                let _ = graph::upsert_edge(&conn, &thought_id, rel, "relates_to");
            }
        }

        // tasks from the annotator
        for t in &annotation.tasks {
            let task = db::Task {
                id: Uuid::new_v4().to_string(),
                title: t.title.clone(),
                project: t.project.clone(),
                thought_id: Some(thought_id.clone()),
                done: false,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            let _ = db::insert_task(&conn, &task);
        }

        // tasks from a parsed direct command ("remind me to …")
        if let Some(cmd) = &command {
            if cmd.intent == "create_task" {
                if let Some(title) = cmd.title.as_deref().map(str::trim).filter(|t| !t.is_empty())
                {
                    let task = db::Task {
                        id: Uuid::new_v4().to_string(),
                        title: title.to_string(),
                        project: None,
                        thought_id: Some(thought_id.clone()),
                        done: false,
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };
                    let _ = db::insert_task(&conn, &task);
                }
            }
        }

        // Fog signals → meta (today's slice, capped) for the Today brief.
        if !fog_signals.is_empty() {
            let day = time::local_day_key();
            let mut sigs: Vec<serde_json::Value> =
                db::get_meta(&conn, "fog_today")
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
            sigs.retain(|v| v["day"].as_str() == Some(day.as_str()));
            for f in &fog_signals {
                sigs.push(serde_json::json!({
                    "day": day,
                    "tag": f.tag,
                    "context": f.context,
                    "intensity": f.intensity,
                    "question": f.question,
                }));
            }
            sigs.truncate(10);
            let _ = db::set_meta(
                &conn,
                "fog_today",
                &serde_json::to_string(&sigs).unwrap_or_default(),
            );
        }

        // backfill the summary
        if let Some(summary) = &annotation.summary {
            let _ = conn.execute(
                "UPDATE thoughts SET summary = ?2 WHERE id = ?1",
                rusqlite::params![thought_id, summary],
            );
        }
    }

    state.emit("thoughts-updated", &serde_json::json!({}));
    state.emit("tasks-updated", &serde_json::json!({}));
    state.emit(
        "capture-complete",
        &serde_json::json!({ "thoughtId": thought_id }),
    );
    stage(&state, "done");

    Ok(CaptureResult {
        thought_id,
        transcript,
        summary: annotation.summary.clone(),
        tasks,
        topics: annotation.topics,
        people: annotation.people,
        related: annotation.related,
        command,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_capture_is_20_minutes() {
        assert_eq!(MAX_CAPTURE_BYTES, 20 * 60 * 16_000 * 2);
    }
}
