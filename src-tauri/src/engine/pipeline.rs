//! The capture pipeline: what was said → things worth keeping.
//!
//! ```text
//! transcript (live, audio::session)
//!     │
//!     ├─ sort()      rules: tasks · ideas · learnings · questions · decisions
//!     ├─ fog         only when its cues appear (engine::fog)
//!     │      nothing kept? → the recording is marked noise and stops here
//!     ├─ annotate()  LLM: topics · people · goals of what was kept
//!     ├─ tasks       → Today (duplicates skipped, engine::tasks)
//!     ├─ items       → items table + mind-map nodes, linked to their topics
//!     └─ EmbeddingGemma 2: search index + "similar" links between items
//! ```
//!
//! Meetings get minutes; "You" action items become tasks, decisions become
//! items, and the user's own lines are sorted like a note.

use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use crate::audio::session::CaptureMode;
use crate::engine::fog::{self, FogSignal};
use crate::engine::meeting::{self, Minutes};
use crate::engine::sort::{self, Kind};
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{db, graph, vec_};
use crate::time;
pub use crate::engine::tasks::TaskRef;

/// What a recording is about (LLM).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub topics: Vec<String>,
    pub people: Vec<String>,
    pub goals: Vec<String>,
    pub sentiment: String,
}

/// An idea / learning / question / decision kept from a recording.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub id: String,
    pub kind: String,
    pub text: String,
}

/// What the UI shows after a capture completes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub thought_id: String,
    pub transcript: String,
    pub summary: Option<String>,
    /// Tasks, already added to Today (the UI offers undo).
    pub tasks: Vec<TaskRef>,
    /// Ideas, learnings, questions and decisions.
    pub items: Vec<ItemRef>,
    pub topics: Vec<String>,
    pub people: Vec<String>,
    /// Where the thinking seemed stuck, if anywhere.
    pub fog: Vec<FogSignal>,
    /// Nothing worth keeping (small talk, a test): hidden from Today and the map.
    pub noise: bool,
    pub duration_secs: i64,
    pub mode: CaptureMode,
    /// Meeting minutes (meeting mode only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<Minutes>,
}

fn stage(state: &AppState, stage: &str) {
    state.emit("capture-stage", &serde_json::json!({ "stage": stage, "ts": time::now_millis() }));
}

/// Longest single capture; the session stops itself after this.
pub const MAX_CAPTURE_SECS: u64 = 20 * 60;

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>> {
    state.db.lock().map_err(|_| Error::Store("db poisoned".into()))
}

