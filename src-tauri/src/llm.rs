use serde::{Deserialize, Serialize};
use crate::prompts;
use std::time::Instant;

// Ollama default endpoint
const OLLAMA_URL: &str = "http://127.0.0.1:11434/api/generate";

#[derive(Debug, Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaHealth {
    pub available: bool,
    pub responding: bool,
    pub models: Vec<String>,
    pub running_models: Vec<String>,
    pub latency_ms: u128,
    pub last_error: Option<String>,
}

/// Send a prompt to local Ollama and return the response text.
/// Returns an error string if Ollama isn't running or model is missing.
pub fn ollama_complete(model: &str, prompt: &str) -> Result<String, String> {
    ollama_complete_inner(model, prompt, false)
}

/// Like `ollama_complete` but forces JSON output via Ollama's `format: "json"` +
/// disables inline thinking (`think: false`) so reasoning text never leaks into
/// the response. Use this for every prompt that must return a JSON object.
pub fn ollama_complete_json(model: &str, prompt: &str) -> Result<String, String> {
    ollama_complete_inner(model, prompt, true)
}

fn ollama_complete_inner(model: &str, prompt: &str, json_mode: bool) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let body = OllamaRequest {
        model,
        prompt,
        stream: false,
        format: if json_mode { Some("json") } else { None },
        // Suppress built-in "thinking" for models that support it (e.g. qwen3).
        // This prevents reasoning text from leaking into the response.
        think: if json_mode { Some(false) } else { None },
    };

    let res = client
        .post(OLLAMA_URL)
        .json(&body)
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                format!("Ollama request timed out at {}", OLLAMA_URL)
            } else if e.is_connect() {
                format!("Ollama connection failed at {}", OLLAMA_URL)
            } else {
                format!("Ollama request failed: {}", e)
            }
        })?;

    let text = res.text().map_err(|e| e.to_string())?;

    // Ollama non-streaming returns one JSON object
    let parsed: OllamaResponse = serde_json::from_str(&text)
        .map_err(|e| format!("Parse error: {} — raw: {}", e, &text[..text.len().min(200)]))?;

    let raw = parsed.response.trim().to_string();

    // Thinking models (e.g. qwen3) wrap reasoning in <think>...</think> before
    // the actual output. Strip those blocks so callers always get clean text/JSON.
    let cleaned = strip_think_tags(&raw);

    Ok(cleaned)
}

/// Remove all `<think>…</think>` blocks (including nested/multiline) from LLM output.
/// Handles case-insensitive tags (e.g. `<Think>`, `<THINK>`).
fn strip_think_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let lower = s.to_lowercase();
    let mut offset = 0usize;
    loop {
        match lower[offset..].find("<think>") {
            None => {
                result.push_str(&s[offset..]);
                break;
            }
            Some(rel_start) => {
                let abs_start = offset + rel_start;
                result.push_str(&s[offset..abs_start]);
                match lower[abs_start..].find("</think>") {
                    None => break, // malformed — drop the rest
                    Some(end_rel) => {
                        offset = abs_start + end_rel + "</think>".len();
                    }
                }
            }
        }
    }
    // Also strip <|think|>...<|/think|> style tags used by some models
    let cleaned = result.trim().to_string();
    strip_delimited(&cleaned, "<|think|>", "<|/think|>")
}

/// Generic helper to strip delimited blocks (case-insensitive).
fn strip_delimited(s: &str, open: &str, close: &str) -> String {
    let lower = s.to_lowercase();
    let open_l = open.to_lowercase();
    let close_l = close.to_lowercase();
    let mut result = String::with_capacity(s.len());
    let mut offset = 0usize;
    loop {
        match lower[offset..].find(&open_l) {
            None => { result.push_str(&s[offset..]); break; }
            Some(rel) => {
                result.push_str(&s[offset..offset + rel]);
                match lower[offset + rel..].find(&close_l) {
                    None => break,
                    Some(end_rel) => { offset = offset + rel + end_rel + close.len(); }
                }
            }
        }
    }
    result.trim().to_string()
}

