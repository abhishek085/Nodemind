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
    /// Qwen3.5 "thinks" before answering by default — slower, and the
    /// reasoning would leak into our JSON. Every Nodemind call is extraction,
    /// so thinking stays off.
    chat_template_kwargs: serde_json::Value,
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
        chat_template_kwargs: serde_json::json!({ "enable_thinking": false }),
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
    let text = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();
    Ok(strip_thinking(&text))
}

/// Drop any `<think>…</think>` block a model emits despite thinking being off.
fn strip_thinking(text: &str) -> String {
    match (text.find("<think>"), text.find("</think>")) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &text[..a], &text[b + "</think>".len()..]).trim().to_string(),
        (None, Some(b)) => text[b + "</think>".len()..].trim().to_string(),
        _ => text.to_string(),
    }
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

/// Embed statements for retrieval, with EmbeddingGemma's document prefix.
pub async fn embed_documents(texts: &[String]) -> Result<Vec<Vec<f32>>, Error> {
    let prefixed: Vec<String> = texts.iter().map(|t| format!("title: none | text: {t}")).collect();
    embed(&prefixed).await
}

/// Embed a search query, with EmbeddingGemma's retrieval-query prefix.
/// (The prefixes widened the right-vs-runner-up margin ~25% in our tests.)
pub async fn embed_query(query: &str) -> Result<Vec<f32>, Error> {
    embed(&[format!("task: search result | query: {query}")])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Engine("embed server returned no vector".into()))
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
    // Parse the first complete value starting at the first '{' and ignore
    // whatever follows it (commentary, a second object, stray fences).
    let start = s.find('{')?;
    let first = serde_json::Deserializer::from_str(&s[start..])
        .into_iter::<serde_json::Value>()
        .next();
    match first {
        Some(Ok(v)) => Some(v),
        // Small models sometimes stop before the last closing brace(s).
        _ => serde_json::from_str(&close_brackets(&s[start..])).ok(),
    }
}

/// Append whatever `}`/`]` an otherwise complete JSON prefix is missing
/// (ignores brackets inside strings).
fn close_brackets(s: &str) -> String {
    let mut stack: Vec<char> = Vec::new();
    let (mut in_str, mut esc) = (false, false);
    for c in s.chars() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }
    let mut out = s.trim_end().trim_end_matches("```").trim_end().to_string();
    while let Some(c) = stack.pop() {
        out.push(c);
    }
    out
}

/// Annotation for the thought graph: a one-line summary and the entities
/// (topics, people, goals) the thought is about. Related thoughts are found
/// by EmbeddingGemma 2 instead (engine::links), not guessed by the LLM.
pub async fn annotate(text: &str) -> Result<crate::engine::pipeline::Annotation, Error> {
    use crate::engine::pipeline::Annotation;
    let raw = chat(
        &prompts::get("annotate.system"),
        &prompts::fill("annotate.user", &[("text", text)]),
        0.2,
        400,
    )
    .await?;
    let v = extract_json(&raw).unwrap_or(serde_json::json!({}));
    let list = |key: &str, kind: &str| -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.trim().trim_end_matches('.').to_string())
                    .filter(|s| !s.is_empty() && !crate::engine::links::is_junk_entity(kind, s))
                    .filter(|s| crate::engine::links::is_grounded(kind, s, text))
                    .filter(|s| seen.insert(s.to_lowercase()))
                    .take(5)
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(Annotation {
        summary: v["summary"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        topics: list("topics", "topic"),
        people: list("people", "person"),
        goals: list("goals", "goal"),
        sentiment: v["sentiment"]
            .as_str()
            .map(|s| s.to_lowercase())
            .filter(|s| ["positive", "neutral", "negative"].contains(&s.as_str()))
            .unwrap_or_else(|| "neutral".to_string()),
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
    fn extracts_json_ending_the_response() {
        // Regression: a closing brace as the very last byte used to panic.
        let raw = "Minutes: {\"title\": \"Beta\", \"decisions\": [\"Ship Wednesday\"]}";
        assert_eq!(extract_json(raw).unwrap()["title"].as_str(), Some("Beta"));
    }

    #[test]
    fn strips_thinking_blocks() {
        assert_eq!(strip_thinking("<think>hmm</think>\n{\"tasks\": []}"), "{\"tasks\": []}");
        assert_eq!(strip_thinking("reasoning…</think>{}"), "{}");
        assert_eq!(strip_thinking("{\"a\": 1}"), "{\"a\": 1}");
    }

    #[test]
    fn repairs_missing_closing_brace() {
        let raw = r#"{"signals": [{"tag": "overthinking", "context": "launch {date}", "question": "Why?"}]"#;
        let v = extract_json(raw).unwrap();
        assert_eq!(v["signals"][0]["tag"].as_str(), Some("overthinking"));
        assert_eq!(v["signals"][0]["context"].as_str(), Some("launch {date}"));
    }

    #[test]
    fn extracts_first_of_two_objects() {
        let raw = "{\"a\": 1}\n{\"b\": 2}";
        assert_eq!(extract_json(raw).unwrap()["a"].as_i64(), Some(1));
    }

}
