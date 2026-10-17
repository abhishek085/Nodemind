//! Meeting mode: a two-voice transcript and the minutes made from it.
//!
//! The mic is "You"; the Mac's audio output (via `nodemind-helper tap`) is
//! "Them". Because the two sources are captured separately, every line is
//! labelled by where it came from — no speaker-guessing model needed.
//!
//! On speakers (not headphones) the mic also hears the other side, so a
//! "You" line that repeats a nearby "Them" line is dropped as echo.
//!
//! Minutes are map-reduce so an hour-long call fits the small model's context:
//! ~1,800-word chunks → notes in parallel → one merged summary.

use serde::{Deserialize, Serialize};

use crate::engine::llm::{chat, extract_json};
use crate::error::Error;
use crate::prompts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    You,
    Them,
}

impl Speaker {
    fn label(self) -> &'static str {
        match self {
            Speaker::You => "You",
            Speaker::Them => "Them",
        }
    }
}

/// One transcribed utterance, stamped with when it started.
#[derive(Debug, Clone)]
pub struct Line {
    pub at_ms: u64,
    pub speaker: Speaker,
    pub text: String,
}

/// How close in time (and how similar) a "You" line must be to a "Them"
/// line to count as the mic picking up the speakers.
const ECHO_WINDOW_MS: u64 = 4_000;
const ECHO_SIMILARITY: f64 = 0.5;

