//! Clients for the two llama-server instances (OpenAI-compatible HTTP APIs).
//!
//!   port 8642 — chat  (`/v1/chat/completions`)
//!   port 8643 — embed (`/v1/embeddings`)
//!
//! Both servers are loopback-only and are owned by this process.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::engine::sidecar;
use crate::error::Error;
use crate::prompts;

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("reqwest client")
    })
}

fn chat_url() -> String {
    format!("http://127.0.0.1:{}", sidecar::CHAT_PORT)
}
fn embed_url() -> String {
    format!("http://127.0.0.1:{}", sidecar::EMBED_PORT)
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    temperature: f32,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize, Clone)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMsg,
}

#[derive(Deserialize)]
struct ChatMsg {
    content: Option<String>,
}

/// One chat completion. `model` is accepted by llama-server but may be any
/// string; we always load exactly one model per server.
pub async fn chat(system: &str, user: &str, temperature: f32, max_tokens: u32) -> Result<String, Error> {
    let body = ChatRequest {
        model: "local",
        messages: &[
            Message {
                role: "system",
                content: system.to_string(),
            },
            Message {
                role: "user",
                content: user.to_string(),
            },
        ],
        temperature,
        max_tokens,
        stream: false,
    };
    let resp = client()
        .post(format!("{}/v1/chat/completions", chat_url()))
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Engine(format!("chat server unreachable: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Engine(format!("chat server {status}: {}", text.chars().take(200).collect::<String>())));
    }
    let parsed: ChatResponse = resp.json().await.map_err(Error::Http)?;
    Ok(parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default())
}

// ── Embeddings ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
    embedding: Vec<f32>,
}

/// Embed a batch of texts. Returns vectors in input order.
pub async fn embed(texts: &[String]) -> Result<Vec<Vec<f32>>, Error> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let resp = client()
        .post(format!("{}/v1/embeddings", embed_url()))
        .json(&serde_json::json!({ "input": texts }))
        .send()
        .await
        .map_err(|e| Error::Engine(format!("embed server unreachable: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Engine(format!(
            "embed server {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    let parsed: EmbedResponse = resp.json().await.map_err(Error::Http)?;
    Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
}

// ── High-level prompt runners ───────────────────────────────────────────────

/// Extract the first balanced JSON object from a model response, tolerating
/// markdown fences and preamble.
pub fn extract_json(raw: &str) -> Option<serde_json::Value> {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s)
        .trim();
    let s = s.trim_end_matches("```").trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
        return Some(v);
    }
    let start = s.find('{')?;
    let end = rfind('}', &s[start..])? + start;
    let candidate = &s[start..=end];
    serde_json::from_str(candidate).ok()
}

fn rfind(c: char, s: &str) -> Option<usize> {
    s.char_indices()
        .rev()
        .find(|(_, ch)| *ch == c)
        .map(|(i, _)| i + c.len_utf8())
}

/// Brain-dump → checklist. The flagship prompt; returns the task strings.
pub async fn clean_checklist(rant: &str) -> Result<Vec<String>, Error> {
    let system = prompts::get("clean_checklist.system");
    let user = prompts::fill("clean_checklist.user", &[("text", rant)]);
    let raw = chat(&system, &user, 0.1, 512).await?;
    Ok(parse_tasks(&raw))
}

/// Lenient parser — mirrors the lessons from the web app:
/// first JSON object, else line-split.
pub fn parse_tasks(raw: &str) -> Vec<String> {
    if let Some(obj) = extract_json(raw) {
        if let Some(arr) = obj["tasks"].as_array() {
            let mut out = Vec::new();
            for t in arr {
                if let Some(s) = t.as_str() {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                }
            }
            return out;
        }
    }
    raw.lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•', 'x', 'X']))
        .map(|l| l.trim_start_matches(|c: char| c.is_ascii_digit()))
        .map(|l| l.trim().trim_start_matches(['.', ' ']))
        .filter(|l| l.len() >= 3)
        .take(30)
        .map(|s| s.to_string())
        .collect()
}

