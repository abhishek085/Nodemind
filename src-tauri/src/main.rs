// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod db;
mod llm;
mod prompts;
mod pipeline;

use db::{open_db, init_schema, Task, Goal, Meeting, TranscriptChunk, FogEntry, FogEntryExtended};
use pipeline::ThoughtBlockBuffer;
use whisper_rs::{WhisperContext, WhisperContextParameters, FullParams, SamplingStrategy};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::collections::{BTreeSet, HashMap, HashSet};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use uuid::Uuid;
use chrono::{DateTime, Datelike, Duration, Local, Timelike, Utc};
use tauri::Emitter;
use tauri::Manager;

struct AppState {
    transcript: Arc<Mutex<String>>,
    language: Arc<Mutex<String>>,
    listening: Arc<AtomicBool>,
    stop_signal: Arc<AtomicBool>,
    thread_handle: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    active_meeting_id: Arc<Mutex<Option<String>>>,
    llm_model: Arc<Mutex<String>>,
    session_transcript: Arc<Mutex<String>>,
    model_loaded: Arc<AtomicBool>,
    ollama_available: Arc<AtomicBool>,
    last_ollama_warning_ts: Arc<Mutex<i64>>,
    last_ollama_autostart_ts: Arc<Mutex<i64>>,
    thought_buffer: ThoughtBlockBuffer,
}

fn save_buffered_live_note(model: &str, block: pipeline::ThoughtBlock, app: &tauri::AppHandle) {
    if block.combined_text.trim().is_empty() {
        let _ = app.emit("final-note-processing-finished", ());
        return;
    }

    let combined_text = block.combined_text.clone();
    let recent_topics = open_db()
        .ok()
        .and_then(|conn| db::get_last_active_topics(&conn).ok())
        .unwrap_or_default()
        .join(", ");

    let extracted = llm::five_min_extract(model, &combined_text, &recent_topics);
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&extracted) {
        let live_note_payload = &json["live_note"];
        let summary = live_note_payload["summary"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| combined_text.chars().take(160).collect::<String>());
        let highlights: Vec<String> = live_note_payload["highlights"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
            .unwrap_or_default();
        let ideas: Vec<String> = live_note_payload["ideas"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
            .unwrap_or_default();
        let note_tasks: Vec<String> = live_note_payload["tasks"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
            .unwrap_or_default();

        let live_note = db::LiveNote {
            id: Uuid::new_v4().to_string(),
            day_key: db::local_today_key(),
            window_started_at: block.started_at_rfc3339,
            window_ended_at: Utc::now().to_rfc3339(),
            summary: summary.clone(),
            highlights: highlights.clone(),
            ideas: ideas.clone(),
            tasks: note_tasks.clone(),
            fingerprint: db::fingerprint_live_note(&summary, &highlights, &ideas, &note_tasks),
            created_at: Utc::now().to_rfc3339(),
        };
        if let Ok(conn) = open_db() {
            let _ = db::insert_live_note(&conn, &live_note);
        }

        let _ = app.emit("live-notes-updated", ());
    }

    let _ = app.emit("final-note-processing-finished", ());
}

fn tokenize_for_rag(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter_map(|token| {
            let t = token.trim().to_lowercase();
            if t.len() >= 3 { Some(t) } else { None }
        })
        .collect()
}

fn overlap_score(query: &str, candidate: &str) -> usize {
    let q: BTreeSet<String> = tokenize_for_rag(query).into_iter().collect();
    let c: BTreeSet<String> = tokenize_for_rag(candidate).into_iter().collect();
    q.intersection(&c).count()
}

fn normalize_task_candidates(raw: &[String]) -> Vec<String> {
    let mut dedup = BTreeSet::new();
    for item in raw {
        let trimmed = item.trim();
        if trimmed.len() < 4 {
            continue;
        }
        dedup.insert(trimmed.to_string());
    }
    dedup.into_iter().collect()
}

fn slugify_label(label: &str) -> String {
    label
        .trim()
        .to_lowercase()
        .replace('&', " and ")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn canonical_entity_text(label: &str) -> String {
    let lowered = label.trim().to_lowercase();
    match lowered.as_str() {
        "db" => "database".to_string(),
        "repo" => "repository".to_string(),
        "prd" => "product requirement doc".to_string(),
        "api" => "api".to_string(),
        _ => lowered,
    }
}

fn is_subsequence(shorter: &str, longer: &str) -> bool {
    let mut it = longer.chars();
    for c in shorter.chars() {
        if !it.by_ref().any(|lc| lc == c) {
            return false;
        }
    }
    true
}

fn labels_alias_match(a: &str, b: &str) -> bool {
    let a_c = canonical_entity_text(a);
    let b_c = canonical_entity_text(b);

    if a_c == b_c {
        return true;
    }

    if a_c.contains(&b_c) || b_c.contains(&a_c) {
        return true;
    }

    let a_compact: String = a_c.chars().filter(|c| c.is_alphanumeric()).collect();
    let b_compact: String = b_c.chars().filter(|c| c.is_alphanumeric()).collect();
    if a_compact.len() >= 2 && a_compact.len() <= 4 && is_subsequence(&a_compact, &b_compact) {
        return true;
    }
    if b_compact.len() >= 2 && b_compact.len() <= 4 && is_subsequence(&b_compact, &a_compact) {
        return true;
    }

    false
}

fn normalize_goal_key(label: &str) -> String {
    canonical_entity_text(label)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_duplicate_goal_title(candidate: &str, existing_titles: &[String]) -> bool {
    let key = normalize_goal_key(candidate);
    existing_titles
        .iter()
        .any(|existing| labels_alias_match(&key, &normalize_goal_key(existing)))
}

fn resolve_existing_node_id(conn: &rusqlite::Connection, node_type: &str, label: &str) -> Option<String> {
    let existing = db::get_nodes(conn).ok()?;
    existing
        .into_iter()
        .filter(|n| n.node_type == node_type)
        .find(|n| labels_alias_match(&n.label, label))
        .map(|n| n.id)
}

fn resolve_existing_node_id_any_type(conn: &rusqlite::Connection, label: &str) -> Option<(String, String)> {
    let existing = db::get_nodes(conn).ok()?;
    existing
        .into_iter()
        .find(|n| labels_alias_match(&n.label, label))
        .map(|n| (n.id, n.node_type))
}

fn normalize_entity_key(label: &str) -> String {
    canonical_entity_text(label).trim().to_string()
}

fn normalize_category(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim().to_lowercase();
    match value.as_str() {
        "work" => Some("Work".to_string()),
        "wellness" => Some("Wellness".to_string()),
        "social" => Some("Social".to_string()),
        "growth" => Some("Growth".to_string()),
        _ => None,
    }
}

fn normalize_relation(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "blocks" => "blocks".to_string(),
        "works_with" => "works_with".to_string(),
        "mentions" => "mentions".to_string(),
        "triggers" => "triggers".to_string(),
        _ => "relates_to".to_string(),
    }
}

fn node_metadata_json(
    category: Option<&str>,
    signal_type: Option<&str>,
    status: Option<&str>,
    impact: Option<&str>,
    context: Option<&str>,
) -> Option<String> {
    let mut map = serde_json::Map::new();
    if let Some(cat) = normalize_category(category) {
        map.insert("category".to_string(), serde_json::Value::String(cat));
    }
    if let Some(sig) = signal_type.and_then(|s| {
        let v = s.trim().to_lowercase();
        if v == "intent" || v == "action" { Some(v) } else { None }
    }) {
        map.insert("signal_type".to_string(), serde_json::Value::String(sig));
    }
    if let Some(st) = status.and_then(|s| {
        let v = s.trim().to_lowercase();
        if v.is_empty() { None } else { Some(v) }
    }) {
        map.insert("status".to_string(), serde_json::Value::String(st));
    }
    if let Some(imp) = impact.and_then(|s| {
        let v = s.trim().to_lowercase();
        if v.is_empty() { None } else { Some(v) }
    }) {
        map.insert("impact".to_string(), serde_json::Value::String(imp));
    }
    if let Some(ctx) = context.and_then(|s| {
        let v = s.trim();
        if v.is_empty() { None } else { Some(v.to_string()) }
    }) {
        map.insert("context".to_string(), serde_json::Value::String(ctx));
    }

    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map).to_string())
    }
}

fn parse_impact_sets(json: &serde_json::Value) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let parse_set = |items: Option<&Vec<serde_json::Value>>| -> HashSet<String> {
        items
            .into_iter()
            .flat_map(|arr| arr.iter())
            .filter_map(|item| item.as_str())
            .map(normalize_entity_key)
            .filter(|s| !s.is_empty())
            .collect()
    };

    let hi = &json["high_impact"];
    (
        parse_set(hi["resolved_nodes"].as_array()),
        parse_set(hi["stressor_nodes"].as_array()),
        parse_set(hi["spark_nodes"].as_array()),
    )
}

/// Returns the node ID of an existing node (matching `node_type_filter`) whose label
/// shares ≥ `threshold` Jaccard token-overlap with `label`.
/// Used for leaf-merging (atoms) and branch-fattening (topics).
fn find_similar_atom(
    conn: &rusqlite::Connection,
    label: &str,
    node_type_filter: &str,
    threshold: f64,
) -> Option<String> {
    let q_tokens: BTreeSet<String> = tokenize_for_rag(label).into_iter().collect();
    if q_tokens.is_empty() {
        return None;
    }
    let nodes = db::get_nodes(conn).ok()?;
    nodes
        .into_iter()
        .filter(|n| n.node_type == node_type_filter)
        .find(|n| {
            let c_tokens: BTreeSet<String> = tokenize_for_rag(&n.label).into_iter().collect();
            if c_tokens.is_empty() {
                return false;
            }
            let intersection = q_tokens.intersection(&c_tokens).count();
            let denom = q_tokens.len().max(c_tokens.len());
            (intersection as f64 / denom as f64) >= threshold
        })
        .map(|n| n.id)
}

fn infer_node_type_from_label(
    label: &str,
    person_labels: &HashSet<String>,
    project_labels: &HashSet<String>,
    goal_labels: &HashSet<String>,
    task_labels: &HashSet<String>,
) -> String {
    let key = normalize_entity_key(label);
    if task_labels.contains(&key) {
        return "task".to_string();
    }
    if person_labels.contains(&key) {
        return "person".to_string();
    }
    if project_labels.contains(&key) {
        return "project".to_string();
    }
    if goal_labels.contains(&key) {
        return "goal".to_string();
    }
    "topic".to_string()
}