/// Strip markdown code fences from LLM output.
fn strip_markdown_fences(s: &str) -> String {
    let s = s.trim();
    // Handle ```json ... ``` or ``` ... ```
    let s = if s.starts_with("```") {
        let inner = s.trim_start_matches("```json").trim_start_matches("```");
        let inner = if let Some(end_pos) = inner.rfind("```") {
            &inner[..end_pos]
        } else {
            inner
        };
        inner.trim()
    } else {
        s
    };
    s.to_string()
}

/// Extract the first complete JSON object `{…}` from an LLM response.
/// Validates that the extracted string is actually valid JSON.
/// Falls back to bracket-counting if the naive first/last approach fails.
fn extract_json_object(s: &str) -> String {
    let s = strip_markdown_fences(s);

    // Try 1: find first `{` and last `}`, check if it's valid JSON
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if end >= start {
                let candidate = &s[start..=end];
                if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                    return candidate.to_string();
                }
            }
        }
    }

    // Try 2: bracket-counting — find the first balanced `{…}` block
    if let Some(result) = find_balanced_json(&s, '{', '}') {
        if serde_json::from_str::<serde_json::Value>(&result).is_ok() {
            return result;
        }
    }

    // Try 3: maybe it's a JSON array at top level
    if let Some(result) = find_balanced_json(&s, '[', ']') {
        if serde_json::from_str::<serde_json::Value>(&result).is_ok() {
            return format!("{{\"items\":{}}}", result);
        }
    }

    // Give up — return the raw string and let the caller handle it
    s.to_string()
}

/// Walk through `s` starting from the first occurrence of `open` and count
/// matching open/close characters to extract a balanced block.
fn find_balanced_json(s: &str, open: char, close: char) -> Option<String> {
    let start = s.find(open)?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    for (i, ch) in s[start..].char_indices() {
        if escape_next { escape_next = false; continue; }
        if ch == '\\' && in_string { escape_next = true; continue; }
        if ch == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        if ch == open { depth += 1; }
        if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(s[start..=start + i].to_string());
            }
        }
    }
    None
}

// ── Prompt builders ────────────────────────────────────────────────────────────

/// Classify a transcript chunk as Noise, Observation, Command, or CoreThought.
pub fn classify_signal(model: &str, text: &str) -> String {
    let prompt = prompts::get().signal_classifier.template.replace("{text}", text);
    match ollama_complete(model, &prompt) {
        Ok(r) => r.trim().to_string(),
        Err(_) => "CoreThought".to_string(),
    }
}

/// Parse a direct command transcript and return a JSON intent object.
pub fn parse_command(model: &str, command: &str, now: &str) -> String {
    let prompt = prompts::get().parse_command.template
        .replace("{now}", now)
        .replace("{command}", command);
    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(_) => extracted,
                Err(e) => {
                    eprintln!("[nodemind] parse_command: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(300)]);
                    format!("{{\"intent\":\"Unknown\",\"raw\":\"{}\"}}", command.replace('"', "\\\"" ))
                }
            }
        }
        Err(e) => format!("{{\"intent\":\"Unknown\",\"error\":\"{}\",\"raw\":\"{}\"}}", e, command.replace('"', "\\\"")),
    }
}

/// Summarize a block of transcript text.
pub fn summarize_transcript(model: &str, transcript: &str, window_label: &str) -> String {
    if transcript.trim().is_empty() {
        return "No transcript available for this period.".to_string();
    }
    let prompt = prompts::get().summarize_transcript.template
        .replace("{window_label}", window_label)
        .replace("{transcript}", transcript);
    match ollama_complete(model, &prompt) {
        Ok(r) => r,
        Err(e) => format!("Could not generate summary: {}", e),
    }
}