/// Annotation for the thought graph: summary, entities, and which previous
/// thoughts this one relates to.
pub async fn annotate(
    text: &str,
    recent_thoughts: &[(String, String)],
) -> Result<crate::engine::pipeline::Annotation, Error> {
    use crate::engine::pipeline::Annotation;
    let system = prompts::get("annotate.system");
    let ctx = if recent_thoughts.is_empty() {
        "(no earlier thoughts yet)".to_string()
    } else {
        recent_thoughts
            .iter()
            .enumerate()
            .map(|(i, (t, s))| format!("[{i}] {t} — {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let user = prompts::fill(
        "annotate.user",
        &[("text", text), ("recent_thoughts", &ctx)],
    );
    let raw = chat(&system, &user, 0.2, 700).await?;
    let v = extract_json(&raw).unwrap_or(serde_json::json!({}));

    fn str_arr(v: &serde_json::Value, key: &str) -> Vec<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn obj_arr(v: &serde_json::Value, key: &str) -> Vec<crate::engine::pipeline::TaskDraft> {
        use crate::engine::pipeline::TaskDraft;
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        let title = o["title"].as_str().map(|s| s.trim().to_string())?;
                        if title.is_empty() {
                            return None;
                        }
                        Some(TaskDraft {
                            title,
                            project: o["project"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                            due_hint: o["due_hint"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // "related" is a list of *indices* into the recent-thought list we passed.
    let related: Vec<String> = v["related"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    let idx = x.as_u64()? as usize;
                    recent_thoughts.get(idx).map(|(id, _)| id.clone())
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Annotation {
        summary: v["summary"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        tasks: obj_arr(&v, "tasks"),
        topics: str_arr(&v, "topics"),
        people: str_arr(&v, "people"),
        goals: str_arr(&v, "goals"),
        related,
        sentiment: v["sentiment"]
            .as_str()
            .map(|s| s.to_lowercase())
            .filter(|s| ["positive", "neutral", "negative"].contains(&s.as_str()))
            .unwrap_or_else(|| "neutral".to_string()),
    })
}

/// Lightweight fog/friction signal for the Today view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FogSignal {
    pub tag: String,
    pub context: String,
    pub intensity: f64,
    #[serde(default)]
    pub question: String,
}

pub async fn fog(text: &str) -> Result<Vec<FogSignal>, Error> {
    let system = prompts::get("fog.system");
    let user = prompts::fill("fog.user", &[("text", text)]);
    let raw = chat(&system, &user, 0.2, 300).await?;
    let v = extract_json(&raw).unwrap_or(serde_json::json!({}));
    Ok(v["signals"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    Some(FogSignal {
                        tag: o["tag"].as_str()?.to_string(),
                        context: o["context"].as_str().unwrap_or("").to_string(),
                        intensity: o["intensity"].as_f64().unwrap_or(0.5),
                        question: o["question"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Daily brief for the Today view (friction + suggestion).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyBrief {
    pub friction: Option<String>,
    pub suggestion: Option<String>,
}

/// Owned fields — the struct crosses into an async task, so it must be `Send`.
#[derive(Debug, Clone, Serialize)]
pub struct BriefInput {
    pub open_tasks: usize,
    pub open_task_list: String,
    pub thoughts_today: usize,
    pub events_today: usize,
    pub scheduled_hours: f64,
    pub focus_minutes: i64,
    pub fog_json: String,
}

pub async fn daily_brief(input: BriefInput) -> Result<DailyBrief, Error> {
    let system = prompts::get("daily_brief.system");
    let user = prompts::fill(
        "daily_brief.user",
        &[
            ("open_tasks", &input.open_tasks.to_string()),
            ("open_task_list", &input.open_task_list),
            ("thoughts_today", &input.thoughts_today.to_string()),
            ("events_today", &input.events_today.to_string()),
            ("scheduled_hours", &format!("{:.1}", input.scheduled_hours)),
            ("focus_minutes", &input.focus_minutes.to_string()),
            ("fog_json", &input.fog_json),
        ],
    );
    let raw = chat(&system, &user, 0.3, 220).await?;
    let v = extract_json(&raw).unwrap_or(serde_json::json!({}));
    Ok(DailyBrief {
        friction: v["friction"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        suggestion: v["suggestion"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
    })
}

/// Command parser: "add a task to email Sam" → structured intent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommandIntent {
    pub intent: String, // "create_task" | "start_focus" | "unknown"
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub minutes: Option<u32>,
}

pub async fn parse_command(text: &str) -> Result<CommandIntent, Error> {
    let now = crate::time::now_secs().to_string();
    let system = prompts::fill("parse_command.system", &[("now", &now)]);
    let user = prompts::fill("parse_command.user", &[("text", text)]);
    let raw = chat(&system, &user, 0.0, 120).await?;
    let v = extract_json(&raw).unwrap_or(serde_json::json!({}));
    Ok(CommandIntent {
        intent: v["intent"].as_str().unwrap_or("unknown").to_string(),
        title: v["title"].as_str().map(|s| s.to_string()),
        minutes: v["minutes"].as_u64().map(|m| m as u32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_with_fences() {
        let raw = "Sure!\n```json\n{\"tasks\": [\"a\", \"b\"]}\n```\nbye";
        let v = extract_json(raw).unwrap();
        assert_eq!(v["tasks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn extracts_json_with_preamble() {
        let raw = "Here is the list: {\"tasks\": [\"one\"]} hope that helps";
        let v = extract_json(raw).unwrap();
        assert_eq!(v["tasks"][0].as_str(), Some("one"));
    }

    #[test]
    fn parse_tasks_fallback_lines() {
        let raw = "- Call the bank\n* Renew passport\n3. Walk the dog";
        let t = parse_tasks(raw);
        assert_eq!(t, vec!["Call the bank", "Renew passport", "Walk the dog"]);
    }
}
