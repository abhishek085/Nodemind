//! Fog: where the user's thinking is stuck (overthinking, unclarity,
//! abstraction, overwhelm), each with one question to get unstuck.
//!
//! A 0.8B model flags "fog" in almost anything, including calm task lists, so
//! detection is model + rules:
//!   1. cue gate — if the words contain no fog cue at all, no model call;
//!   2. the model names the fog, what it's about, and an unsticking question;
//!   3. verify — a tag is kept only when its own cues are in the words; a
//!      mislabel is moved to the one tag whose cues are present, else dropped.

use serde::{Deserialize, Serialize};

use crate::engine::llm::{chat, extract_json};
use crate::prompts;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FogSignal {
    /// overthinking | unclarity | abstraction | overwhelmed
    pub tag: String,
    /// What they're stuck on, a few words.
    pub context: String,
    /// One question that would get them moving.
    pub question: String,
}

/// Phrases that show each kind of fog in spoken English (lowercase).
const CUES: &[(&str, &[&str])] = &[
    ("overthinking", &[
        "back and forth", "what if", "but then again", "then again", "keep thinking", "keep going back",
        "can't decide", "cannot decide", "can't make up my mind", "not sure whether", "not sure if",
        "still don't know", "i don't know if", "second-guess", "second guess", "over and over", "again and again",
        "on the other hand", "should i or", "torn between",
    ]),
    ("unclarity", &[
        "at some point", "somehow", "something about", "do something", "look into", "some stuff", "stuff like that",
        "probably should", "maybe i should", "figure it out", "figure out what",
        "not sure what", 
    ]),
    ("abstraction", &[
        "vision", "big picture", "in general", "humanity", "change the world", "someday", "the future of",
        "the idea is", "conceptually", "in theory", "fundamentally", "paradigm", "philosophy",
    ]),
    ("overwhelmed", &[
        "overwhelmed", "too much", "too many", "piling up", "don't know where to start", "where do i even start",
        "drowning", "buried", "so much to do", "everything at once", "can't keep up", "falling behind",
        "stressed", "burnt out", "burned out", "exhausted",
    ]),
];

/// Tags with at least two distinct cues in the words. One stray phrase
/// ("I still don't know whether…") is a question, not fog.
fn cued_tags(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    CUES.iter()
        .filter(|(_, cues)| cues.iter().filter(|c| lower.contains(*c)).count() >= 2)
        .map(|(tag, _)| *tag)
        .collect()
}

/// Detect fog in what the user said. Empty for clear speech (most of it).
pub async fn detect(text: &str) -> Vec<FogSignal> {
    let cued = cued_tags(text);
    if cued.is_empty() || text.split_whitespace().count() < 25 {
        return Vec::new();
    }
    let raw = match chat(
        &prompts::get("fog.system"),
        &prompts::fill("fog.user", &[("text", text)]),
        0.2,
        300,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[fog] model call failed: {e}");
            return Vec::new();
        }
    };
    verify(parse(&raw), &cued)
}

fn parse(raw: &str) -> Vec<FogSignal> {
    extract_json(raw)
        .and_then(|v| v["signals"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|o| {
            Some(FogSignal {
                tag: o["tag"].as_str()?.trim().to_lowercase(),
                context: short(o["context"].as_str().unwrap_or(""), 10),
                question: o["question"].as_str().unwrap_or("").trim().to_string(),
            })
        })
        .collect()
}

/// Keep only signals backed by cues in the words (relabelling when exactly
/// one cued tag is left to move to); one signal per tag, at most two.
fn verify(signals: Vec<FogSignal>, cued: &[&str]) -> Vec<FogSignal> {
    let mut out: Vec<FogSignal> = Vec::new();
    for mut s in signals {
        if !cued.contains(&s.tag.as_str()) {
            let free: Vec<&&str> = cued.iter().filter(|t| !out.iter().any(|o| o.tag == **t)).collect();
            match free.as_slice() {
                [only] => s.tag = only.to_string(),
                _ => continue,
            }
        }
        if s.question.is_empty() || out.iter().any(|o| o.tag == s.tag) {
            continue;
        }
        out.push(s);
        if out.len() == 2 {
            break;
        }
    }
    out
}

/// First `n` words, with an ellipsis when cut.
fn short(s: &str, n: usize) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let cut = words.len() > n;
    let mut out = words.into_iter().take(n).collect::<Vec<_>>().join(" ");
    out = out.trim_end_matches(['.', ',']).to_string();
    if cut {
        out.push('…');
    }
    out
}

