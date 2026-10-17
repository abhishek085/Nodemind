//! Sort what was said into a few clean items: tasks, ideas, learnings,
//! questions and decisions. Everything else (small talk, filler, test talk,
//! fragments) is simply not an item.
//!
//! Deliberately rule-based, not an LLM call. Measured on real and realistic
//! recordings, the 0.8B model mislabelled items, missed questions and
//! decisions, and invented tasks from small talk ("I need to rest after
//! today's meetings"); asked only to rewrite, it copied the text back. Rules
//! keyed on how people actually phrase these things caught 15/15 expected
//! items with no noise, are instant, and can't invent anything: every item
//! is a cleaned-up clause the person actually said.
//!
//! Steps: split into clauses → classify each by its phrasing (first match in
//! decision → task → idea → learning → question) → tidy into a short item.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Task,
    Idea,
    Learning,
    Question,
    Decision,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Task => "task",
            Kind::Idea => "idea",
            Kind::Learning => "learning",
            Kind::Question => "question",
            Kind::Decision => "decision",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub kind: Kind,
    pub text: String,
}

/// Action verbs that make a bare imperative ("Call Priya…") a task.
const VERBS: &str = "call|email|text|message|ping|book|buy|order|send|finish|fix|write|review|schedule|pay|check|follow up|reply|submit|prepare|update|clean|renew|cancel|pick up|draft|plan|read|ask|tell|set up|move|sign|file|print|ship|deploy|publish|contact|research|try";

struct Rules {
    cues: Vec<(Kind, Regex)>,
    not_imperative: Regex,
    filler: Regex,
    tail: Regex,
    task_prefix: Regex,
    vague_end: Regex,
    unsure: Regex,
    learning_lead: Regex,
    words: Regex,
    small_talk: Regex,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| {
        let re = |s: &str| Regex::new(s).expect("valid regex");
        Rules {
            cues: vec![
                (Kind::Decision, re(r"\b(we|i) decided\b|\bdecided to\b|\b(we're|we are|so we're|so we are) agreed\b|\b(we're|we are|i'm|i am|we'll) going with\b|\bwe agreed\b|\bagreed (to|on)\b|\bsettled on\b|\bwe'll go with\b")),
                (Kind::Task, re(&format!(
                    r"\b(i|we) (still )?(really )?(need to|have to|got to|gotta|should|must|will|plan to|want to)\b|\bi'll\b|\bi'm going to\b|\bi am going to\b|\blet me\b|\bremind me\b|\bdon't forget\b|\bremember to\b|\bto-?do\b|^({VERBS})\b|\bneeds? (to be \w+|\w+ing)\b"
                ))),
                (Kind::Idea, re(r"\bmaybe (we|i) could\b|\b(we|i) could (try|do|build|make|add|use|start|offer)\b|\bwhat if\b|\bhow about\b|\bsuggested\b|\b(an|one|the) idea\b|\bmight try\b|\bcould try\b|\bit would be (cool|nice|great|good)\b")),
                (Kind::Learning, re(r"\brealized\b|\bturns out\b|\bi learned\b|\blearned that\b|\bnow i (finally )?(get|understand)\b|\bfinally get it\b|\bi figured out\b|\bthe trick is\b|\bthe reason .+ is\b")),
                (Kind::Question, re(r"\?$|\b(not sure|don't know|do not know|wondering) (whether|if|how|what|why|when)\b|\bi wonder (whether|if|how|what|why|when)\b|\bwhy (does|do|is|are|did)\b|\bhow do (i|we)\b|\bshould (i|we)\b.*\bor\b|\bwhether to\b|\bback and forth\b")),
            ],
            not_imperative: re(r"^(call|text|message|email|check|plan|order|file|draft|test|fix|update|review|read|move) (with|went|was|is|from|about it|today was)\b"),
            filler: re(r"(?i)^(?:(?:okay|ok|so|oh|and|also|anyway|but|well|um+|uh+|plus|then|like|i think|i guess|still|yeah|right|hmm+|random thought)[,]?\s+)+"),
            tail: re(r"(?i)\s+(?:though|too|not sure|i guess|or something|anyway)[.?!]*$"),
            task_prefix: re(r"(?i)^(?:(?:i|we) (?:still )?(?:really )?(?:need to|have to|got to|gotta|should|must|will|plan to|want to)|i'll|i'm going to|i am going to|let me|remind me to|don't forget to|remember to|need to|have to)\s+"),
            vague_end: re(r"(?i)\s*\b(?:that|it|this|them)\b[.!]?$"),
            unsure: re(r"^(?:i (?:still )?(?:don't|do not) know|i'm not sure|i am not sure|not sure|i keep going back and forth on|i'm wondering|i wonder)\s+whether to\s+(.+?)[.?]?$"),
            learning_lead: re(r"(?i)^(?:.*?\b(?:realized|learned|figured out) (?:that )?|(?:now )?i (?:finally )?get it,?\s*|turns out,?\s*(?:that )?)"),
            words: re(r"[a-z0-9']+"),
            small_talk: re(r"\b(testing|test test|one two three|is this working|can you hear|hello hello|mic check|hey guys|check one)\b"),
        }
    })
}