/// Organize a finished transcript. Transcription already happened live
/// (see `audio::session`), so this is the only wait after the user stops.
pub async fn organize(
    state: Arc<AppState>,
    transcript: String,
    language: String,
    started_at_ms: i64,
    duration_secs: i64,
    mode: CaptureMode,
) -> Result<CaptureResult> {
    let transcript = transcript.trim().to_string();
    let mut result = CaptureResult {
        thought_id: Uuid::new_v4().to_string(),
        transcript: transcript.clone(),
        summary: None,
        tasks: Vec::new(),
        items: Vec::new(),
        topics: Vec::new(),
        people: Vec::new(),
        fog: Vec::new(),
        noise: false,
        duration_secs,
        mode,
        minutes: None,
    };
    if transcript.len() < 2 {
        result.transcript = String::new(); // UI shows "didn't catch any speech"
        stage(&state, "done");
        return Ok(result);
    }

    // 1) Persist the recording first (it survives even if a later step fails).
    let said_at = chrono::DateTime::from_timestamp_millis(started_at_ms)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let thought_id = result.thought_id.clone();
    db::insert_thought(
        &*lock(&state)?,
        &db::Thought {
            id: thought_id.clone(),
            text: transcript.clone(),
            summary: None,
            language,
            day: time::local_day_key(),
            started_at: said_at.clone(),
            duration_secs,
            created_at: chrono::Utc::now().to_rfc3339(),
            kind: match mode {
                CaptureMode::Note => "note",
                CaptureMode::Meeting => "meeting",
            }
            .to_string(),
            minutes: None,
            fog: None,
            noise: false,
        },
    )?;

    // 2) Sort what was said (and, for meetings, write the minutes).
    stage(&state, "organizing");
    let (sorted, minutes, own_words) = match mode {
        CaptureMode::Note => (sort::sort(&transcript), None, transcript.clone()),
        CaptureMode::Meeting => {
            let minutes = meeting::minutes(&transcript).await.unwrap_or_else(|e| {
                eprintln!("[meeting] minutes failed: {e}");
                Minutes { title: "Meeting".into(), ..Minutes::default() }
            });
            let mine: String = transcript.lines().filter_map(|l| l.strip_prefix("You: ")).collect::<Vec<_>>().join(" ");
            // Action items come from the minutes; the user's own lines add ideas,
            // learnings and questions; decisions come from the minutes.
            let mut sorted: Vec<sort::Item> = minutes
                .action_items
                .iter()
                .filter(|a| a.is_mine())
                .map(|a| sort::Item {
                    kind: Kind::Task,
                    text: if a.due.is_empty() { a.task.clone() } else { format!("{} ({})", a.task, a.due) },
                })
                .chain(minutes.decisions.iter().map(|d| sort::Item { kind: Kind::Decision, text: d.clone() }))
                .collect();
            sorted.extend(sort::sort(&mine).into_iter().filter(|i| !matches!(i.kind, Kind::Task | Kind::Decision)));
            (sorted, Some(minutes), mine)
        }
    };
    let fog_signals = fog::detect(&own_words).await;

    // Nothing worth keeping: mark it and stop — no tasks, no map, no index.
    if sorted.is_empty() && fog_signals.is_empty() && minutes.as_ref().is_none_or(|m| m.summary.is_empty()) {
        lock(&state)?
            .execute("UPDATE thoughts SET noise = 1 WHERE id = ?1", rusqlite::params![thought_id])
            .map_err(|e| Error::Store(e.to_string()))?;
        result.noise = true;
        state.emit("thoughts-updated", &serde_json::json!({}));
        stage(&state, "done");
        return Ok(result);
    }

    // 3) What it's about: annotate the kept statements (clean, short), or the
    // minutes / own words when nothing was itemised.
    let kept_text = sorted.iter().map(|i| i.text.as_str()).collect::<Vec<_>>().join(". ");
    let about = match &minutes {
        Some(m) => format!("{}. {}. {kept_text}", m.title, m.summary),
        None if kept_text.is_empty() => own_words.clone(),
        None => kept_text.clone(),
    };
    let annotation = crate::engine::llm::annotate(&about).await.unwrap_or_default();
    let summary = minutes
        .as_ref()
        .map(|m| m.summary.clone())
        .filter(|s| !s.is_empty())
        .or(annotation.summary.clone())
        .or_else(|| sorted.first().map(|i| i.text.clone()));

    // 4) Tasks → Today (duplicates of open tasks skipped).
    let task_titles: Vec<String> = sorted.iter().filter(|i| i.kind == Kind::Task).map(|i| i.text.clone()).collect();
    result.tasks = crate::engine::tasks::add_unique(&state, task_titles, Some(thought_id.clone()))
        .await
        .unwrap_or_else(|e| {
            eprintln!("[tasks] add failed: {e}");
            Vec::new()
        });

    // 5) Items → table + map nodes; entities; fog hubs.
    let entity_labels: Vec<(&str, &String)> = [("topic", &annotation.topics), ("person", &annotation.people), ("goal", &annotation.goals)]
        .into_iter()
        .flat_map(|(k, v)| v.iter().take(5).map(move |l| (k, l)))
        .collect();
    let entity_vecs = crate::engine::llm::embed(&entity_labels.iter().map(|(_, l)| (*l).clone()).collect::<Vec<_>>())
        .await
        .unwrap_or_default();
    {
        let conn = lock(&state)?;
        for i in sorted.iter().filter(|i| i.kind != Kind::Task) {
            let item = db::Item {
                id: Uuid::new_v4().to_string(),
                thought_id: thought_id.clone(),
                kind: i.kind.as_str().to_string(),
                text: i.text.clone(),
                created_at: said_at.clone(),
            };
            db::insert_item(&conn, &item)?;
            graph::upsert_item_node(&conn, &item.id, &item.kind, &item.text, &said_at)?;
            result.items.push(ItemRef { id: item.id, kind: item.kind, text: item.text });
        }

        let mut entity_ids = Vec::new();
        for (n, (kind, label)) in entity_labels.iter().enumerate() {
            let vec = entity_vecs.get(n).filter(|v| !v.is_empty());
            if let Some(v) = vec {
                let _ = vec_::insert(&conn, kind, label, crate::engine::search::EMBED_MODEL, v);
            }
            if let Some(id) = graph::upsert_entity(
                &conn,
                crate::engine::search::EMBED_MODEL,
                kind,
                label,
                &annotation.sentiment,
                vec.map(|v| v.as_slice()),
            )? {
                entity_ids.push(id);
            }
        }
        // Each item is about the recording's topics / people / goals.
        for item in &result.items {
            for e in &entity_ids {
                graph::upsert_edge(&conn, &item.id, e, "involves")?;
            }
        }
        // Fog hubs (one per kind) connect to what the fog was about.
        let fog_targets: Vec<&String> = if result.items.is_empty() {
            entity_ids.iter().collect()
        } else {
            result.items.iter().map(|i| &i.id).collect()
        };
        for f in &fog_signals {
            if let Some(hub) = graph::upsert_entity(&conn, crate::engine::search::EMBED_MODEL, "fog", &fog::label(&f.tag), "negative", None)? {
                for t in &fog_targets {
                    graph::upsert_edge(&conn, &hub, t, "fog")?;
                }
            }
        }

        conn.execute(
            "UPDATE thoughts SET summary = ?2, fog = ?3, minutes = ?4 WHERE id = ?1",
            rusqlite::params![
                thought_id,
                summary,
                (!fog_signals.is_empty()).then(|| serde_json::to_string(&fog_signals).unwrap_or_default()),
                minutes.as_ref().map(|m| serde_json::to_string(m).unwrap_or_default()),
            ],
        )
        .map_err(|e| Error::Store(e.to_string()))?;
    }

    // 6) EmbeddingGemma 2: index items + new tasks for search, then link
    // each new item to the closest things said before.
    let statements: Vec<(String, String, String)> = result
        .items
        .iter()
        .map(|i| ("item".to_string(), i.id.clone(), i.text.clone()))
        .chain(result.tasks.iter().map(|t| ("task".to_string(), t.id.clone(), t.title.clone())))
        .collect();
    if let Err(e) = crate::engine::search::index(&state, &statements).await {
        eprintln!("[search] indexing failed: {e}");
    }
    {
        let conn = lock(&state)?;
        for item in &result.items {
            let _ = crate::engine::links::link_similar(&conn, &item.id);
        }
    }

    state.emit("thoughts-updated", &serde_json::json!({}));
    state.emit("tasks-updated", &serde_json::json!({}));
    // `capture-complete` (with the full result) is emitted by audio::session.
    stage(&state, "done");

    result.summary = summary;
    result.topics = annotation.topics;
    result.people = annotation.people;
    result.fog = fog_signals;
    result.minutes = minutes;
    Ok(result)
}