/// Generate meeting summary and MOM (Minutes of Meeting).
pub fn meeting_summary(model: &str, transcript: &str, person: &str, topic: &str) -> String {
    let default = r#"{"summary":"Could not generate summary.","decisions":[],"action_items":[],"open_questions":[]}"#;
    if transcript.trim().is_empty() {
        return r#"{"summary":"No transcript.","decisions":[],"action_items":[],"open_questions":[]}"#.to_string();
    }
    let prompt = prompts::get().meeting_summary.template
        .replace("{person}", person)
        .replace("{topic}", topic)
        .replace("{transcript}", transcript);
    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(_) => extracted,
                Err(e) => {
                    eprintln!("[nodemind] meeting_summary: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(500)]);
                    default.to_string()
                }
            }
        }
        Err(e) => {
            eprintln!("[nodemind] meeting_summary: LLM error: {}", e);
            format!("{{\"summary\":\"Error: {}\",\"decisions\":[],\"action_items\":[],\"open_questions\":[]}}", e)
        }
    }
}

/// Generate daily suggestions based on tasks, goals, and fog patterns.
pub fn daily_suggestions(model: &str, tasks_json: &str, goals_json: &str, fog_json: &str) -> String {
    let prompt = prompts::get().daily_suggestions.template
        .replace("{tasks_json}", tasks_json)
        .replace("{goals_json}", goals_json)
        .replace("{fog_json}", fog_json);
    let default = r#"{"suggestions":[],"fog_insights":[]}"#;
    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            // Validate it's actually parseable JSON with the expected shape
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(val) => {
                    // Ensure it has the "suggestions" key
                    if val.get("suggestions").is_some() {
                        extracted
                    } else if let Some(arr) = val.as_array() {
                        // Model returned a bare array — wrap it
                        format!(r#"{{"suggestions":{},"fog_insights":[]}}"#, serde_json::to_string(arr).unwrap_or_default())
                    } else {
                        eprintln!("[nodemind] daily_suggestions: unexpected JSON shape: {}", &extracted[..extracted.len().min(300)]);
                        default.to_string()
                    }
                }
                Err(e) => {
                    eprintln!("[nodemind] daily_suggestions: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(500)]);
                    default.to_string()
                }
            }
        }
        Err(e) => {
            eprintln!("[nodemind] daily_suggestions: LLM error: {}", e);
            format!("{{\"suggestions\":[],\"fog_insights\":[],\"error\":\"{}\"}}", e)
        }
    }
}

/// Try to start `ollama serve` if Ollama is not already running.
/// Spawns the process detached so it outlives the calling thread.
/// Waits up to ~5 s for the server to become reachable.
pub fn ensure_ollama_running() -> Result<(), String> {
    // Fast-path: already up
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    if client.get("http://127.0.0.1:11434/api/tags").send().is_ok() {
        return Ok(());
    }

    // Launch `ollama serve` as a detached background process
    std::process::Command::new("ollama")
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start Ollama: {} — make sure 'ollama' is installed and on PATH", e))?;

    // Wait up to 5 s for it to be reachable
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if client.get("http://127.0.0.1:11434/api/tags").send().is_ok() {
            return Ok(());
        }
    }
    Err("Ollama did not start in time — please run 'ollama serve' manually".to_string())
}

/// Detailed health for Ollama API status panel/circuit-breaker usage.
pub fn check_ollama_health() -> OllamaHealth {
    let mut health = OllamaHealth {
        available: false,
        responding: false,
        models: vec![],
        running_models: vec![],
        latency_ms: 0,
        last_error: None,
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build() {
        Ok(c) => c,
        Err(e) => {
            health.last_error = Some(e.to_string());
            return health;
        }
    };

    let started = Instant::now();
    let tags_res = client.get("http://127.0.0.1:11434/api/tags").send();
    health.latency_ms = started.elapsed().as_millis();

    let tags_body = match tags_res {
        Ok(res) => match res.text() {
            Ok(text) => text,
            Err(e) => {
                health.last_error = Some(format!("failed reading /api/tags body: {}", e));
                return health;
            }
        },
        Err(e) => {
            health.last_error = Some(format!("/api/tags unreachable: {}", e));
            return health;
        }
    };

    health.available = true;
    health.responding = true;

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&tags_body) {
        if let Some(models) = json["models"].as_array() {
            health.models = models.iter()
                .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                .collect();
        }
    }

    // Optional runtime status endpoint (Ollama /api/ps)
    if let Ok(res) = client.get("http://127.0.0.1:11434/api/ps").send() {
        if let Ok(text) = res.text() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(models) = json["models"].as_array() {
                    health.running_models = models.iter()
                        .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                        .collect();
                }
            }
        }
    }

    health
}

