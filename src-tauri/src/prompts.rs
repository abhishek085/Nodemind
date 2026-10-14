//! Prompt loading.
//!
//! Prompts live in `resources/prompts.toml` — a plain TOML file that sits in
//! the app bundle, editable at runtime without recompiling (see Settings →
//! "Open prompts file"). If the file is missing or unparseable, a compiled
//! fallback is used so the app never fails to start over prompts.
//!
//! Template syntax: `{placeholder_name}`, substituted via [`fill`].

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
struct Section {
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    value: Option<String>,
    /// dotted keys, e.g. [stt] hindi_initial_prompt
    #[serde(flatten)]
    extras: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
struct PromptFile {
    #[serde(flatten)]
    sections: BTreeMap<String, Section>,
}

fn parse(text: &str) -> Vec<(String, String)> {
    let Ok(file) = toml::from_str::<PromptFile>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (section, sec) in file.sections {
        if let Some(t) = sec.template {
            out.push((format!("{section}.template"), t));
        }
        if let Some(s) = sec.system {
            out.push((format!("{section}.system"), s));
        }
        if let Some(v) = sec.value {
            out.push((section.clone(), v));
        }
        for (k, v) in sec.extras {
            if k != "template" && k != "system" && k != "value" {
                out.push((format!("{section}.{k}"), v));
            }
        }
    }
    out
}

static PROMPTS: OnceLock<BTreeMap<String, String>> = OnceLock::new();

/// Load prompts: runtime file first, embedded fallback for anything missing.
/// Called once from `tauri::Builder::setup`.
pub fn init(resource_dir: &std::path::Path) {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(resource_dir.join("resources/prompts.toml")) {
        for (k, v) in parse(&text) {
            map.insert(k, v);
        }
    }
    if map.is_empty() {
        eprintln!("[prompts] runtime file missing/unreadable — using embedded prompts");
    }
    for (k, v) in parse(include_str!("../resources/prompts.toml")) {
        map.entry(k).or_insert(v);
    }
    let _ = PROMPTS.set(map);
}

/// Fetch a prompt by dotted key. Never fails — returns a marker string if
/// the key is absent (surfaces as an obvious model error, not a crash).
pub fn get(key: &str) -> String {
    PROMPTS
        .get()
        .and_then(|m| m.get(key))
        .cloned()
        .unwrap_or_else(|| format!("[missing prompt: {key}]"))
}

/// Substitute `{name}` placeholders.
pub fn fill(key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = get(key);
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_extras() {
        let text = r#"
[stt]
hindi_initial_prompt = "This is a Hinglish conversation."

[clean_checklist]
system = """You convert speech into tasks."""
user = """Rant: {text}"""
"#;
        let parsed = parse(text);
        let map: BTreeMap<String, String> = parsed.into_iter().collect();
        assert!(map.contains_key("stt.hindi_initial_prompt"));
        assert!(map.get("clean_checklist.user").unwrap().contains("{text}"));
    }

    #[test]
    fn fill_replaces_placeholders() {
        let mut map = BTreeMap::new();
        map.insert("x".into(), "Hello {name}!".into());
        PROMPTS.set(map).unwrap();
        assert_eq!(fill("x", &[("name", "Sam")]), "Hello Sam!");
    }
}
