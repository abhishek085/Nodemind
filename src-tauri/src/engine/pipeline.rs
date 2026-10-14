//! The capture pipeline: voice → text → checklist + graph memory.
//!
//! ```text
//! mic → live utterances → whisper ─→ transcript   (audio::session)
//!                                         │
//!              ┌──────────────┬───────────┤
//!              ▼              ▼           ▼
//!          clean()       annotate()   embed()  (EmbeddingGemma 2)
//!        checklist      graph nodes   vector store
//!                             │           │
//!                             └─── merge (fuzzy, cosine ≥ 0.8)
//! ```
//!
//! The LLM passes run concurrently on llama-server's parallel slots; the
//! command is async so the UI never freezes.

use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{db, graph, vec_};
use crate::time;
use uuid::Uuid;

use crate::audio::session::CaptureMode;
use crate::engine::meeting::{self, Minutes};

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

/// A task created by a capture — enough for the UI to show and undo it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRef {
    pub id: String,
    pub title: String,
}

/// Normalized form for duplicate detection ("Call Priya." == "call priya").
pub(crate) fn task_key(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the UI shows after a capture completes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub thought_id: String,
    pub transcript: String,
    pub summary: Option<String>,
    /// Checklist items, already added to Today (the UI offers undo).
    pub tasks: Vec<TaskRef>,
    pub topics: Vec<String>,
    pub people: Vec<String>,
    pub related: Vec<String>,
    /// Parsed direct command, if the transcript looked like one ("remind me…",
    /// "start focus…"). The UI renders it as an actionable chip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<crate::engine::llm::CommandIntent>,
    pub duration_secs: i64,
    pub mode: CaptureMode,
    /// Meeting minutes (meeting mode only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<Minutes>,
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

/// Longest single capture; the session stops itself after this.
pub const MAX_CAPTURE_SECS: u64 = 20 * 60;

/// Organize a finished transcript: store it, run the LLM passes, file the
/// results into the graph, and add the checklist to Today.
///
/// Transcription already happened live during the capture
/// (see `audio::session`), so this is the only wait after the user stops.
/// Meetings get minutes instead of a brain-dump checklist; their action
/// items for "You" become the tasks.
pub async fn organize(
    state: Arc<AppState>,
    transcript: String,
    language: String,
    started_at_ms: i64,
    duration_secs: i64,
    mode: CaptureMode,
) -> Result<CaptureResult> {
    let transcript = transcript.trim().to_string();
    if transcript.len() < 2 {
        stage(&state, "done");
        return Ok(CaptureResult {
            thought_id: Uuid::new_v4().to_string(),
            transcript: String::new(), // UI shows "didn't catch any speech"
            summary: None,
            tasks: Vec::new(),
            topics: Vec::new(),
            people: Vec::new(),
            related: Vec::new(),
            command: None,
            duration_secs,
            mode,
            minutes: None,
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
        kind: match mode {
            CaptureMode::Note => "note",
            CaptureMode::Meeting => "meeting",
        }
        .to_string(),
        minutes: None,
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

    // 4) Recent thoughts as linking context for the annotator.
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

    // 5) All LLM passes run concurrently — llama-server decodes them in
    // parallel slots, so wall time is the slowest call, not the sum.
    stage(&state, "organizing");
    let (checklist, annotation, command, fog_signals, minutes) = match mode {
        CaptureMode::Note => {
            //  - checklist cleanup (the flagship feature)
            //  - graph annotation
            //  - direct-command parse, only for transcripts that *look* like one
            //  - Cognitive Mirror friction signals, only for longer transcripts
            let wants_command = looks_like_command(&transcript);
            let wants_fog = transcript.chars().count() > 200;
            let (checklist, annotation, command, fog_signals) = tokio::join!(
                async { crate::engine::llm::clean_checklist(&transcript).await.unwrap_or_default() },
                async { crate::engine::llm::annotate(&transcript, &pairs).await.unwrap_or_default() },
                async {
                    if !wants_command {
                        return None;
                    }
                    crate::engine::llm::parse_command(&transcript)
                        .await
                        .ok()
                        .filter(|c| c.intent != "unknown")
                },
                async {
                    if !wants_fog {
                        return Vec::new();
                    }
                    crate::engine::llm::fog(&transcript).await.unwrap_or_default()
                },
            );
            (checklist, annotation, command, fog_signals, None)
        }
        CaptureMode::Meeting => {
            // Minutes first (map-reduce over the whole call), then the graph
            // annotation works from the minutes, which fit the model's context.
            let minutes = meeting::minutes(&transcript).await.unwrap_or_else(|e| {
                eprintln!("[meeting] minutes failed: {e}");
                Minutes { title: "Meeting".into(), ..Minutes::default() }
            });
            let digest = format!(
                "{}\n{}\nDecisions: {}\nAction items: {}",
                minutes.title,
                minutes.summary,
                minutes.decisions.join("; "),
                minutes
                    .action_items
                    .iter()
                    .map(|a| format!("{} — {}", a.owner, a.task))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            let mut annotation = crate::engine::llm::annotate(&digest, &pairs).await.unwrap_or_default();
            annotation.summary = Some(minutes.summary.clone()).filter(|s| !s.is_empty()).or(annotation.summary);
            let mine: Vec<String> = minutes
                .action_items
                .iter()
                .filter(|a| a.is_mine())
                .map(|a| if a.due.is_empty() { a.task.clone() } else { format!("{} ({})", a.task, a.due) })
                .collect();
            (mine, annotation, None, Vec::new(), Some(minutes))
        }
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
    let mut tasks: Vec<TaskRef> = Vec::new();
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

        // The cleaned checklist goes straight onto Today, skipping anything
        // already open. (The annotator's task list only feeds the graph.)
        let mut seen: std::collections::HashSet<String> = db::get_tasks(&conn, false)
            .unwrap_or_default()
            .iter()
            .map(|t| task_key(&t.title))
            .collect();
        let command_task = command
            .as_ref()
            .filter(|c| c.intent == "create_task")
            .and_then(|c| c.title.as_deref())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        for title in checklist.iter().cloned().chain(command_task) {
            if !seen.insert(task_key(&title)) {
                continue;
            }
            let task = db::Task {
                id: Uuid::new_v4().to_string(),
                title: title.clone(),
                project: None,
                thought_id: Some(thought_id.clone()),
                done: false,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            if db::insert_task(&conn, &task).is_ok() {
                tasks.push(TaskRef { id: task.id, title });
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

        if let Some(m) = &minutes {
            let _ = conn.execute(
                "UPDATE thoughts SET minutes = ?2 WHERE id = ?1",
                rusqlite::params![thought_id, serde_json::to_string(m).unwrap_or_default()],
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

    // Passage index for memory search (keyword-only if embedding fails).
    if let Err(e) = crate::engine::search::index_thought(&state, &thought_id, &transcript).await {
        eprintln!("[search] indexing failed: {e}");
    }

    state.emit("thoughts-updated", &serde_json::json!({}));
    state.emit("tasks-updated", &serde_json::json!({}));
    state.emit(
        "capture-complete",
        &serde_json::json!({ "thoughtId": thought_id }),
    );
    stage(&state, "done");
    crate::commands::today::refresh_brief_in_background(state.clone());

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
        mode,
        minutes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_key_ignores_case_and_punctuation() {
        assert_eq!(task_key("Call Priya."), task_key("call   priya"));
        assert_ne!(task_key("Call Priya"), task_key("Call Rahul"));
    }
}