/// Stop all currently running Ollama models to free RAM before loading a new one.
pub fn stop_all_running_models() -> Result<(), String> {
    let h = check_ollama_health();
    if !h.available {
        return Ok(());
    }

    let mut seen = std::collections::BTreeSet::new();
    for m in h.running_models {
        if m.trim().is_empty() || !seen.insert(m.clone()) {
            continue;
        }
        let _ = unload_model(&m);
    }
    Ok(())
}

// ── Model lifecycle ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OllamaKeepAliveRequest<'a> {
    model: &'a str,
    keep_alive: i64, // -1 = indefinite, 0 = unload immediately
}

/// Pre-load a model into Ollama memory so it's warm for fast responses.
/// Starts `ollama serve` first if not already running, then sends keep_alive=-1
/// (programmatic equivalent of `ollama run <model>` without the interactive REPL).
pub fn preload_model(model: &str) -> Result<(), String> {
    // Ensure the Ollama server is up before trying to load
    ensure_ollama_running()?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let body = OllamaKeepAliveRequest { model, keep_alive: -1 };
    client
        .post(OLLAMA_URL)
        .json(&body)
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                format!("Ollama preload timed out at {}", OLLAMA_URL)
            } else if e.is_connect() {
                format!("Ollama preload connection failed at {}", OLLAMA_URL)
            } else {
                format!("Ollama preload failed: {}", e)
            }
        })?;
    Ok(())
}

/// Unload a model from Ollama memory using `ollama stop <model>`.
/// Falls back to the keep_alive=0 HTTP approach if the CLI is unavailable.
pub fn unload_model(model: &str) -> Result<(), String> {
    // Try `ollama stop <model>` first (Ollama v0.3+)
    let cli_result = std::process::Command::new("ollama")
        .arg("stop")
        .arg(model)
        .output();

    match cli_result {
        Ok(out) if out.status.success() => return Ok(()),
        // CLI failed or not available — fall back to HTTP keep_alive=0
        _ => {}
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let body = OllamaKeepAliveRequest { model, keep_alive: 0 };
    client
        .post(OLLAMA_URL)
        .json(&body)
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                format!("Ollama unload timed out at {}", OLLAMA_URL)
            } else if e.is_connect() {
                format!("Ollama unload connection failed at {}", OLLAMA_URL)
            } else {
                format!("Ollama unload failed: {}", e)
            }
        })?;
    Ok(())
}

/// Lightweight fog signal extraction from a single raw transcript chunk.
/// Only detects linguistic friction — no task or topic extraction.
/// Run in real-time on every chunk; keeps per-chunk processing fast.
pub fn fog_signals_only(model: &str, text: &str) -> String {
    let default = r#"{"signals":[]}"#;
    let prompt = prompts::get().fog_signals_only.template
        .replace("{text}", text);
    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(val) => {
                    // Normalize legacy output shape {"fog_signals":["..."]}
                    // into diagnosis objects expected by the pipeline.
                    if let Some(items) = val.get("signals").and_then(|v| v.as_array()) {
                        let normalized = serde_json::json!({ "signals": items });
                        normalized.to_string()
                    } else if let Some(tags) = val.get("fog_signals").and_then(|v| v.as_array()) {
                        let signals: Vec<serde_json::Value> = tags
                            .iter()
                            .filter_map(|t| t.as_str())
                            .map(|tag| {
                                serde_json::json!({
                                    "tag": tag,
                                    "context": text.chars().take(180).collect::<String>(),
                                    "intensity": 0.45,
                                    "clarity_question": "What is the smallest concrete next step right now?"
                                })
                            })
                            .collect();
                        serde_json::json!({ "signals": signals }).to_string()
                    } else {
                        default.to_string()
                    }
                }
                Err(e) => {
                    eprintln!("[nodemind] fog_signals_only: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(300)]);
                    default.to_string()
                }
            }
        }
        Err(e) => {
            eprintln!("[nodemind] fog_signals_only: LLM error: {}", e);
            default.to_string()
        }
    }
}