fn resolve_or_create_graph_node(
    conn: &rusqlite::Connection,
    label: &str,
    node_type_hint: &str,
) -> Option<(String, String)> {
    if let Some((id, ty)) = resolve_existing_node_id_any_type(conn, label) {
        return Some((id, ty));
    }

    let resolved_type = if node_type_hint.is_empty() { "topic" } else { node_type_hint };
    let node_id = format!("{}-{}", resolved_type, slugify_label(label));
    let node = db::GraphNode {
        id: node_id.clone(),
        node_type: resolved_type.to_string(),
        label: label.trim().to_string(),
        data: None,
        created_at: Utc::now().to_rfc3339(),
    };
    let _ = db::insert_node(conn, &node);
    Some((node_id, resolved_type.to_string()))
}

fn build_day_rag_context(candidates: &[String]) -> String {
    let Ok(conn) = open_db() else {
        return "No local context found for today.".to_string();
    };

    let day_key = db::local_today_key();
    let mut snippets: Vec<String> = Vec::new();

    if let Ok(notes) = db::get_live_notes_for_day(&conn, &day_key) {
        for note in notes {
            snippets.push(format!("[note-summary] {}", note.summary));
            for h in note.highlights {
                snippets.push(format!("[note-highlight] {}", h));
            }
            for idea in note.ideas {
                snippets.push(format!("[note-idea] {}", idea));
            }
            for task in note.tasks {
                snippets.push(format!("[note-task] {}", task));
            }
        }
    }

    if let Ok(chunks) = db::get_today_chunks(&conn) {
        for chunk in chunks.into_iter().rev().take(80) {
            let snippet = chunk.text.trim();
            if !snippet.is_empty() {
                snippets.push(format!("[chunk] {}", snippet));
            }
        }
    }

    if snippets.is_empty() {
        return "No local context found for today.".to_string();
    }

    let mut scored: Vec<(usize, String)> = Vec::new();
    for snippet in &snippets {
        let mut best = 0usize;
        for c in candidates {
            best = best.max(overlap_score(c, snippet));
        }
        scored.push((best, snippet.clone()));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    let mut selected = Vec::new();
    for (_, s) in scored.iter().take(20) {
        selected.push(s.clone());
    }
    if selected.is_empty() {
        return snippets.into_iter().take(20).collect::<Vec<_>>().join("\n");
    }
    selected.join("\n")
}

#[tauri::command]
fn get_latest_transcript(state: tauri::State<AppState>) -> String {
    state.transcript.lock().unwrap().clone()
}

#[tauri::command]
fn get_session_transcript(state: tauri::State<AppState>) -> String {
    state.session_transcript.lock().unwrap().clone()
}

#[tauri::command]
fn is_listening(state: tauri::State<AppState>) -> bool {
    state.listening.load(Ordering::Relaxed)
}

#[tauri::command]
fn set_language(state: tauri::State<AppState>, language: String) {
    let mut lang = state.language.lock().unwrap();
    *lang = match language.to_lowercase().as_str() {
        "hindi" | "hi" => "hi".to_string(),
        _ => "en".to_string(),
    };
}

#[tauri::command]
fn clear_transcript(state: tauri::State<AppState>) {
    state.transcript.lock().unwrap().clear();
    state.session_transcript.lock().unwrap().clear();
}

/// Returns all transcript text recorded since midnight today, joined into one string.
/// This lets the frontend restore the day's transcript on app launch.
#[tauri::command]
fn get_today_transcript() -> String {
    match open_db() {
        Ok(conn) => {
            match db::get_today_chunks(&conn) {
                Ok(chunks) => chunks.into_iter().map(|c| c.text).collect::<Vec<_>>().join(" "),
                Err(_) => String::new(),
            }
        }
        Err(_) => String::new(),
    }
}

/// Returns all transcript chunks for today with timestamps (for the UI).
#[tauri::command]
fn get_today_transcript_chunks() -> Vec<TranscriptChunk> {
    match open_db() {
        Ok(conn) => db::get_today_chunks(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn get_today_live_notes() -> Vec<db::LiveNote> {
    match open_db() {
        Ok(conn) => {
            let _ = db::archive_previous_live_notes(&conn);
            db::get_live_notes_for_day(&conn, &db::local_today_key()).unwrap_or_default()
        }
        Err(_) => vec![],
    }
}

#[tauri::command]
fn get_historical_notes() -> Vec<db::HistoricalNote> {
    match open_db() {
        Ok(conn) => {
            let _ = db::archive_previous_live_notes(&conn);
            db::get_historical_notes(&conn).unwrap_or_default()
        }
        Err(_) => vec![],
    }
}

#[tauri::command]
fn get_unprocessed_notes_count() -> i64 {
    match open_db() {
        Ok(conn) => db::count_unprocessed_notes_for_today(&conn).unwrap_or(0),
        Err(_) => 0,
    }
}

#[tauri::command]
fn start_listening(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> String {
    if state.listening.load(Ordering::Relaxed) {
        return "Already listening".into();
    }

    state.stop_signal.store(false, Ordering::Relaxed);

    let transcript_clone = Arc::clone(&state.transcript);
    let session_clone = Arc::clone(&state.session_transcript);
    let language_clone = Arc::clone(&state.language);
    let listening_clone = Arc::clone(&state.listening);
    let stop_signal_clone = Arc::clone(&state.stop_signal);
    let active_meeting_clone = Arc::clone(&state.active_meeting_id);
    let llm_model_clone = Arc::clone(&state.llm_model);
    let app_handle = app.clone();
    // Clone shares the same Arc-backed internal storage — buffer is shared across threads.
    let thought_buffer_clone = state.thought_buffer.clone();
    let ollama_available_clone = Arc::clone(&state.ollama_available);
    let _last_ollama_warning_clone = Arc::clone(&state.last_ollama_warning_ts);

    let handle = std::thread::spawn(move || {
        listening_clone.store(true, Ordering::Relaxed);

        let model_path = {
            let exe = std::env::current_exe().unwrap_or_default();
            let base = exe.parent().unwrap_or(std::path::Path::new("."));

            // 1) Bundled macOS app location: <app>.app/Contents/MacOS
            //    model is in <app>.app/Contents/Resources/resources/models/...
            let candidate1 = base.join("resources/models/ggml-small.bin");
            let candidate2 = base.join("../Resources/resources/models/ggml-small.bin");

            if candidate1.exists() {
                candidate1.to_string_lossy().to_string()
            } else if candidate2.exists() {
                candidate2.to_string_lossy().to_string()
            } else {
                // 3) Development fallback (current working dir)
                "resources/models/ggml-small.bin".to_string()
            }
        };

        let mut open_params = WhisperContextParameters::new();
        open_params.use_gpu(true);

        // On some macOS setups, Core ML init can fail even when the model files exist.
        // Fallback to CPU so STT still works instead of aborting listening.
        let ctx = match WhisperContext::new_with_params(&model_path, open_params) {
            Ok(c) => c,
            Err(gpu_err) => {
                eprintln!(
                    "[nodemind] Whisper GPU/CoreML init failed ({}). Falling back to CPU.",
                    gpu_err
                );

                let mut cpu_params = WhisperContextParameters::new();
                cpu_params.use_gpu(false);

                match WhisperContext::new_with_params(&model_path, cpu_params) {
                    Ok(c) => {
                        let _ = app_handle.emit(
                            "error",
                            "Whisper GPU/CoreML unavailable, using CPU fallback (slower).",
                        );
                        c
                    }
                    Err(cpu_err) => {
                        let _ = app_handle.emit(
                            "error",
                            format!(
                                "Failed to load Whisper model (GPU/CoreML: {}, CPU fallback: {})",
                                gpu_err, cpu_err
                            ),
                        );
                        listening_clone.store(false, Ordering::Relaxed);
                        return;
                    }
                }
            }
        };

        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                let _ = app_handle.emit("error", "No microphone found".to_string());
                listening_clone.store(false, Ordering::Relaxed);
                return;
            }
        };
        let config = device.default_input_config().unwrap();
        let source_sample_rate = config.sample_rate() as f32;

        let audio_buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let buffer_writer = Arc::clone(&audio_buffer);

        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &_| {
                let mut buffer = buffer_writer.lock().unwrap();
                let step = source_sample_rate / 16000.0;
                let mut i = 0.0_f32;
                while (i as usize) < data.len() {
                    buffer.push(data[i as usize]);
                    i += step;
                }
                if buffer.len() > 16000 * 30 {
                    buffer.drain(0..16000 * 5);
                }
            },
            |_err| {},
            None,
        ).unwrap();

        stream.play().unwrap();

        // ── FIFO 5-minute extraction queue ────────────────────────────────────
        // Bounded channel: max 2 pending jobs. try_send drops the block if full
        // instead of stacking unlimited Ollama requests that all timeout.
        // The worker processes one job at a time (FIFO) → one inference in flight.
        // Job tuple: (model, combined_text, started_at_rfc3339)
        let (extract_tx, extract_rx) = std::sync::mpsc::sync_channel::<(String, String, String)>(2);

        {
            let extract_worker_app = app_handle.clone();
            let ollama_avail_for_worker = Arc::clone(&ollama_available_clone);
            std::thread::spawn(move || {
                while let Ok((model, combined_text, started_at_rfc)) = extract_rx.recv() {
                    if !ollama_avail_for_worker.load(Ordering::Relaxed) {
                        eprintln!("[nodemind] Ollama offline — skipping 5-min extraction block");
                        let _ = extract_worker_app.emit(
                            "error",
                            "LLM offline: skipping 5-min extraction window.",
                        );
                        continue;
                    }

                    let combined_word_count = combined_text.split_whitespace().count() as i64;
                    const MIN_WORDS_FOR_EXPECTED_EXTRACT: i64 = 25;

                    // ── Step 1: Dedicated fog classification call (before extraction) ──
                    // Runs fog_signals_only on the full 5-minute block to get
                    // speech-pattern-based fog — not per-chunk noise.
                    {
                        let fog_text = combined_text.clone();
                        // Skip blank/very short blocks for fog
                        let word_count = fog_text.split_whitespace().count();
                        if word_count >= 15 {
                            let fog_result = llm::fog_signals_only(&model, &fog_text);
                            let ah = extract_worker_app.clone();
                            if let Ok(fog_json) = serde_json::from_str::<serde_json::Value>(&fog_result) {
                                if let Some(signals) = fog_json["signals"].as_array() {
                                    for sig in signals {
                                        let raw_tag = sig["tag"].as_str().unwrap_or("").trim();
                                        if raw_tag.is_empty() { continue; }
                                        let tag = raw_tag.to_lowercase();
                                        let context = sig["context"]
                                            .as_str()
                                            .map(|s| s.trim().to_string())
                                            .filter(|s| !s.is_empty())
                                            .unwrap_or_else(|| fog_text[..fog_text.len().min(200)].to_string());
                                        let intensity = sig["intensity"].as_f64().unwrap_or(0.45).clamp(0.0, 1.0);
                                        let clarity_question = sig["clarity_question"]
                                            .as_str()
                                            .map(|s| s.trim().to_string())
                                            .filter(|s| !s.is_empty())
                                            .unwrap_or_else(|| "What is the smallest concrete next step right now?".to_string());
                                        let ts = Utc::now().to_rfc3339();
                                        let entry = FogEntry {
                                            id: Uuid::new_v4().to_string(),
                                            tag: tag.clone(),
                                            context: context.clone(),
                                            timestamp: ts.clone(),
                                        };
                                        let extended = FogEntryExtended {
                                            id: Uuid::new_v4().to_string(),
                                            chunk_id: "5min-fog-block".to_string(),
                                            tag: tag.clone(),
                                            context: context.clone(),
                                            resolution: clarity_question,
                                            strength: intensity,
                                            markers: serde_json::json!([tag.clone()]).to_string(),
                                            timestamp: ts,
                                        };
                                        if let Ok(conn) = open_db() {
                                            let _ = db::insert_fog(&conn, &entry);
                                            let _ = db::insert_fog_extended(&conn, &extended);
                                            if let Some(subject) = pipeline::detect_recent_mental_loop(&conn) {
                                                let payload = serde_json::json!({ "subject": subject });
                                                let _ = ah.emit("mental-loop-detected", payload.to_string());
                                            }
                                        }
                                    }
                                    if !signals.is_empty() {
                                        let _ = ah.emit("fog-detected", fog_result);
                                    }
                                }
                            }
                        }
                    }

                    // ── Step 2: Deep extraction call ──────────────────────────
                    let recent_topics = open_db()
                        .and_then(|conn| db::get_last_active_topics(&conn))
                        .unwrap_or_default()
                        .join(", ");

                    let extracted = llm::five_min_extract(&model, &combined_text, &recent_topics);
                    let timestamp_now = Utc::now().to_rfc3339();
                    let ah = extract_worker_app.clone();

                    // Debug logging for empty/invalid extractions
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&extracted) {
                        let is_empty_arrays = json["tasks"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["goals"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["people"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["calendar_items"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["projects"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["connections"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["tray_suggestions"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["live_note"]["highlights"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["live_note"]["ideas"].as_array().map(|v| v.is_empty()).unwrap_or(true)
                            && json["live_note"]["tasks"].as_array().map(|v| v.is_empty()).unwrap_or(true);

                        if combined_word_count >= MIN_WORDS_FOR_EXPECTED_EXTRACT && is_empty_arrays {
                            if let Ok(conn) = open_db() {
                                let _ = db::insert_extractor_debug_run(
                                    &conn,
                                    &Uuid::new_v4().to_string(),
                                    &timestamp_now,
                                    combined_word_count,
                                    "five_min_extract_empty",
                                    &extracted,
                                );
                            }
                            let _ = ah.emit(
                                "error",
                                "five_min_extract returned empty arrays despite enough transcript. Saved debug payload.",
                            );
                        }
                    } else if combined_word_count >= MIN_WORDS_FOR_EXPECTED_EXTRACT {
                        if let Ok(conn) = open_db() {
                            let _ = db::insert_extractor_debug_run(
                                &conn,
                                &Uuid::new_v4().to_string(),
                                &timestamp_now,
                                combined_word_count,
                                "five_min_extract_invalid_json",
                                &extracted,
                            );
                        }
                        let _ = ah.emit(
                            "error",
                            "five_min_extract returned invalid JSON. Saved debug payload.",
                        );
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&extracted) {
                        let mut person_labels: HashSet<String> = HashSet::new();
                        let mut project_labels: HashSet<String> = HashSet::new();
                        let mut goal_labels: HashSet<String> = HashSet::new();
                        let mut task_labels: HashSet<String> = HashSet::new();
                        let mut edge_node_cache: HashMap<String, (String, String)> = HashMap::new();
                        let (resolved_nodes, stressor_nodes, spark_nodes) = parse_impact_sets(&json);

                        let live_note_payload = &json["live_note"];

                        let summary = live_note_payload["summary"]
                            .as_str()
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                            .unwrap_or_else(|| combined_text.chars().take(160).collect::<String>());
                        let highlights: Vec<String> = live_note_payload["highlights"]
                            .as_array()
                            .map(|items| items.iter().filter_map(|item| item.as_str().map(|value| value.trim().to_string())).filter(|value| !value.is_empty()).collect())
                            .unwrap_or_default();
                        let ideas: Vec<String> = live_note_payload["ideas"]
                            .as_array()
                            .map(|items| items.iter().filter_map(|item| item.as_str().map(|value| value.trim().to_string())).filter(|value| !value.is_empty()).collect())
                            .unwrap_or_default();
                        let note_tasks: Vec<String> = live_note_payload["tasks"]
                            .as_array()
                            .map(|items| items.iter().filter_map(|item| item.as_str().map(|value| value.trim().to_string())).filter(|value| !value.is_empty()).collect())
                            .unwrap_or_else(|| {
                                json["tasks"]
                                    .as_array()
                                    .map(|items| {
                                        items.iter()
                                            .filter_map(|item| item["title"].as_str().map(|value| value.trim().to_string()))
                                            .filter(|value| !value.is_empty())
                                            .collect()
                                    })
                                    .unwrap_or_default()
                            });

                        let live_note = db::LiveNote {
                            id: Uuid::new_v4().to_string(),
                            day_key: db::local_today_key(),
                            window_started_at: started_at_rfc.clone(),
                            window_ended_at: Utc::now().to_rfc3339(),
                            summary: summary.clone(),
                            highlights: highlights.clone(),
                            ideas: ideas.clone(),
                            tasks: note_tasks.clone(),
                            fingerprint: db::fingerprint_live_note(&summary, &highlights, &ideas, &note_tasks),
                            created_at: Utc::now().to_rfc3339(),
                        };
                        if let Ok(conn) = open_db() {
                            let _ = db::insert_live_note(&conn, &live_note);
                        }

                        // ── Tasks: candidate extraction -> RAG -> LLM approval -> pending suggestions ──
                        let mut candidate_tasks: Vec<String> = Vec::new();
                        if let Some(tasks) = json["tasks"].as_array() {
                            for t in tasks {
                                if let Some(title) = t["title"].as_str() {
                                    candidate_tasks.push(title.to_string());
                                    task_labels.insert(normalize_entity_key(title));

                                    if let Ok(conn) = open_db() {
                                        let node_id = resolve_existing_node_id(&conn, "task", title)
                                            .unwrap_or_else(|| format!("task-{}", slugify_label(title)));
                                        let impact = {
                                            let key = normalize_entity_key(title);
                                            if resolved_nodes.contains(&key) {
                                                Some("resolved")
                                            } else if stressor_nodes.contains(&key) {
                                                Some("stressor")
                                            } else if spark_nodes.contains(&key) {
                                                Some("spark")
                                            } else {
                                                None
                                            }
                                        };
                                        let node = db::GraphNode {
                                            id: node_id,
                                            node_type: "task".to_string(),
                                            label: title.to_string(),
                                            data: node_metadata_json(
                                                t["category"].as_str(),
                                                t["signal_type"].as_str().or(Some("action")),
                                                None,
                                                impact,
                                                t["project"].as_str(),
                                            ),
                                            created_at: Utc::now().to_rfc3339(),
                                        };
                                        let _ = db::insert_node(&conn, &node);
                                        if let Ok(conn) = open_db() {
                                            let impact = {
                                                let key = normalize_entity_key(title);
                                                if resolved_nodes.contains(&key) { Some("resolved") }
                                                else if stressor_nodes.contains(&key) { Some("stressor") }
                                                else if spark_nodes.contains(&key) { Some("spark") }
                                                else { None }
                                            };
                                            // Leaf merging: ≥90% token-similar task → pulse existing node (+1 badge)
                                            if let Some(existing_id) = find_similar_atom(&conn, title, "task", 0.9) {
                                                let _ = db::increment_node_field(&conn, &existing_id, "mentions_count", 1);
                                            } else {
                                                let node_id = format!("task-{}", slugify_label(title));
                                                let node = db::GraphNode {
                                                    id: node_id,
                                                    node_type: "task".to_string(),
                                                    label: title.to_string(),
                                                    data: node_metadata_json(
                                                        t["category"].as_str(),
                                                        t["signal_type"].as_str().or(Some("action")),
                                                        None,
                                                        impact,
                                                        t["project"].as_str(),
                                                    ),
                                                    created_at: Utc::now().to_rfc3339(),
                                                };
                                                let _ = db::insert_node(&conn, &node);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // ── Goals → labels only (persisted in 4h/manual batch only) ──
                        if let Some(goals) = json["goals"].as_array() {
                            for g in goals {
                                if let Some(title) = g["title"].as_str() {
                                    if title.len() > 3 {
                                        goal_labels.insert(normalize_entity_key(title));
                                        if let Ok(conn) = open_db() {
                                            let node_id = resolve_existing_node_id(&conn, "goal", title)
                                                .unwrap_or_else(|| format!("goal-{}", slugify_label(title)));
                                            let impact = {
                                                let key = normalize_entity_key(title);
                                                if resolved_nodes.contains(&key) {
                                                    Some("resolved")
                                                } else if stressor_nodes.contains(&key) {
                                                    Some("stressor")
                                                } else if spark_nodes.contains(&key) {
                                                    Some("spark")
                                                } else {
                                                    None
                                                }
                                            };
                                            let node = db::GraphNode {
                                                id: node_id,
                                                node_type: "goal".to_string(),
                                                label: title.to_string(),
                                                data: node_metadata_json(
                                                    g["category"].as_str(),
                                                    g["signal_type"].as_str().or(Some("intent")),
                                                    None,
                                                    impact,
                                                    g["description"].as_str(),
                                                ),
                                                created_at: Utc::now().to_rfc3339(),
                                            };
                                            let _ = db::insert_node(&conn, &node);
                                        }
                                    }
                                }
                            }
                        }

                        // ── People → graph nodes ──
                        if let Some(people) = json["people"].as_array() {
                            for p in people {
                                if let Some(name) = p["name"].as_str() {
                                    if name.len() > 1 {
                                        person_labels.insert(normalize_entity_key(name));
                                        if let Ok(conn) = open_db() {
                                            let node_id = resolve_existing_node_id(&conn, "person", name)
                                                .unwrap_or_else(|| format!("person-{}", slugify_label(name)));
                                            let node = db::GraphNode {
                                                id: node_id,
                                                node_type: "person".to_string(),
                                                label: name.to_string(),
                                                data: {
                                                    let key = normalize_entity_key(name);
                                                    let impact = if resolved_nodes.contains(&key) {
                                                        Some("resolved")
                                                    } else if stressor_nodes.contains(&key) {
                                                        Some("stressor")
                                                    } else if spark_nodes.contains(&key) {
                                                        Some("spark")
                                                    } else {
                                                        None
                                                    };
                                                    node_metadata_json(
                                                        Some("Social"),
                                                        None,
                                                        None,
                                                        impact,
                                                        p["context"].as_str(),
                                                    )
                                                },
                                                created_at: Utc::now().to_rfc3339(),
                                            };
                                            let _ = db::insert_node(&conn, &node);
                                        }
                                    }
                                }
                            }
                        }

                        // ── Projects → graph nodes ──
                        if let Some(projects) = json["projects"].as_array() {
                            for proj in projects {
                                let (name, category) = if let Some(n) = proj.as_str() {
                                    (n.to_string(), None)
                                } else {
                                    (
                                        proj["name"].as_str().unwrap_or_default().to_string(),
                                        proj["category"].as_str(),
                                    )
                                };
                                if name.len() > 1 {
                                    project_labels.insert(normalize_entity_key(&name));
                                    if let Ok(conn) = open_db() {
                                        let node_id = resolve_existing_node_id(&conn, "project", &name)
                                            .unwrap_or_else(|| format!("project-{}", slugify_label(&name)));
                                        let impact = {
                                            let key = normalize_entity_key(&name);
                                            if resolved_nodes.contains(&key) {
                                                Some("resolved")
                                            } else if stressor_nodes.contains(&key) {
                                                Some("stressor")
                                            } else if spark_nodes.contains(&key) {
                                                Some("spark")
                                            } else {
                                                None
                                            }
                                        };
                                        let node = db::GraphNode {
                                            id: node_id,
                                            node_type: "project".to_string(),
                                            label: name,
                                            data: node_metadata_json(category, None, None, impact, None),
                                            created_at: Utc::now().to_rfc3339(),
                                        };
                                        let _ = db::insert_node(&conn, &node);
                                    }
                                }
                            }
                        }

                        // ── Topics → graph nodes ──
                        if let Some(topics) = json["topics"].as_array() {
                            for topic in topics {
                                let name = topic["name"].as_str().unwrap_or_default();
                                if name.len() <= 1 {
                                    continue;
                                }
                                if let Ok(conn) = open_db() {
                                    let impact = {
                                        let key = normalize_entity_key(name);
                                        if resolved_nodes.contains(&key) { Some("resolved") }
                                        else if stressor_nodes.contains(&key) { Some("stressor") }
                                        else if spark_nodes.contains(&key) { Some("spark") }
                                        else { None }
                                    };
                                    // Branch fattening: if topic already exists, increment mentions_count
                                    if let Some(existing_id) = resolve_existing_node_id(&conn, "topic", name) {
                                        let _ = db::increment_node_field(&conn, &existing_id, "mentions_count", 1);
                                    } else {
                                        let node_id = format!("topic-{}", slugify_label(name));
                                        let node = db::GraphNode {
                                            id: node_id,
                                            node_type: "topic".to_string(),
                                            label: name.to_string(),
                                            data: node_metadata_json(topic["category"].as_str(), None, None, impact, None),
                                            created_at: Utc::now().to_rfc3339(),
                                        };
                                        let _ = db::insert_node(&conn, &node);
                                    }
                                }
                            }
                        }

                        // ── Mental map connections → graph edges ──
                        if let Some(connections) = json["connections"].as_array() {
                            if let Ok(conn) = open_db() {
                                for conn_val in connections {
                                    if let (Some(from), Some(to), Some(rel)) = (
                                        conn_val["from"].as_str(),
                                        conn_val["to"].as_str(),
                                        conn_val["relation"].as_str(),
                                    ) {
                                        if from.trim().is_empty() || to.trim().is_empty() {
                                            continue;
                                        }

                                        let from_key = normalize_entity_key(from);
                                        let to_key = normalize_entity_key(to);

                                        let from_resolved = if let Some(found) = edge_node_cache.get(&from_key) {
                                            Some(found.clone())
                                        } else {
                                            let inferred = infer_node_type_from_label(
                                                from,
                                                &person_labels,
                                                &project_labels,
                                                &goal_labels,
                                                &task_labels,
                                            );
                                            let resolved = resolve_or_create_graph_node(&conn, from, &inferred);
                                            if let Some(ref item) = resolved {
                                                edge_node_cache.insert(from_key.clone(), item.clone());
                                            }
                                            resolved
                                        };

                                        let to_resolved = if let Some(found) = edge_node_cache.get(&to_key) {
                                            Some(found.clone())
                                        } else {
                                            let inferred = infer_node_type_from_label(
                                                to,
                                                &person_labels,
                                                &project_labels,
                                                &goal_labels,
                                                &task_labels,
                                            );
                                            let resolved = resolve_or_create_graph_node(&conn, to, &inferred);
                                            if let Some(ref item) = resolved {
                                                edge_node_cache.insert(to_key.clone(), item.clone());
                                            }
                                            resolved
                                        };

                                        if let (Some((from_id, _)), Some((to_id, _))) = (from_resolved, to_resolved) {
                                            let direction = conn_val["direction"].as_str().unwrap_or("from_to");
                                            let (final_from, final_to) = if direction == "to_from" {
                                                (to_id, from_id)
                                            } else {
                                                (from_id, to_id)
                                            };
                                            let edge = db::GraphEdge {
                                                id: Uuid::new_v4().to_string(),
                                                from_node: final_from,
                                                to_node: final_to,
                                                edge_type: normalize_relation(rel),
                                                weight: 1.0,
                                                last_updated: Utc::now().to_rfc3339(),
                                            };
                                            let _ = db::insert_edge(&conn, &edge);
                                        }
                                    }
                                }
                            }
                        }

                        // ── Intent vs Action links (goal -> task) ──
                        if let (Some(goals_arr), Some(tasks_arr), Ok(conn)) = (
                            json["goals"].as_array(),
                            json["tasks"].as_array(),
                            open_db(),
                        ) {
                            for g in goals_arr {
                                let Some(goal_title) = g["title"].as_str() else { continue; };
                                let Some(goal_id) = resolve_existing_node_id(&conn, "goal", goal_title) else { continue; };

                                for t in tasks_arr {
                                    let Some(task_title) = t["title"].as_str() else { continue; };
                                    let Some(task_id) = resolve_existing_node_id(&conn, "task", task_title) else { continue; };
                                    let project = t["project"].as_str().unwrap_or_default();

                                    let overlap = overlap_score(goal_title, task_title)
                                        .max(overlap_score(goal_title, project));
                                    if overlap == 0 {
                                        continue;
                                    }

                                    let edge = db::GraphEdge {
                                        id: Uuid::new_v4().to_string(),
                                        from_node: goal_id.clone(),
                                        to_node: task_id,
                                        edge_type: "triggers".to_string(),
                                        weight: 1.0 + (overlap as f64 * 0.2),
                                        last_updated: Utc::now().to_rfc3339(),
                                    };
                                    let _ = db::insert_edge(&conn, &edge);
                                }
                            }
                        }

                        // ── Calendar items → collect candidate tasks for future batch ──
                        if let Some(cal_items) = json["calendar_items"].as_array() {
                            for item in cal_items {
                                if let Some(title) = item["title"].as_str() {
                                    if title.len() > 3 {
                                        candidate_tasks.push(title.to_string());
                                    }
                                }
                            }
                        }

                        // NOTE: Suggestion generation is intentionally NOT done here.
                        // It is batched every 4 hours (or on manual refresh) by
                        // run_batch_analysis(), which processes only unread notes and
                        // deduplicates against today's existing suggestions.

                        let _ = ah.emit("five-min-extract-done", extracted);
                        let _ = ah.emit("live-notes-updated", ());
                        let _ = ah.emit("tasks-updated", ());
                        let _ = ah.emit("mental-map-updated", ());
                    }
                }
            });
        }

        let chunk_size = 16000 * 2;
        let mut silence_counter = 0u32;

        while !stop_signal_clone.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(300));

            let mut chunk_to_process = Vec::new();
            {
                let mut buffer = audio_buffer.lock().unwrap();
                if buffer.len() >= chunk_size {
                    chunk_to_process = buffer.drain(..chunk_size).collect();
                }
            }

            if chunk_to_process.is_empty() {
                silence_counter += 1;
                if silence_counter >= 75 {
                    let _ = app_handle.emit("listening-idle", ());
                    break;
                }
                continue;
            }
            silence_counter = 0;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(4);
            params.set_no_context(true);
            params.set_single_segment(true);

            let curr_lang = language_clone.lock().unwrap().clone();
            params.set_language(Some(&curr_lang));
            if curr_lang == "hi" {
                params.set_initial_prompt(&prompts::get().stt.hindi_initial_prompt);
                params.set_translate(true);
            }

            let mut wstate = ctx.create_state().expect("failed to create whisper state");
            if let Ok(()) = wstate.full(params, &chunk_to_process) {
                let mut segment_text = String::new();
                let n = wstate.full_n_segments();
                for i in 0..n {
                    if let Some(seg) = wstate.get_segment(i) {
                        if let Ok(text) = seg.to_str() {
                            segment_text.push_str(text);
                        }
                    }
                }

                let trimmed = segment_text.trim().to_string();
                if !trimmed.is_empty() && !trimmed.contains("[_") && trimmed.len() > 3 {
                    {
                        let mut t = transcript_clone.lock().unwrap();
                        t.push_str(&format!("{} ", trimmed));
                    }
                    {
                        let mut s = session_clone.lock().unwrap();
                        s.push_str(&format!("{} ", trimmed));
                    }

                    // Push transcript to UI immediately; keep analysis on background paths.
                    let _ = app_handle.emit("transcript-chunk", trimmed.clone());

                    let meeting_id = active_meeting_clone.lock().unwrap().clone();
                    let chunk = TranscriptChunk {
                        id: Uuid::new_v4().to_string(),
                        text: trimmed.clone(),
                        language: curr_lang.clone(),
                        timestamp: Utc::now().to_rfc3339(),
                        meeting_id: meeting_id.clone(),
                        fog_tags: None,
                    };
                    if let Ok(conn) = open_db() {
                        let _ = db::insert_chunk(&conn, &chunk);
                    }

                    // All chunks are treated as ambient speech.
                    // No per-chunk LLM calls — just accumulate in the 5-min buffer.
                    // The FIFO worker above fires fog + extraction every 5 minutes.
                    let should_flush = thought_buffer_clone.add_segment(&trimmed);
                    if should_flush {
                        if let Some(block) = thought_buffer_clone.flush() {
                            let word_count = block.combined_text.split_whitespace().count();
                            if word_count >= 10 {
                                let job = (
                                    llm_model_clone.lock().unwrap().clone(),
                                    block.combined_text,
                                    block.started_at_rfc3339,
                                );
                                if extract_tx.try_send(job).is_err() {
                                    let _ = app_handle.emit(
                                        "error",
                                        "5-min extract queue full — window dropped (Ollama may be slow).",
                                    );
                                    eprintln!("[nodemind] 5-min extract queue full, dropping block ({} words)", word_count);
                                }
                            }
                        }
                    }
                }
            }
        }

        listening_clone.store(false, Ordering::Relaxed);
    });

    *state.thread_handle.lock().unwrap() = Some(handle);
    "Listening started".into()
}

#[tauri::command]
fn stop_listening(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> String {
    state.stop_signal.store(true, Ordering::Relaxed);
    if let Some(handle) = state.thread_handle.lock().unwrap().take() {
        let thought_buffer = state.thought_buffer.clone();
        let model = state.llm_model.lock().unwrap().clone();
        let _ = app.emit("final-note-processing-started", ());
        std::thread::spawn(move || {
            let _ = handle.join();

            if let Some(block) = thought_buffer.flush() {
                save_buffered_live_note(&model, block, &app);
            } else {
                let _ = app.emit("final-note-processing-finished", ());
            }
        });
    }

    "Listening stopped".into()
}

#[tauri::command]
fn get_tasks(_state: tauri::State<AppState>) -> Vec<Task> {
    match open_db().and_then(|conn| { init_schema(&conn).map(|_| conn) }) {
        Ok(conn) => db::get_tasks(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn mark_task_done(_state: tauri::State<AppState>, id: String) -> bool {
    match open_db() {
        Ok(conn) => db::mark_task_done(&conn, &id).is_ok(),
        Err(_) => false,
    }
}

#[tauri::command]
fn create_task(_state: tauri::State<AppState>, title: String, project: Option<String>, due_hint: Option<String>) -> bool {
    let task = Task {
        id: Uuid::new_v4().to_string(),
        title,
        project,
        due_hint,
        done: false,
        created_at: Utc::now().to_rfc3339(),
        source: "manual".to_string(),
        status: "official".to_string(),
    };
    match open_db() {
        Ok(conn) => db::insert_task(&conn, &task).is_ok(),
        Err(_) => false,
    }
}

#[tauri::command]
fn get_goals(_state: tauri::State<AppState>) -> Vec<Goal> {
    match open_db().and_then(|conn| { init_schema(&conn).map(|_| conn) }) {
        Ok(conn) => db::get_goals(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn create_goal(_state: tauri::State<AppState>, title: String, description: Option<String>) -> bool {
    let conn = match open_db() {
        Ok(conn) => conn,
        Err(_) => return false,
    };
    let existing_titles: Vec<String> = db::get_goals(&conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|g| g.status == "active")
        .map(|g| g.title)
        .collect();
    if is_duplicate_goal_title(&title, &existing_titles) {
        return true;
    }

    let goal = Goal {
        id: Uuid::new_v4().to_string(),
        title,
        description,
        created_at: Utc::now().to_rfc3339(),
        status: "active".to_string(),
        horizon: "milestone".to_string(),
    };
    db::insert_goal(&conn, &goal).is_ok()
}

#[tauri::command]
fn get_meetings(_state: tauri::State<AppState>) -> Vec<Meeting> {
    match open_db().and_then(|conn| { init_schema(&conn).map(|_| conn) }) {
        Ok(conn) => db::get_meetings(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn get_active_meeting(state: tauri::State<AppState>) -> Option<String> {
    state.active_meeting_id.lock().unwrap().clone()
}

#[tauri::command]
fn start_meeting(state: tauri::State<AppState>, title: String, person: Option<String>) -> String {
    let mid = Uuid::new_v4().to_string();
    let meeting = Meeting {
        id: mid.clone(),
        title,
        person,
        started_at: Utc::now().to_rfc3339(),
        ended_at: None,
        summary: None,
        action_items: None,
        transcript: None,
    };
    if let Ok(conn) = open_db() {
        let _ = db::upsert_meeting(&conn, &meeting);
    }
    *state.active_meeting_id.lock().unwrap() = Some(mid.clone());
    mid
}

#[tauri::command]
fn end_meeting(state: tauri::State<'_, AppState>) -> String {
    let mid = match state.active_meeting_id.lock().unwrap().take() {
        Some(id) => id,
        None => return "No active meeting".to_string(),
    };
    let transcript = state.session_transcript.lock().unwrap().clone();
    let model = state.llm_model.lock().unwrap().clone();
    let mid_clone = mid.clone();
    std::thread::spawn(move || {
        if let Ok(conn) = open_db() {
            if let Ok(meetings) = db::get_meetings(&conn) {
                if let Some(m) = meetings.iter().find(|m| m.id == mid_clone) {
                    let person = m.person.as_deref().unwrap_or("Unknown").to_string();
                    let topic = m.title.clone();
                    let json = llm::meeting_summary(&model, &transcript, &person, &topic);
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json) {
                        let summary = parsed["summary"].as_str().unwrap_or("").to_string();
                        let action_items = serde_json::to_string(&parsed["action_items"]).unwrap_or_default();
                        let updated = Meeting {
                            id: mid_clone.clone(),
                            title: m.title.clone(),
                            person: m.person.clone(),
                            started_at: m.started_at.clone(),
                            ended_at: Some(Utc::now().to_rfc3339()),
                            summary: Some(summary),
                            action_items: Some(action_items),
                            transcript: Some(transcript.clone()),
                        };
                        let _ = db::upsert_meeting(&conn, &updated);
                    }
                }
            }
        }
    });
    mid
}

#[tauri::command]
async fn summarize_last_n_minutes(state: tauri::State<'_, AppState>, minutes: i64) -> Result<String, String> {
    // Ensure Ollama is running before making LLM calls
    tokio::task::spawn_blocking(llm::ensure_ollama_running)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e)?;
    let model = state.llm_model.lock().unwrap().clone();
    let chunks = open_db()
        .and_then(|conn| db::get_recent_chunks(&conn, minutes))
        .unwrap_or_default();
    let combined: String = chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" ");
    let label = format!("the last {} minutes", minutes);
    tokio::task::spawn_blocking(move || llm::summarize_transcript(&model, &combined, &label))
        .await
        .map_err(|e| format!("Could not generate summary: {}", e))
}

#[tauri::command]
async fn get_daily_suggestions(state: tauri::State<'_, AppState>) -> Result<String, String> {
    // Ensure Ollama is running before making LLM calls
    tokio::task::spawn_blocking(llm::ensure_ollama_running)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e)?;
    let model = state.llm_model.lock().unwrap().clone();
    let tasks = open_db().and_then(|conn| db::get_tasks(&conn)).unwrap_or_default();
    let goals = open_db().and_then(|conn| db::get_goals(&conn)).unwrap_or_default();
    let fog_stats = open_db().and_then(|conn| db::get_fog_stats(&conn)).unwrap_or_default();
    let tasks_json = serde_json::to_string(&tasks).unwrap_or_default();
    let goals_json = serde_json::to_string(&goals).unwrap_or_default();
    let fog_json = serde_json::to_string(&fog_stats).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        llm::daily_suggestions(&model, &tasks_json, &goals_json, &fog_json)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_fog_stats_cmd(_state: tauri::State<AppState>) -> Vec<(String, i64)> {
    open_db().and_then(|conn| db::get_fog_stats(&conn)).unwrap_or_default()
}

/// Get detailed fog entries (with strength, markers, explanation) from the last N hours.
/// Used by FogView to show why something is marked as foggy.
#[tauri::command]
fn get_fog_details(hours: i64) -> Vec<db::FogEntryExtended> {
    open_db()
        .and_then(|conn| db::get_fog_entries_extended(&conn, hours))
        .unwrap_or_default()
}

#[tauri::command]
async fn check_ollama_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let mut health = tokio::task::spawn_blocking(llm::check_ollama_health)
        .await
        .map_err(|e| e.to_string())?;

    let mut autostart_attempted = false;
    let mut autostart_ok = false;
    let mut autostart_error: Option<String> = None;

    if !health.available {
        let now = Utc::now().timestamp();
        let should_attempt = {
            let mut last_attempt = state.last_ollama_autostart_ts.lock().unwrap();
            if now - *last_attempt >= 30 {
                *last_attempt = now;
                true
            } else {
                false
            }
        };

        if should_attempt {
            autostart_attempted = true;
            let start_res = tokio::task::spawn_blocking(llm::ensure_ollama_running)
                .await
                .map_err(|e| e.to_string())?;

            match start_res {
                Ok(()) => {
                    autostart_ok = true;
                    health = tokio::task::spawn_blocking(llm::check_ollama_health)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Err(e) => {
                    autostart_error = Some(e);
                }
            }
        }
    }

    state.ollama_available.store(health.available, Ordering::Relaxed);

    Ok(serde_json::json!({
        "available": health.available,
        "responding": health.responding,
        "models": health.models,
        "running_models": health.running_models,
        "latency_ms": health.latency_ms,
        "last_error": health.last_error,
        "autostart_attempted": autostart_attempted,
        "autostart_ok": autostart_ok,
        "autostart_error": autostart_error,
    }))
}

#[tauri::command]
fn set_llm_model(state: tauri::State<AppState>, model: String) -> bool {
    let model = model.trim().to_string();
    if model.is_empty() {
        return false;
    }

    *state.llm_model.lock().unwrap() = model.clone();
    let saved = match open_db() {
        Ok(conn) => db::set_setting(&conn, "llm_model", &model).is_ok(),
        Err(_) => false,
    };
    if !saved {
        return false;
    }

    // Hot-load the model so the first voice command responds instantly.
    let model_loaded = Arc::clone(&state.model_loaded);
    let ollama_available = Arc::clone(&state.ollama_available);
    std::thread::spawn(move || {
        model_loaded.store(false, Ordering::Relaxed);
        let _ = llm::stop_all_running_models();
        match llm::preload_model(&model) {
            Ok(_) => {
                model_loaded.store(true, Ordering::Relaxed);
                ollama_available.store(true, Ordering::Relaxed);
            }
            Err(_) => {
                model_loaded.store(false, Ordering::Relaxed);
                ollama_available.store(false, Ordering::Relaxed);
            }
        }
    });
    saved
}

#[tauri::command]
fn get_llm_model(state: tauri::State<AppState>) -> String {
    state.llm_model.lock().unwrap().clone()
}

#[tauri::command]
fn is_model_loaded(state: tauri::State<AppState>) -> bool {
    state.model_loaded.load(Ordering::Relaxed)
}

#[tauri::command]
fn unload_model_cmd(state: tauri::State<AppState>) -> bool {
    let model = state.llm_model.lock().unwrap().clone();
    let model_loaded = Arc::clone(&state.model_loaded);
    std::thread::spawn(move || {
        let _ = llm::unload_model(&model);
        model_loaded.store(false, Ordering::Relaxed);
    });
    true
}

/// Load (warm) the currently saved model without changing which model is active.
/// Starts `ollama serve` if needed, then loads the model into Ollama memory.
#[tauri::command]
fn load_model_cmd(state: tauri::State<AppState>) -> bool {
    let model = state.llm_model.lock().unwrap().clone();
    if model.is_empty() {
        return false;
    }
    let model_loaded = Arc::clone(&state.model_loaded);
    let ollama_available = Arc::clone(&state.ollama_available);
    std::thread::spawn(move || {
        model_loaded.store(false, Ordering::Relaxed);
        let _ = llm::stop_all_running_models();
        match llm::preload_model(&model) {
            Ok(_) => {
                model_loaded.store(true, Ordering::Relaxed);
                ollama_available.store(true, Ordering::Relaxed);
            }
            Err(_) => {
                model_loaded.store(false, Ordering::Relaxed);
                ollama_available.store(false, Ordering::Relaxed);
            }
        }
    });
    true
}

// ── Tray status update ────────────────────────────────────────────────────────

#[tauri::command]
fn update_tray_status(app: tauri::AppHandle, status: String) {
    if let Some(tray) = app.tray_by_id("main") {
        let tooltip = format!("Nodemind — {}", status);
        let _ = tray.set_tooltip(Some(&tooltip));
        // On macOS menu bar, set_title shows text next to the icon
        let title = match status.as_str() {
            "Listening" => "●",
            "Idle" => "◑",
            _ => "",
        };
        let _ = tray.set_title(Some(title));
    }
}

// ── Focus sessions ────────────────────────────────────────────────────────────

fn hhmm_to_minutes(value: &str) -> Option<i32> {
    let mut parts = value.split(':');
    let h = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<i32>().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(h * 60 + m)
}

fn ranges_overlap(start_a: i32, end_a: i32, start_b: i32, end_b: i32) -> bool {
    start_a < end_b && end_a > start_b
}

fn has_focus_conflict(conn: &rusqlite::Connection, date: &str, start_min: i32, end_min: i32) -> bool {
    let sessions = db::get_focus_sessions_in_range(conn, date, date).unwrap_or_default();
    sessions.into_iter().any(|session| {
        let Some(existing_start) = hhmm_to_minutes(&session.start_time) else { return false; };
        let Some(existing_end) = hhmm_to_minutes(&session.end_time) else { return false; };
        ranges_overlap(start_min, end_min, existing_start, existing_end)
    })
}

fn has_meeting_conflict(conn: &rusqlite::Connection, date: &str, start_min: i32, end_min: i32) -> bool {
    let meetings = db::get_meetings(conn).unwrap_or_default();
    meetings.into_iter().any(|meeting| {
        let Ok(started_at) = DateTime::parse_from_rfc3339(&meeting.started_at) else {
            return false;
        };
        let local_start = started_at.with_timezone(&Local);
        if local_start.format("%Y-%m-%d").to_string() != date {
            return false;
        }

        let local_end = meeting
            .ended_at
            .as_deref()
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|dt| dt.with_timezone(&Local))
            .unwrap_or_else(|| local_start + Duration::minutes(60));

        let meeting_start = (local_start.hour() as i32) * 60 + (local_start.minute() as i32);
        let meeting_end = (local_end.hour() as i32) * 60 + (local_end.minute() as i32);

        ranges_overlap(start_min, end_min, meeting_start, meeting_end)
    })
}

#[tauri::command]
fn get_focus_sessions(start_date: String, end_date: String) -> Vec<db::FocusSession> {
    match open_db() {
        Ok(conn) => db::get_focus_sessions_in_range(&conn, &start_date, &end_date).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn create_focus_session(title: String, date: String, start_time: String, end_time: String, notes: Option<String>) -> bool {
    let title = title.trim().to_string();
    if title.is_empty() {
        return false;
    }

    let Some(start_min) = hhmm_to_minutes(&start_time) else {
        return false;
    };
    let Some(end_min) = hhmm_to_minutes(&end_time) else {
        return false;
    };
    if end_min <= start_min {
        return false;
    }

    let conn = match open_db() {
        Ok(conn) => conn,
        Err(_) => return false,
    };

    if has_focus_conflict(&conn, &date, start_min, end_min) || has_meeting_conflict(&conn, &date, start_min, end_min) {
        return false;
    }

    let session = db::FocusSession {
        id: Uuid::new_v4().to_string(),
        title,
        date,
        start_time,
        end_time,
        notes: notes.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        created_at: Utc::now().to_rfc3339(),
    };
    db::insert_focus_session(&conn, &session).is_ok()
}

#[tauri::command]
fn delete_focus_session(id: String) -> bool {
    match open_db() {
        Ok(conn) => db::delete_focus_session(&conn, &id).is_ok(),
        Err(_) => false,
    }
}

// ── Suggestion persistence ────────────────────────────────────────────────────

/// Persist a batch of LLM-generated suggestions. Dismisses any existing
/// pending suggestions first so only the latest batch is shown as pending.
#[tauri::command]
fn save_suggestions(_state: tauri::State<AppState>, suggestions_json: String) -> bool {
    let values: Vec<serde_json::Value> = match serde_json::from_str(&suggestions_json) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Ok(conn) = open_db() else { return false };
    // Replace old pending batch with the new one
    let _ = db::dismiss_pending_suggestions(&conn);
    for s in &values {
        let suggestion = db::StoredSuggestion {
            id: Uuid::new_v4().to_string(),
            suggestion_type: s["type"].as_str().unwrap_or("task").to_string(),
            text: s["text"].as_str().unwrap_or("").to_string(),
            priority: s["priority"].as_str().unwrap_or("medium").to_string(),
            status: "pending".to_string(),
            created_at: Utc::now().to_rfc3339(),
        };
        let _ = db::insert_suggestion(&conn, &suggestion);
    }
    true
}

#[tauri::command]
fn get_saved_suggestions() -> Vec<db::StoredSuggestion> {
    match open_db() {
        Ok(conn) => db::get_suggestions(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn update_suggestion_status(id: String, status: String) -> bool {
    match open_db() {
        Ok(conn) => db::update_suggestion_status(&conn, &id, &status).is_ok(),
        Err(_) => false,
    }
}

// ── Mental Map Graph Data ─────────────────────────────────────────────────

#[tauri::command]
fn get_graph_nodes() -> Vec<db::GraphNode> {
    match open_db() {
        Ok(conn) => db::get_nodes(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

#[tauri::command]
fn get_graph_edges() -> Vec<db::GraphEdge> {
    match open_db() {
        Ok(conn) => db::get_edges(&conn).unwrap_or_default(),
        Err(_) => vec![],
    }
}

// ── Background Decay Jobs ─────────────────────────────────────────────────

/// True when the normalized new text overlaps heavily with any existing suggestion text.
fn is_duplicate_suggestion(text: &str, existing: &[String]) -> bool {
    let norm: String = text.to_lowercase();
    let norm = norm.trim();
    for ex in existing {
        let ex_norm: String = ex.to_lowercase();
        let ex_norm = ex_norm.trim();
        if ex_norm == norm {
            return true;
        }
        // Token-overlap >= 75% of the shorter item → near-duplicate
        let text_tokens: BTreeSet<String> = tokenize_for_rag(norm).into_iter().collect();
        let ex_tokens: BTreeSet<String> = tokenize_for_rag(ex_norm).into_iter().collect();
        if text_tokens.is_empty() || ex_tokens.is_empty() {
            continue;
        }
        let overlap = text_tokens.intersection(&ex_tokens).count();
        let shorter = text_tokens.len().min(ex_tokens.len());
        if overlap as f64 / shorter as f64 >= 0.75 {
            return true;
        }
    }
    false
}

/// Process unread live notes: extract entities/suggestions via LLM, dedup, save, mark processed.
/// Shared by the 4-hour background job and the manual refresh commands.
/// Returns (notes_processed, suggestions_added).
fn run_batch_analysis(model: &str) -> (usize, usize) {
    let conn = match open_db() {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };

    let notes = match db::get_unprocessed_notes_for_today(&conn) {
        Ok(n) => n,
        Err(_) => return (0, 0),
    };

    if notes.is_empty() {
        return (0, 0);
    }

    let note_ids: Vec<String> = notes.iter().map(|n| n.id.clone()).collect();

    // Aggregate content from unprocessed notes into one block for the LLM.
    let combined: String = notes
        .iter()
        .flat_map(|n| {
            let mut parts = vec![format!("[summary] {}", n.summary)];
            parts.extend(n.highlights.iter().map(|h| format!("[highlight] {}", h)));
            parts.extend(n.ideas.iter().map(|id| format!("[idea] {}", id)));
            parts.extend(n.tasks.iter().map(|t| format!("[task] {}", t)));
            parts
        })
        .collect::<Vec<_>>()
        .join("\n");

    // ── Graph update: re-extract entities from aggregated notes ──────────────
    let recent_topics = db::get_last_active_topics(&conn)
        .unwrap_or_default()
        .join(", ");
    let extracted = llm::five_min_extract(model, &combined, &recent_topics);

    // Do not mark notes processed when extraction fails (e.g. Ollama offline).
    match serde_json::from_str::<serde_json::Value>(&extracted) {
        Ok(json) => {
            if json["error"]
                .as_str()
                .map(|e| !e.trim().is_empty())
                .unwrap_or(false)
            {
                eprintln!("[nodemind] batch analysis skipped: extraction error from LLM");
                return (0, 0);
            }
        }
        Err(e) => {
            eprintln!("[nodemind] batch analysis skipped: invalid extraction JSON: {}", e);
            return (0, 0);
        }
    }

    let mut candidate_tasks: Vec<String> = Vec::new();
    let mut person_labels: HashSet<String> = HashSet::new();
    let mut project_labels: HashSet<String> = HashSet::new();
    let mut goal_labels: HashSet<String> = HashSet::new();
    let mut task_labels: HashSet<String> = HashSet::new();

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&extracted) {
        let (resolved_nodes, stressor_nodes, spark_nodes) = parse_impact_sets(&json);
        let mut existing_goal_titles: Vec<String> = db::get_goals(&conn)
            .unwrap_or_default()
            .into_iter()
            .filter(|g| g.status == "active")
            .map(|g| g.title)
            .collect();

        // Goals
        if let Some(goals_arr) = json["goals"].as_array() {
            for g in goals_arr {
                if let Some(title) = g["title"].as_str() {
                    if title.len() > 3 {
                        if is_duplicate_goal_title(title, &existing_goal_titles) {
                            continue;
                        }
                        goal_labels.insert(normalize_entity_key(title));
                        let description = g["description"].as_str().map(|s| s.to_string());
                        let horizon = llm::classify_goal_horizon(model, title, description.as_deref());
                        let goal = db::Goal {
                            id: Uuid::new_v4().to_string(),
                            title: title.to_string(),
                            description,
                            created_at: Utc::now().to_rfc3339(),
                            status: "active".to_string(),
                            horizon,
                        };
                        let _ = db::insert_goal(&conn, &goal);
                        existing_goal_titles.push(title.to_string());
                        let goal_node_id = resolve_existing_node_id(&conn, "goal", title)
                            .unwrap_or_else(|| format!("goal-{}", slugify_label(title)));
                        let goal_node = db::GraphNode {
                            id: goal_node_id,
                            node_type: "goal".to_string(),
                            label: title.to_string(),
                            data: {
                                let key = normalize_entity_key(title);
                                let impact = if resolved_nodes.contains(&key) {
                                    Some("resolved")
                                } else if stressor_nodes.contains(&key) {
                                    Some("stressor")
                                } else if spark_nodes.contains(&key) {
                                    Some("spark")
                                } else {
                                    None
                                };
                                node_metadata_json(
                                    g["category"].as_str(),
                                    g["signal_type"].as_str().or(Some("intent")),
                                    None,
                                    impact,
                                    g["description"].as_str(),
                                )
                            },
                            created_at: Utc::now().to_rfc3339(),
                        };
                        let _ = db::insert_node(&conn, &goal_node);
                    }
                }
            }
        }
        // People
        if let Some(people_arr) = json["people"].as_array() {
            for p in people_arr {
                if let Some(name) = p["name"].as_str() {
                    if name.len() > 1 {
                        person_labels.insert(normalize_entity_key(name));
                        let node_id = resolve_existing_node_id(&conn, "person", name)
                            .unwrap_or_else(|| format!("person-{}", slugify_label(name)));
                        let node = db::GraphNode {
                            id: node_id,
                            node_type: "person".to_string(),
                            label: name.to_string(),
                            data: {
                                let key = normalize_entity_key(name);
                                let impact = if resolved_nodes.contains(&key) {
                                    Some("resolved")
                                } else if stressor_nodes.contains(&key) {
                                    Some("stressor")
                                } else if spark_nodes.contains(&key) {
                                    Some("spark")
                                } else {
                                    None
                                };
                                node_metadata_json(Some("Social"), None, None, impact, p["context"].as_str())
                            },
                            created_at: Utc::now().to_rfc3339(),
                        };
                        let _ = db::insert_node(&conn, &node);
                    }
                }
            }
        }
        // Projects
        if let Some(projects_arr) = json["projects"].as_array() {
            for proj in projects_arr {
                let (name, category) = if let Some(n) = proj.as_str() {
                    (n.to_string(), None)
                } else {
                    (
                        proj["name"].as_str().unwrap_or_default().to_string(),
                        proj["category"].as_str(),
                    )
                };
                if name.len() > 1 {
                    project_labels.insert(normalize_entity_key(&name));
                    let node_id = resolve_existing_node_id(&conn, "project", &name)
                        .unwrap_or_else(|| format!("project-{}", slugify_label(&name)));
                    let impact = {
                        let key = normalize_entity_key(&name);
                        if resolved_nodes.contains(&key) {
                            Some("resolved")
                        } else if stressor_nodes.contains(&key) {
                            Some("stressor")
                        } else if spark_nodes.contains(&key) {
                            Some("spark")
                        } else {
                            None
                        }
                    };
                    let node = db::GraphNode {
                        id: node_id,
                        node_type: "project".to_string(),
                        label: name,
                        data: node_metadata_json(category, None, None, impact, None),
                        created_at: Utc::now().to_rfc3339(),
                    };
                    let _ = db::insert_node(&conn, &node);
                }
            }
        }

        if let Some(topics_arr) = json["topics"].as_array() {
            for topic in topics_arr {
                let name = topic["name"].as_str().unwrap_or_default();
                if name.len() <= 1 {
                    continue;
                }
                let impact = {
                    let key = normalize_entity_key(name);
                    if resolved_nodes.contains(&key) {
                        Some("resolved")
                    } else if stressor_nodes.contains(&key) {
                        Some("stressor")
                    } else if spark_nodes.contains(&key) {
                        Some("spark")
                    } else {
                        None
                    }
                };
                if let Some(existing_id) = resolve_existing_node_id(&conn, "topic", name) {
                    let _ = db::increment_node_field(&conn, &existing_id, "mentions_count", 1);
                } else {
                    let node = db::GraphNode {
                        id: format!("topic-{}", slugify_label(name)),
                        node_type: "topic".to_string(),
                        label: name.to_string(),
                        data: node_metadata_json(topic["category"].as_str(), None, None, impact, None),
                        created_at: Utc::now().to_rfc3339(),
                    };
                    let _ = db::insert_node(&conn, &node);
                }
            }
        }

        if let Some(tasks_arr) = json["tasks"].as_array() {
            for t in tasks_arr {
                if let Some(title) = t["title"].as_str() {
                    task_labels.insert(normalize_entity_key(title));
                    let impact = {
                        let key = normalize_entity_key(title);
                        if resolved_nodes.contains(&key) {
                            Some("resolved")
                        } else if stressor_nodes.contains(&key) {
                            Some("stressor")
                        } else if spark_nodes.contains(&key) {
                            Some("spark")
                        } else {
                            None
                        }
                    };
                    if let Some(existing_id) = find_similar_atom(&conn, title, "task", 0.9) {
                        let _ = db::increment_node_field(&conn, &existing_id, "mentions_count", 1);
                    } else {
                        let node = db::GraphNode {
                            id: format!("task-{}", slugify_label(title)),
                            node_type: "task".to_string(),
                            label: title.to_string(),
                            data: node_metadata_json(
                                t["category"].as_str(),
                                t["signal_type"].as_str().or(Some("action")),
                                None,
                                impact,
                                t["project"].as_str(),
                            ),
                            created_at: Utc::now().to_rfc3339(),
                        };
                        let _ = db::insert_node(&conn, &node);
                    }
                    candidate_tasks.push(title.to_string());
                }
            }
        }
        // Connections → graph edges
        if let Some(connections) = json["connections"].as_array() {
            let mut edge_cache: HashMap<String, (String, String)> = HashMap::new();
            for conn_val in connections {
                if let (Some(from), Some(to), Some(rel)) = (
                    conn_val["from"].as_str(),
                    conn_val["to"].as_str(),
                    conn_val["relation"].as_str(),
                ) {
                    if from.trim().is_empty() || to.trim().is_empty() {
                        continue;
                    }
                    let from_key = normalize_entity_key(from);
                    let to_key = normalize_entity_key(to);
                    let from_resolved = if let Some(found) = edge_cache.get(&from_key) {
                        Some(found.clone())
                    } else {
                        let inferred = infer_node_type_from_label(from, &person_labels, &project_labels, &goal_labels, &task_labels);
                        let resolved = resolve_or_create_graph_node(&conn, from, &inferred);
                        if let Some(ref item) = resolved { edge_cache.insert(from_key, item.clone()); }
                        resolved
                    };
                    let to_resolved = if let Some(found) = edge_cache.get(&to_key) {
                        Some(found.clone())
                    } else {
                        let inferred = infer_node_type_from_label(to, &person_labels, &project_labels, &goal_labels, &task_labels);
                        let resolved = resolve_or_create_graph_node(&conn, to, &inferred);
                        if let Some(ref item) = resolved { edge_cache.insert(to_key, item.clone()); }
                        resolved
                    };
                    if let (Some((from_id, _)), Some((to_id, _))) = (from_resolved, to_resolved) {
                        let direction = conn_val["direction"].as_str().unwrap_or("from_to");
                        let (final_from, final_to) = if direction == "to_from" {
                            (to_id, from_id)
                        } else {
                            (from_id, to_id)
                        };
                        let edge = db::GraphEdge {
                            id: Uuid::new_v4().to_string(),
                            from_node: final_from,
                            to_node: final_to,
                            edge_type: normalize_relation(rel),
                            weight: 1.0,
                            last_updated: Utc::now().to_rfc3339(),
                        };
                        let _ = db::insert_edge(&conn, &edge);
                    }
                }
            }
        }

        if let (Some(goals_arr), Some(tasks_arr)) = (json["goals"].as_array(), json["tasks"].as_array()) {
            for g in goals_arr {
                let Some(goal_title) = g["title"].as_str() else { continue; };
                let Some(goal_id) = resolve_existing_node_id(&conn, "goal", goal_title) else { continue; };

                for t in tasks_arr {
                    let Some(task_title) = t["title"].as_str() else { continue; };
                    let Some(task_id) = resolve_existing_node_id(&conn, "task", task_title) else { continue; };
                    let project = t["project"].as_str().unwrap_or_default();

                    let overlap = overlap_score(goal_title, task_title)
                        .max(overlap_score(goal_title, project));
                    if overlap == 0 {
                        continue;
                    }

                    let edge = db::GraphEdge {
                        id: Uuid::new_v4().to_string(),
                        from_node: goal_id.clone(),
                        to_node: task_id,
                        edge_type: "triggers".to_string(),
                        weight: 1.0 + (overlap as f64 * 0.2),
                        last_updated: Utc::now().to_rfc3339(),
                    };
                    let _ = db::insert_edge(&conn, &edge);
                }
            }
        }

        // Collect task candidates from extraction
        if let Some(tasks_arr) = json["tasks"].as_array() {
            for t in tasks_arr {
                if let Some(title) = t["title"].as_str() {
                    candidate_tasks.push(title.to_string());
                }
            }
        }
        if let Some(cal_items) = json["calendar_items"].as_array() {
            for item in cal_items {
                if let Some(title) = item["title"].as_str() {
                    if title.len() > 3 {
                        candidate_tasks.push(title.to_string());
                    }
                }
            }
        }
    }

    // ── Suggestions: task candidates via RAG judge (LLM validates) ───────────
    let mut seen_texts = db::get_pending_suggestion_texts(&conn).unwrap_or_default();
    let mut added = 0usize;

    let normalized_candidates = normalize_task_candidates(&candidate_tasks);
    if !normalized_candidates.is_empty() {
        let rag_ctx = build_day_rag_context(&normalized_candidates);
        let approved_json = llm::task_rag_judge(
            model,
            &serde_json::to_string(&normalized_candidates).unwrap_or_else(|_| "[]".to_string()),
            &rag_ctx,
        );
        if let Ok(approved) = serde_json::from_str::<serde_json::Value>(&approved_json) {
            if let Some(items) = approved["approved_tasks"].as_array() {
                for item in items {
                    if let Some(text) = item["text"].as_str() {
                        let trimmed = text.trim().to_string();
                        if trimmed.is_empty() || is_duplicate_suggestion(&trimmed, &seen_texts) {
                            continue;
                        }
                        let s = db::StoredSuggestion {
                            id: Uuid::new_v4().to_string(),
                            suggestion_type: "task".to_string(),
                            text: trimmed.clone(),
                            priority: item["priority"].as_str().unwrap_or("medium").to_string(),
                            status: "pending".to_string(),
                            created_at: Utc::now().to_rfc3339(),
                        };
                        let _ = db::insert_suggestion(&conn, &s);
                        seen_texts.push(trimmed);
                        added += 1;
                    }
                }
            }
        }
    }

    // ── Suggestions: reflections/insights via daily_suggestions (LLM judges) ─
    let tasks = db::get_tasks(&conn).unwrap_or_default();
    let goals = db::get_goals(&conn).unwrap_or_default();
    let fog_stats = db::get_fog_stats(&conn).unwrap_or_default();
    let sugg_raw = llm::daily_suggestions(
        model,
        &serde_json::to_string(&tasks).unwrap_or_default(),
        &serde_json::to_string(&goals).unwrap_or_default(),
        &serde_json::to_string(&fog_stats).unwrap_or_default(),
    );
    if let Ok(sugg_json) = serde_json::from_str::<serde_json::Value>(&sugg_raw) {
        if let Some(suggs) = sugg_json["suggestions"].as_array() {
            for s in suggs {
                if let Some(text) = s["text"].as_str() {
                    let trimmed = text.trim().to_string();
                    if trimmed.is_empty() || is_duplicate_suggestion(&trimmed, &seen_texts) {
                        continue;
                    }
                    let sugg = db::StoredSuggestion {
                        id: Uuid::new_v4().to_string(),
                        suggestion_type: s["type"].as_str().unwrap_or("reflection").to_string(),
                        text: trimmed.clone(),
                        priority: s["priority"].as_str().unwrap_or("medium").to_string(),
                        status: "pending".to_string(),
                        created_at: Utc::now().to_rfc3339(),
                    };
                    let _ = db::insert_suggestion(&conn, &sugg);
                    seen_texts.push(trimmed);
                    added += 1;
                }
            }
        }
    }

    // ── Mark notes as processed so the batch job skips them next time ─────────
    let _ = db::mark_notes_processed(&conn, &note_ids);

    (note_ids.len(), added)
}

/// Start background maintenance jobs.
fn start_decay_jobs() {
    // ── Live-note archival (hourly) ───────────────────────────────────────────
    std::thread::spawn(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if let Ok(conn) = open_db() {
                    let archived = db::archive_previous_live_notes(&conn).unwrap_or(0);
                    if archived > 0 {
                        eprintln!("[nodemind] Archived {} day(s) of live notes", archived);
                    }
                }
            }
        })
    });

    // ── Edge decay (weekly, Sunday 02:00 UTC) ────────────────────────────────
    std::thread::spawn(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            loop {
                let now = chrono::Utc::now();
                let next_sunday_02 = next_sunday_at_02();
                let duration_until = (next_sunday_02 - now).to_std().unwrap_or_else(|_| std::time::Duration::from_secs(86400));
                tokio::time::sleep(duration_until).await;
                if let Ok(conn) = open_db() {
                    let _ = db::decay_edge_weights(&conn);
                    eprintln!("[nodemind] Edge decay job completed");
                }
                tokio::time::sleep(std::time::Duration::from_secs(7 * 24 * 3600)).await;
            }
        })
    });

    // ── Batch analysis every 4 hours (suggestions + graph + fog) ─────────────
    // Only runs when there are unprocessed notes, saving LLM calls.
    std::thread::spawn(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(4 * 3600));
            // Skip the first immediate tick so we don't run on startup.
            interval.tick().await;
            loop {
                interval.tick().await;
                let model = open_db()
                    .ok()
                    .and_then(|conn| db::get_setting(&conn, "llm_model"))
                    .unwrap_or_else(|| "qwen3.5:9b".to_string());
                let (notes, suggestions) = tokio::task::spawn_blocking(move || {
                    run_batch_analysis(&model)
                })
                .await
                .unwrap_or((0, 0));
                if notes > 0 {
                    eprintln!("[nodemind] 4h batch: processed {} notes, added {} suggestions", notes, suggestions);
                }
            }
        })
    });
}

/// Calculate next Sunday at 02:00 AM UTC.
fn next_sunday_at_02() -> chrono::DateTime<chrono::Utc> {
    use chrono::Weekday;
    let now = chrono::Utc::now();
    let today_weekday = now.weekday();
    
    let days_until_sunday = match today_weekday {
        Weekday::Sun => 0,
        Weekday::Mon => 6,
        Weekday::Tue => 5,
        Weekday::Wed => 4,
        Weekday::Thu => 3,
        Weekday::Fri => 2,
        Weekday::Sat => 1,
    };
    
    let target_date = now.date_naive().succ_opt().and_then(|d| {
        let d = d + chrono::Duration::days(days_until_sunday as i64 - 1);
        d.and_hms_opt(2, 0, 0)
    });
    
    if let Some(target) = target_date {
        target.and_utc()
    } else {
        now + chrono::Duration::days(7)
    }
}

/// Manually trigger the batch analysis on unread notes: updates suggestions (LLM-judged & deduped).
/// Accepts a note ID list (ignored; runs on all unprocessed notes today).
#[tauri::command]
async fn refresh_suggestions(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<String, String> {
    tokio::task::spawn_blocking(llm::ensure_ollama_running)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e)?;
    let model = state.llm_model.lock().unwrap().clone();
    let (notes, added) = tokio::task::spawn_blocking(move || run_batch_analysis(&model))
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("suggestions-updated", ());
    let _ = app.emit("mental-map-updated", ());
    Ok(format!("Processed {} notes, added {} new suggestions", notes, added))
}

/// Manually trigger the batch analysis on unread notes: rebuilds graph nodes/edges and suggestions.
#[tauri::command]
async fn refresh_mental_map(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<String, String> {
    tokio::task::spawn_blocking(llm::ensure_ollama_running)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e)?;
    let model = state.llm_model.lock().unwrap().clone();
    let (notes, _) = tokio::task::spawn_blocking(move || run_batch_analysis(&model))
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("mental-map-updated", ());
    let _ = app.emit("suggestions-updated", ());
    Ok(format!("Mental map updated from {} new notes", notes))
}

fn main() {
    if let Ok(conn) = open_db() {
        let _ = init_schema(&conn);
        let _ = db::archive_previous_live_notes(&conn);
    }

    // Start background maintenance jobs
    start_decay_jobs();

    let saved_model = open_db()
        .ok()
        .and_then(|conn| db::get_setting(&conn, "llm_model"))
        .unwrap_or_else(|| "qwen3.5:9b".to_string());

    tauri::Builder::default()
        .manage(AppState {
            transcript: Arc::new(Mutex::new(String::new())),
            language: Arc::new(Mutex::new("en".to_string())),
            listening: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(AtomicBool::new(false)),
            thread_handle: Arc::new(Mutex::new(None)),
            active_meeting_id: Arc::new(Mutex::new(None)),
            llm_model: Arc::new(Mutex::new(saved_model)),
            session_transcript: Arc::new(Mutex::new(String::new())),
            model_loaded: Arc::new(AtomicBool::new(false)),
            ollama_available: Arc::new(AtomicBool::new(false)),
            last_ollama_warning_ts: Arc::new(Mutex::new(0)),
            last_ollama_autostart_ts: Arc::new(Mutex::new(0)),
            thought_buffer: ThoughtBlockBuffer::default(),
        })
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
            use tauri::tray::TrayIconBuilder;

            let toggle   = MenuItem::with_id(app, "toggle",   "⏺  Toggle Listening",  true, None::<&str>)?;
            let summarize = MenuItem::with_id(app, "summarize", "📋  Summarize 30 min",  true, None::<&str>)?;
            let open_dash = MenuItem::with_id(app, "open",     "🏠  Open Dashboard",    true, None::<&str>)?;
            let sep       = PredefinedMenuItem::separator(app)?;
            let quit      = MenuItem::with_id(app, "quit",     "Quit Nodemind",              true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&toggle, &summarize, &open_dash, &sep, &quit])?;

            // Use the Nodemind icon for the macOS menu bar item
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../../public/nodemind_icon.png"))
                .unwrap_or_else(|_| app.default_window_icon().unwrap().clone());

            TrayIconBuilder::with_id("main")
                .tooltip("Nodemind — Off")
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "toggle" => {
                            let _ = app.emit("tray-toggle-listening", ());
                        }
                        "summarize" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                            let _ = app.emit("tray-summarize", ());
                        }
                        "open" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // On app launch, immediately process any pending live notes so
            // suggestions and map state catch up without waiting for the 4h job.
            let startup_model = app
                .state::<AppState>()
                .llm_model
                .lock()
                .map(|m| m.clone())
                .unwrap_or_else(|_| "qwen3.5:9b".to_string());
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = tokio::task::spawn_blocking(move || -> Result<(usize, usize), String> {
                    llm::ensure_ollama_running()?;
                    Ok(run_batch_analysis(&startup_model))
                })
                .await;

                match result {
                    Ok(Ok((notes, suggestions))) => {
                        if notes > 0 {
                            eprintln!(
                                "[nodemind] startup catch-up: processed {} notes, added {} suggestions",
                                notes, suggestions
                            );
                            let _ = app_handle.emit("suggestions-updated", ());
                            let _ = app_handle.emit("mental-map-updated", ());
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("[nodemind] startup catch-up skipped: {}", e);
                    }
                    Err(e) => {
                        eprintln!("[nodemind] startup catch-up task join error: {}", e);
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_listening,
            stop_listening,
            is_listening,
            set_language,
            get_latest_transcript,
            get_session_transcript,
            clear_transcript,
            get_today_transcript,
            get_today_transcript_chunks,
            get_today_live_notes,
            get_historical_notes,
            get_unprocessed_notes_count,
            get_tasks,
            mark_task_done,
            create_task,
            get_goals,
            create_goal,
            get_meetings,
            get_active_meeting,
            start_meeting,
            end_meeting,
            summarize_last_n_minutes,
            get_daily_suggestions,
            get_fog_stats_cmd,
            get_fog_details,
            check_ollama_status,
            set_llm_model,
            get_llm_model,
            is_model_loaded,
            unload_model_cmd,
            load_model_cmd,
            update_tray_status,
            get_focus_sessions,
            create_focus_session,
            delete_focus_session,
            save_suggestions,
            get_saved_suggestions,
            update_suggestion_status,
            get_graph_nodes,
            get_graph_edges,
            refresh_suggestions,
            refresh_mental_map,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