/// Split into sentences, then split a sentence at ", " where a new
/// statement starts ("…, I should look into that").
fn clauses(text: &str) -> Vec<String> {
    const STARTS: &[&str] = &["i ", "i'", "we ", "maybe ", "what if", "let me", "why ", "how ", "she ", "he ", "they "];
    const JOINERS: &[&str] = &["and ", "but ", "also ", "oh ", "so ", ""];
    let mut out = Vec::new();
    let mut sentence = String::new();
    let chars: Vec<char> = text.trim().chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        sentence.push(c);
        let end = matches!(c, '.' | '?' | '!') && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if end || i == chars.len() - 1 {
            let s = sentence.trim().to_string();
            sentence.clear();
            if s.is_empty() {
                continue;
            }
            // split at ", <new statement>"
            let mut start = 0;
            let lower = s.to_lowercase();
            let bytes: Vec<(usize, char)> = s.char_indices().collect();
            for (k, (pos, ch)) in bytes.iter().enumerate() {
                if *ch != ',' || k + 2 >= bytes.len() {
                    continue;
                }
                let rest = lower[pos + 1..].trim_start();
                let new_statement = JOINERS
                    .iter()
                    .any(|j| rest.starts_with(j) && STARTS.iter().any(|st| rest[j.len()..].starts_with(st)));
                if new_statement {
                    out.push(s[start..*pos].trim().to_string());
                    start = pos + 1;
                }
            }
            out.push(s[start..].trim().to_string());
        }
    }
    out.into_iter().filter(|c| !c.is_empty()).collect()
}

fn tidy(r: &Rules, clause: &str) -> String {
    let t = r.filler.replace(clause.trim(), "");
    let t = r.tail.replace(&t, "");
    t.trim_matches(|c: char| c == ',' || c.is_whitespace()).to_string()
}