/// Deep extraction from a 5-minute transcript buffer.
/// Returns tasks, goals, people, calendar items, projects, mental map connections,
/// and tray suggestions. Called once every 5 minutes — never on raw chunks.
pub fn five_min_extract(model: &str, transcript: &str, recent_topics: &str) -> String {
    let default = r#"{"live_note":{"summary":"","highlights":[],"ideas":[],"tasks":[]},"tasks":[],"goals":[],"people":[],"calendar_items":[],"projects":[],"topics":[],"connections":[],"high_impact":{"resolved_nodes":[],"stressor_nodes":[],"spark_nodes":[]},"tray_suggestions":[]}"#;
    if transcript.trim().is_empty() {
        return default.to_string();
    }
    let prompt = prompts::get().five_min_extract.template
        .replace("{transcript}", transcript)
        .replace("{recent_topics}", recent_topics);
    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(_) => extracted,
                Err(e) => {
                    eprintln!("[nodemind] five_min_extract: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(500)]);
                    default.to_string()
                }
            }
        }
        Err(e) => {
            eprintln!("[nodemind] five_min_extract: LLM error: {}", e);
            format!(r#"{{"live_note":{{"summary":"","highlights":[],"ideas":[],"tasks":[]}},"tasks":[],"goals":[],"people":[],"calendar_items":[],"projects":[],"topics":[],"connections":[],"high_impact":{{"resolved_nodes":[],"stressor_nodes":[],"spark_nodes":[]}},"tray_suggestions":[],"error":"{}"}}"#, e)
        }
    }
}

/// Judge candidate task titles against retrieved day context (RAG) and return
/// only approved task suggestions.
pub fn task_rag_judge(model: &str, candidate_tasks_json: &str, rag_context: &str) -> String {
    let default = r#"{"approved_tasks":[]}"#;
    if candidate_tasks_json.trim().is_empty() || candidate_tasks_json.trim() == "[]" {
        return default.to_string();
    }

    let prompt = prompts::get().task_rag_judge.template
        .replace("{candidate_tasks_json}", candidate_tasks_json)
        .replace("{rag_context}", rag_context);

    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(val) => {
                    if val.get("approved_tasks").is_some() {
                        extracted
                    } else {
                        default.to_string()
                    }
                }
                Err(e) => {
                    eprintln!("[nodemind] task_rag_judge: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(500)]);
                    default.to_string()
                }
            }
        }
        Err(e) => {
            eprintln!("[nodemind] task_rag_judge: LLM error: {}", e);
            format!(r#"{{"approved_tasks":[],"error":"{}"}}"#, e)
        }
    }
}

/// Classify a goal into spark, milestone, or north_star horizon.
/// Falls back to milestone on any parse/runtime failure.
pub fn classify_goal_horizon(model: &str, title: &str, description: Option<&str>) -> String {
    let default = "milestone".to_string();
    let prompt = prompts::get().goal_horizon_classifier.template
        .replace("{goal_title}", title)
        .replace("{goal_description}", description.unwrap_or(""));

    match ollama_complete_json(model, &prompt) {
        Ok(r) => {
            let extracted = extract_json_object(&r);
            match serde_json::from_str::<serde_json::Value>(&extracted) {
                Ok(val) => {
                    let horizon = val["horizon"].as_str().unwrap_or("milestone");
                    match horizon {
                        "spark" | "milestone" | "north_star" => horizon.to_string(),
                        _ => default,
                    }
                }
                Err(e) => {
                    eprintln!("[nodemind] classify_goal_horizon: JSON parse failed: {} — raw: {}", e, &r[..r.len().min(300)]);
                    default
                }
            }
        }
        Err(e) => {
            eprintln!("[nodemind] classify_goal_horizon: LLM error: {}", e);
            default
        }
    }
}