/// Display name for a tag ("overthinking" → "Overthinking").
pub fn label(tag: &str) -> String {
    let mut c = tag.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(tag: &str) -> FogSignal {
        FogSignal { tag: tag.into(), context: "x".into(), question: "q?".into() }
    }

    #[test]
    fn calm_speech_has_no_cues() {
        assert!(cued_tags("Today I need to call Priya about the invoice, finish the slides and book the car service.").is_empty());
        assert!(cued_tags("Okay I've decided, we launch next Wednesday. Priya handles the sync bug.").is_empty());
    }

    #[test]
    fn cues_pick_the_right_tags() {
        assert_eq!(cued_tags("I keep going back and forth on the launch, what if it breaks"), vec!["overthinking"]);
        assert_eq!(cued_tags("I should do something about marketing at some point"), vec!["unclarity"]);
        // one stray phrase is not fog
        assert!(cued_tags("We decided on nine dollars. I still don't know whether to offer a free tier.").is_empty());
        assert!(cued_tags("The weather is purple. Anyway whatever, the table sings.").is_empty());
        assert_eq!(cued_tags("everything is piling up and I don't know where to start"), vec!["overwhelmed"]);
    }

    #[test]
    fn unbacked_tags_are_dropped_or_relabelled() {
        // Model said "unclarity" but only overwhelm cues are present → relabel.
        let v = verify(vec![sig("unclarity")], &["overwhelmed"]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].tag, "overwhelmed");
        // Two cued tags and an unbacked label → ambiguous, dropped.
        assert!(verify(vec![sig("abstraction")], &["overwhelmed", "unclarity"]).is_empty());
        // Backed tags pass, duplicates collapse, max two.
        let v = verify(vec![sig("overthinking"), sig("overthinking"), sig("unclarity")], &["overthinking", "unclarity"]);
        assert_eq!(v.iter().map(|s| s.tag.as_str()).collect::<Vec<_>>(), vec!["overthinking", "unclarity"]);
    }

    #[test]
    fn context_is_kept_short() {
        assert_eq!(short("one two three four five six seven eight nine ten eleven", 10), "one two three four five six seven eight nine ten…");
    }

    /// Real model (chat server on CHAT_PORT): `cargo test fog_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn fog_live() {
        crate::prompts::init(std::path::Path::new("/nonexistent"));
        let cases = [
            ("overthinking", "I keep going back and forth on whether to launch Nodemind this month or next month. If I launch now it might be buggy but if I wait maybe someone else ships first, and then again maybe it doesn't matter, but what if it does. I thought about this yesterday too."),
            ("unclarity", "I should probably do something about the marketing at some point. Maybe look into stuff. Things around the website need work too I guess, at some point, somehow, I don't know exactly what yet."),
            ("overwhelmed", "There's the launch, the investor deck, my sister's wedding, the visa paperwork, three bugs in prod, and I haven't slept properly in a week. I don't know where to start, everything is piling up and it's too much."),
            ("abstraction", "I want Nodemind to become a second brain for everyone, a cognitive layer for humanity, where thoughts become knowledge and knowledge becomes wisdom. That's the vision, the big picture of where this all goes in the future."),
            ("", "Today I need to call Priya about the invoice, finish the slides for Monday, and book the car service. Then pick up groceries on the way home and send the report to the team before five."),
            ("", "Okay I've decided, we launch next Wednesday. Priya handles the sync bug, I'll write the post tonight and Rahul sends the release notes tomorrow morning. That's the plan, nice and simple."),
        ];
        let mut right = 0;
        for (want, text) in cases {
            let got = detect(text).await;
            let tags: Vec<&str> = got.iter().map(|s| s.tag.as_str()).collect();
            let ok = if want.is_empty() { tags.is_empty() } else { tags.first() == Some(&want) };
            right += ok as usize;
            println!("{} want {:12} got {:?} {}", if ok { "✓" } else { "✗" }, if want.is_empty() { "none" } else { want }, tags,
                got.first().map(|s| format!("— {} · {}", s.context, s.question)).unwrap_or_default());
        }
        println!("{right}/{}", cases.len());
        assert!(right >= 5);
    }
}