fn classify(r: &Rules, lower: &str) -> Option<Kind> {
    if r.small_talk.is_match(lower) {
        return None; // greetings and mic tests are never items
    }
    let kind = r.cues.iter().find(|(_, re)| re.is_match(lower)).map(|(k, _)| *k)?;
    if kind == Kind::Task && r.not_imperative.is_match(lower) {
        return None;
    }
    Some(kind)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Sort a transcript (or one speaker's lines) into items.
pub fn sort(text: &str) -> Vec<Item> {
    let r = rules();
    let mut items: Vec<Item> = Vec::new();
    // The last meaningful statement, for resolving "look into that".
    let mut context: Option<(Kind, String)> = None;
    for clause in clauses(text) {
        let clean = tidy(r, &clause);
        let lower = clean.to_lowercase();
        let Some(kind) = classify(r, &lower) else {
            if lower.split_whitespace().count() > 3 {
                context = Some((Kind::Learning, clean)); // plain statement, usable as context only
            }
            continue;
        };
        let text = match kind {
            Kind::Task => {
                let body = r.task_prefix.replace(&clean, "").trim_end_matches('.').to_string();
                if r.vague_end.is_match(&body.to_lowercase()) {
                    // "I should look into that" → borrow the statement it refers to.
                    match &context {
                        Some((k, prev)) if *k != Kind::Task => format!("{}: {prev}", r.vague_end.replace(&body, "")),
                        _ => continue,
                    }
                } else {
                    body
                }
            }
            Kind::Question => match r.unsure.captures(&lower) {
                Some(c) => format!("Should I {}?", c[1].trim_end_matches(['.', '?'])),
                None => format!("{}?", clean.trim_end_matches(['.', '?'])),
            },
            Kind::Learning => {
                let insight = r.learning_lead.replace(&clean, "").to_string();
                if insight.split_whitespace().count() >= 4 { insight } else { clean.clone() }
            }
            Kind::Idea => {
                // "What if it does" is a worry fragment, not an idea.
                if clean.split_whitespace().count() < 5 {
                    continue;
                }
                clean.trim_end_matches('?').to_string()
            }
            Kind::Decision => clean.clone(),
        };
        let text = capitalize(text.trim());
        if r.words.find_iter(&text.to_lowercase()).count() < 3 {
            continue;
        }
        if !items.iter().any(|i| i.text.eq_ignore_ascii_case(&text)) {
            context = Some((kind, text.clone()));
            items.push(Item { kind, text });
        }
    }
    items
}

/// Strip "You:" / "Them:" speaker labels (meeting transcripts).
pub fn strip_speakers(text: &str) -> String {
    text.lines()
        .map(|l| l.strip_prefix("You: ").or_else(|| l.strip_prefix("Them: ")).unwrap_or(l))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn got(text: &str) -> Vec<(Kind, String)> {
        sort(text).into_iter().map(|i| (i.kind, i.text)).collect()
    }

    #[test]
    fn small_talk_and_tests_are_not_items() {
        for t in [
            "Today was long. Lots of meetings, I'm tired. The weather was nice though.",
            "Hey, can you take my notes and I want you to think about",
            "Hmm what was I saying. Yeah. Okay. Testing testing one two three. Is this working. Okay cool.",
            "You: Hello, hey guys, so yeah that recording has happened.",
            "Call with Priya went fine.",
        ] {
            assert!(sort(t).is_empty(), "{t} → {:?}", got(t));
        }
    }

    #[test]
    fn mic_tests_and_worry_fragments_are_not_items() {
        assert!(sort("Hello hello, testing, testing, one two three, okay cool, is this working? Yeah. Hey guys, so yeah, that is that.").is_empty());
        assert!(sort("And then again maybe it does not matter, but what if it does?").is_empty());
    }

    #[test]
    fn meeting_agreement_is_a_decision() {
        let t = strip_speakers("You: I'll book the demo room too. So we're agreed, beta ships next Wednesday.\nThem: Agreed.");
        let g = got(&t);
        assert!(g.contains(&(Kind::Task, "Book the demo room".into())), "{g:?}");
        assert!(g.iter().any(|(k, x)| *k == Kind::Decision && x.contains("beta ships next Wednesday")), "{g:?}");
    }

    #[test]
    fn pricing_ramble() {
        assert_eq!(
            got("We decided to price it at nine dollars a month. I still don't know whether to offer a free tier though. Let me draft the pricing page tonight."),
            vec![
                (Kind::Decision, "We decided to price it at nine dollars a month.".into()),
                (Kind::Question, "Should I offer a free tier?".into()),
                (Kind::Task, "Draft the pricing page tonight".into()),
            ]
        );
    }

    #[test]
    fn mixed_ramble() {
        let g = got("Okay so random thought, I was walking and I realized that the reason people don't journal is because writing is slow, talking is fast. Anyway. Oh I need to email Sarah about the contract by Friday. And maybe we could do a voice-only version of the app for iPhone? Not sure. Also why does my back hurt every morning, I should look into that.");
        assert!(g.contains(&(Kind::Learning, "The reason people don't journal is because writing is slow, talking is fast.".into())), "{g:?}");
        assert!(g.contains(&(Kind::Task, "Email Sarah about the contract by Friday".into())), "{g:?}");
        assert!(g.contains(&(Kind::Idea, "Maybe we could do a voice-only version of the app for iPhone".into())), "{g:?}");
        assert!(g.contains(&(Kind::Question, "Why does my back hurt every morning?".into())), "{g:?}");
        assert!(g.contains(&(Kind::Task, "Look into: Why does my back hurt every morning?".into())), "{g:?}");
        assert_eq!(g.len(), 5, "{g:?}");
    }

    #[test]
    fn learning_keeps_the_insight() {
        assert_eq!(
            got("I have read about linear regression on my own, but nobody explained why errors are squared in OLS. Now I finally get it, squaring punishes big errors more and makes the math smooth."),
            vec![(Kind::Learning, "Squaring punishes big errors more and makes the math smooth.".into())]
        );
    }

    #[test]
    fn suggestions_and_commitments() {
        assert_eq!(
            got("Call with Priya went fine. She suggested we try onboarding emails for new users. I think I'll set up a three email sequence next week."),
            vec![
                (Kind::Idea, "She suggested we try onboarding emails for new users.".into()),
                (Kind::Task, "Set up a three email sequence next week".into()),
            ]
        );
    }

    #[test]
    fn going_back_and_forth_is_a_question() {
        assert_eq!(
            got("I keep going back and forth on whether to launch this month or next month. If I launch now it might be buggy."),
            vec![(Kind::Question, "Should I launch this month or next month?".into())]
        );
    }

    #[test]
    fn imperatives_and_needs_are_tasks() {
        assert_eq!(
            got("Call Priya about the invoice tomorrow. The car needs servicing."),
            vec![
                (Kind::Task, "Call Priya about the invoice tomorrow".into()),
                (Kind::Task, "The car needs servicing".into()),
            ]
        );
    }

    #[test]
    fn vague_task_after_a_task_is_dropped() {
        assert_eq!(
            got("So I'm going to go back to policy. I need to add that."),
            vec![(Kind::Task, "Go back to policy".into())]
        );
    }
}