/// Time-ordered "You: … / Them: …" transcript, echo removed, consecutive
/// lines from the same speaker merged.
pub fn render(lines: &[Line]) -> String {
    let mut sorted: Vec<&Line> = lines.iter().collect();
    sorted.sort_by_key(|l| l.at_ms);
    let mut out: Vec<(Speaker, String)> = Vec::new();
    for line in sorted {
        if line.speaker == Speaker::You && is_echo(line, lines) {
            continue;
        }
        match out.last_mut() {
            Some((sp, text)) if *sp == line.speaker => {
                text.push(' ');
                text.push_str(&line.text);
            }
            _ => out.push((line.speaker, line.text.clone())),
        }
    }
    out.into_iter()
        .map(|(sp, text)| format!("{}: {text}", sp.label()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_echo(you: &Line, lines: &[Line]) -> bool {
    lines.iter().any(|l| {
        l.speaker == Speaker::Them
            && l.at_ms.abs_diff(you.at_ms) <= ECHO_WINDOW_MS
            && word_overlap(&you.text, &l.text) >= ECHO_SIMILARITY
    })
}

/// Jaccard similarity of the two texts' lowercase word sets.
fn word_overlap(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> std::collections::HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / a.union(&b).count() as f64
}

// ── Minutes ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Minutes {
    pub title: String,
    pub summary: String,
    pub decisions: Vec<String>,
    pub action_items: Vec<ActionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    pub owner: String,
    pub task: String,
    #[serde(default)]
    pub due: String,
}

impl ActionItem {
    /// Taken on by the Nodemind user — these go onto Today.
    pub fn is_mine(&self) -> bool {
        matches!(self.owner.trim().to_lowercase().as_str(), "you" | "me" | "i" | "")
    }
}

const CHUNK_WORDS: usize = 1_800;

/// Minutes for a rendered meeting transcript.
pub async fn minutes(transcript: &str) -> Result<Minutes, Error> {
    let chunks = chunk_lines(transcript, CHUNK_WORDS);
    let parts: Vec<Minutes> = futures_util::future::join_all(chunks.iter().map(|c| chunk_minutes(c)))
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    if parts.is_empty() {
        return Err(Error::Engine("could not summarise the meeting".into()));
    }

    let mut out = Minutes::default();
    let mut seen = std::collections::HashSet::new();
    for p in &parts {
        for d in &p.decisions {
            if seen.insert(crate::engine::tasks::task_key(d)) {
                out.decisions.push(d.clone());
            }
        }
        for a in &p.action_items {
            if seen.insert(crate::engine::tasks::task_key(&a.task)) {
                out.action_items.push(a.clone());
            }
        }
    }
    if parts.len() == 1 {
        out.title = parts[0].title.clone();
        out.summary = parts[0].summary.clone();
    } else {
        let joined = parts.iter().map(|p| p.summary.as_str()).collect::<Vec<_>>().join("\n");
        let raw = chat(
            &prompts::get("meeting_reduce.system"),
            &prompts::fill("meeting_reduce.user", &[("text", &joined)]),
            0.1,
            300,
        )
        .await?;
        let v = extract_json(&raw).unwrap_or_default();
        out.title = v["title"].as_str().unwrap_or(&parts[0].title).to_string();
        out.summary = v["summary"].as_str().map(str::to_string).unwrap_or(joined);
    }
    Ok(out)
}

/// Minutes for one chunk: the summary prompt and the action-item prompt run
/// in parallel (separate focused prompts recall far more for a small model).
async fn chunk_minutes(text: &str) -> Result<Minutes, Error> {
    let (notes_sys, notes_user) =
        (prompts::get("meeting_chunk.system"), prompts::fill("meeting_chunk.user", &[("text", text)]));
    let (actions_sys, actions_user) =
        (prompts::get("meeting_actions.system"), prompts::fill("meeting_actions.user", &[("text", text)]));
    let (notes, actions) = tokio::join!(
        chat(&notes_sys, &notes_user, 0.1, 400),
        chat(&actions_sys, &actions_user, 0.0, 500),
    );
    // A small model occasionally emits broken JSON; one retry is cheap (~0.3 s).
    let v = match extract_json(&notes?) {
        Some(v) => v,
        None => extract_json(&chat(&notes_sys, &notes_user, 0.3, 400).await?)
            .ok_or_else(|| Error::Engine("minutes were not JSON".into()))?,
    };
    let decisions = v["decisions"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    // Action items are best-effort: minutes without them beat no minutes.
    let action_items = actions
        .ok()
        .and_then(|raw| extract_json(&raw))
        .and_then(|a| a["action_items"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            let task = x["task"].as_str()?.trim();
            (!task.is_empty()).then(|| ActionItem {
                owner: x["owner"].as_str().unwrap_or("").trim().to_string(),
                task: task.to_string(),
                due: x["due"].as_str().unwrap_or("").trim().to_string(),
            })
        })
        .collect();
    Ok(Minutes {
        title: v["title"].as_str().unwrap_or("Meeting").trim().to_string(),
        summary: v["summary"].as_str().unwrap_or("").trim().to_string(),
        decisions,
        action_items,
    })
}

/// Split a transcript into chunks of about `max_words`, on line boundaries.
fn chunk_lines(text: &str, max_words: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut words = 0;
    for line in text.lines() {
        let n = line.split_whitespace().count();
        if words > 0 && words + n > max_words {
            chunks.push(std::mem::take(&mut cur));
            words = 0;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
        words += n;
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at_ms: u64, speaker: Speaker, text: &str) -> Line {
        Line { at_ms, speaker, text: text.into() }
    }

    #[test]
    fn renders_in_time_order_and_merges_turns() {
        let lines = [
            line(5_000, Speaker::Them, "Sounds good."),
            line(0, Speaker::You, "Can you send the deck?"),
            line(9_000, Speaker::Them, "I'll do it today."),
            line(14_000, Speaker::You, "Great, thanks."),
        ];
        assert_eq!(
            render(&lines),
            "You: Can you send the deck?\nThem: Sounds good. I'll do it today.\nYou: Great, thanks."
        );
    }

    #[test]
    fn drops_mic_echo_of_the_other_side() {
        let lines = [
            line(1_000, Speaker::Them, "We should ship the beta on Friday."),
            line(1_400, Speaker::You, "we should ship the beta on friday"),
            line(6_000, Speaker::You, "Agreed, I'll write the release notes."),
        ];
        assert_eq!(
            render(&lines),
            "Them: We should ship the beta on Friday.\nYou: Agreed, I'll write the release notes."
        );
    }

    #[test]
    fn chunks_respect_line_boundaries() {
        let text = (0..10).map(|i| format!("You: {}", "word ".repeat(50).trim().to_string() + &i.to_string())).collect::<Vec<_>>().join("\n");
        let chunks = chunk_lines(&text, 120);
        assert_eq!(chunks.len(), 5);
        assert!(chunks.iter().all(|c| c.lines().all(|l| l.starts_with("You: "))));
    }

    #[test]
    fn owner_you_is_mine() {
        let item = |o: &str| ActionItem { owner: o.into(), task: "x".into(), due: String::new() };
        assert!(item("You").is_mine() && item("").is_mine());
        assert!(!item("Them").is_mine() && !item("Rahul").is_mine());
    }

    /// Minutes from the real model (chat server on CHAT_PORT; prompts from the
    /// embedded file): `cargo test minutes_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn minutes_live() {
        crate::prompts::init(std::path::Path::new("/nonexistent"));
        let t = "You: Thanks for joining. Main thing today is the beta launch date.\n\
Them: Engineering thinks Friday is too tight. The sync bug is still open.\n\
You: Okay, what if we move it to next Wednesday?\n\
Them: Wednesday works. Priya will fix the sync bug by Monday.\n\
You: Great. I'll update the launch post and tell the waitlist.\n\
Them: Also someone needs to book the demo room for Thursday.\n\
You: I'll book the demo room too. So we're agreed, beta ships next Wednesday.\n\
Them: Agreed. Rahul will send the release notes draft tomorrow.";
        let m = minutes(t).await.expect("chat server on 8642");
        for a in &m.action_items {
            println!("  [{}] {} {}", a.owner, a.task, if a.due.is_empty() { String::new() } else { format!("(due {})", a.due) });
        }
        println!("  summary: {}", m.summary);
        let mine: Vec<_> = m.action_items.iter().filter(|a| a.is_mine()).map(|a| a.task.as_str()).collect();
        println!("mine: {mine:?}");
        assert!(!m.summary.is_empty());
        assert!(!mine.is_empty());
    }
}
