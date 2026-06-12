use std::collections::HashMap;
use chrono::Utc;
use crate::db;

/// Compute Levenshtein edit distance between two strings (space-efficient O(n) variant).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 { return n; }
    if n == 0 { return m; }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1]
            } else {
                1 + prev[j - 1].min(prev[j]).min(curr[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Compute normalised similarity in [0.0, 1.0] between two labels.
/// 1.0 = identical, 0.0 = completely different.
pub fn label_similarity(a: &str, b: &str) -> f64 {
    let a = a.trim().to_lowercase();
    let b = b.trim().to_lowercase();
    if a == b { return 1.0; }
    if a.is_empty() && b.is_empty() { return 1.0; }
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 { return 1.0; }
    let dist = levenshtein(&a, &b);
    1.0 - (dist as f64 / max_len as f64)
}

/// Find an existing graph node whose label is similar enough (≥ 0.82) to
/// the given label and matches the given node_type.  Returns its ID if found.
pub fn find_similar_node(
    conn: &rusqlite::Connection,
    label: &str,
    node_type: &str,
) -> Option<String> {
    const DEDUP_THRESHOLD: f64 = 0.82;
    let nodes = db::get_nodes(conn).ok()?;
    nodes
        .into_iter()
        .filter(|n| node_type.is_empty() || n.node_type == node_type)
        .find(|n| label_similarity(label, &n.label) >= DEDUP_THRESHOLD)
        .map(|n| n.id)
}

/// Compute a momentum score in [0.0, 1.0] from raw note counts and sentiment.
///
/// momentum = (notes_last_7d / max(notes_prior_7d, 1)) * 0.6
///          + avg_positive_ratio * 0.4
///
/// Clamped to [0.0, 1.0].
pub fn compute_momentum(mentions_last_7d: i64, mentions_prior_7d: i64, positive_ratio: f64) -> f64 {
    let notes_ratio = (mentions_last_7d as f64) / (mentions_prior_7d.max(1) as f64);
    // Cap the ratio contribution at 1.0 so a very active node doesn't overflow
    let notes_contrib = notes_ratio.min(1.0) * 0.6;
    let sentiment_contrib = positive_ratio.clamp(0.0, 1.0) * 0.4;
    (notes_contrib + sentiment_contrib).clamp(0.0, 1.0)
}

/// Compute momentum scores for all project and goal nodes, returning a map
/// of `node_id -> score`.  Uses `mentions_count` from the node's `data` JSON
/// and the `created_at` date as a recency proxy.
pub fn compute_all_momentum_scores(conn: &rusqlite::Connection) -> HashMap<String, f64> {
    let mut scores = HashMap::new();

    let nodes = match db::get_nodes(conn) {
        Ok(n) => n,
        Err(_) => return scores,
    };

    let now = Utc::now();
    let cutoff_7d = (now - chrono::Duration::days(7)).to_rfc3339();
    let cutoff_14d = (now - chrono::Duration::days(14)).to_rfc3339();

    for node in nodes.iter().filter(|n| n.node_type == "project" || n.node_type == "goal") {
        let data: serde_json::Value = node
            .data
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let mentions_count = data
            .get("mentions_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            .max(1);

        let sentiment = data
            .get("sentiment")
            .and_then(|v| v.as_str())
            .unwrap_or("neutral");

        // Rough recency: if created within last 7 days → recent; within 14 days → prior
        let is_recent = node.created_at > cutoff_7d;
        let is_prior_week = node.created_at > cutoff_14d && !is_recent;

        let mentions_last_7d = if is_recent { mentions_count } else { 0 };
        let mentions_prior_7d = if is_prior_week { mentions_count } else { 1 };

        let positive_ratio = match sentiment {
            "positive" => 0.8,
            "negative" => 0.2,
            _ => 0.5,
        };

        let score = compute_momentum(mentions_last_7d, mentions_prior_7d, positive_ratio);
        scores.insert(node.id.clone(), score);
    }

    scores
}

/// Compute which goal nodes are drifting (not recently mentioned).
/// Returns a list of (node_id, goal_label, days_since_updated, drift_score).
pub fn compute_drift_alerts(conn: &rusqlite::Connection) -> Vec<(String, String, i64, f64)> {
    let mut alerts = Vec::new();

    let nodes = match db::get_nodes(conn) {
        Ok(n) => n,
        Err(_) => return alerts,
    };

    let now = Utc::now();
    let seven_days_ago = (now - chrono::Duration::days(7)).to_rfc3339();

    for node in nodes.iter().filter(|n| n.node_type == "goal") {
        // If the node's created_at or last_mentioned is older than 7 days, it's drifting
        if node.created_at < seven_days_ago {
            let created = chrono::DateTime::parse_from_rfc3339(&node.created_at)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(Utc::now());
            let days_since = (now - created).num_days().max(0);
            let drift_score = (days_since as f64 / 30.0).min(1.0);
            alerts.push((node.id.clone(), node.label.clone(), days_since, drift_score));
        }
    }

    // Sort by drift_score descending (most severe first)
    alerts.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
    alerts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_levenshtein_similar() {
        // "AICamp" vs "AI Camp Dallas" — should have some distance
        let dist = levenshtein("aicamp", "ai camp dallas");
        assert!(dist > 0);
    }

    #[test]
    fn test_label_similarity_identical() {
        assert_eq!(label_similarity("LocalizeAI", "LocalizeAI"), 1.0);
    }

    #[test]
    fn test_label_similarity_high() {
        let sim = label_similarity("AICamp", "AI Camp");
        assert!(sim > 0.6, "Expected similarity > 0.6, got {}", sim);
    }

    #[test]
    fn test_compute_momentum_clamp() {
        let score = compute_momentum(100, 1, 1.0);
        assert!(score <= 1.0 && score >= 0.0);
    }
}
