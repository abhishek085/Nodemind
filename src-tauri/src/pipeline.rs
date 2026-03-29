use std::sync::{Arc, Mutex};
use std::time::{SystemTime, Duration};
use std::collections::BTreeSet;
use chrono::Utc;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use rusqlite::Connection;

/// Represents a segment of text with its timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub timestamp: SystemTime,
    pub rfc3339_timestamp: String,
}

/// Accumulates transcript segments into thought blocks for batched processing.
#[derive(Debug, Clone)]
pub struct ThoughtBlockBuffer {
    segments: Arc<Mutex<Vec<TranscriptSegment>>>,
    first_segment_time: Arc<Mutex<Option<SystemTime>>>,
    last_segment_time: Arc<Mutex<Option<SystemTime>>>,
    flush_duration: Duration,
}

impl ThoughtBlockBuffer {
    /// Creates a new buffer with a flush duration (default: 5 minutes).
    pub fn new(flush_duration: Duration) -> Self {
        ThoughtBlockBuffer {
            segments: Arc::new(Mutex::new(Vec::new())),
            first_segment_time: Arc::new(Mutex::new(None)),
            last_segment_time: Arc::new(Mutex::new(None)),
            flush_duration,
        }
    }

    /// Creates a buffer with default 5-minute flush window.
    pub fn default() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Adds a segment to the buffer and returns true if the buffer should flush.
    pub fn add_segment(&self, text: &str) -> bool {
        let now = SystemTime::now();
        let rfc3339_timestamp = Utc::now().to_rfc3339();

        let segment = TranscriptSegment {
            text: text.to_string(),
            timestamp: now,
            rfc3339_timestamp,
        };

        let mut segments = self.segments.lock().unwrap();
        let mut first_time = self.first_segment_time.lock().unwrap();
        let mut last_time = self.last_segment_time.lock().unwrap();

        // Initialize first segment time if empty.
        if first_time.is_none() {
            *first_time = Some(now);
        }

        segments.push(segment);
        *last_time = Some(now);

        // Check if buffer should flush (5 minutes elapsed).
        if let (Some(start), Some(end)) = (*first_time, *last_time) {
            if let Ok(elapsed) = end.duration_since(start) {
                return elapsed >= self.flush_duration;
            }
        }

        false
    }

    /// Flushes the buffer and returns a combined thought block or None if empty.
    pub fn flush(&self) -> Option<ThoughtBlock> {
        let mut segments = self.segments.lock().unwrap();
        let mut first_time = self.first_segment_time.lock().unwrap();
        let mut last_time = self.last_segment_time.lock().unwrap();

        if segments.is_empty() {
            return None;
        }

        let block = ThoughtBlock {
            id: Uuid::new_v4().to_string(),
            combined_text: segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" "),
            segments: segments.clone(),
            started_at: first_time.unwrap_or_else(SystemTime::now),
            ended_at: last_time.unwrap_or_else(SystemTime::now),
            started_at_rfc3339: Utc::now().to_rfc3339(),
        };

        segments.clear();
        *first_time = None;
        *last_time = None;

        Some(block)
    }

}

/// A complete thought block ready for LLM processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtBlock {
    pub id: String,
    pub combined_text: String,
    pub segments: Vec<TranscriptSegment>,
    pub started_at: SystemTime,
    pub ended_at: SystemTime,
    pub started_at_rfc3339: String,
}


fn tokenize_context(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter_map(|token| {
            let lowered = token.trim().to_lowercase();
            if lowered.len() >= 4 {
                Some(lowered)
            } else {
                None
            }
        })
        .collect()
}

fn subject_overlap(a: &str, b: &str) -> Option<String> {
    let a_tokens = tokenize_context(a);
    let b_tokens = tokenize_context(b);
    a_tokens.intersection(&b_tokens).next().cloned()
}

/// Detect a circular loop by checking the latest three overthinking signals.
/// Returns the common subject token when a loop is likely present.
pub fn detect_recent_mental_loop(conn: &Connection) -> Option<String> {
    let cutoff = (Utc::now() - chrono::Duration::minutes(15)).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT tag, context FROM fog_entries_extended WHERE timestamp > ?1 ORDER BY timestamp DESC LIMIT 3"
    ).ok()?;

    let rows = stmt.query_map([cutoff], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).ok()?;

    let mut recent: Vec<(String, String)> = Vec::new();
    for row in rows.flatten() {
        recent.push(row);
    }

    if recent.len() < 3 {
        return None;
    }

    if recent.iter().any(|(tag, _)| tag.to_lowercase() != "overthinking") {
        return None;
    }

    let s1 = subject_overlap(&recent[0].1, &recent[1].1)?;
    let s2 = subject_overlap(&recent[1].1, &recent[2].1)?;
    if s1 == s2 {
        Some(s1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_creation() {
        let buffer = ThoughtBlockBuffer::default();
        assert!(buffer.flush().is_none());
    }

    #[test]
    fn test_add_segment() {
        let buffer = ThoughtBlockBuffer::default();
        buffer.add_segment("Hello");
        let block = buffer.flush().unwrap();
        assert_eq!(block.segments.len(), 1);
    }

    #[test]
    fn test_flush_empty() {
        let buffer = ThoughtBlockBuffer::default();
        assert!(buffer.flush().is_none());
    }

    #[test]
    fn test_flush_with_segments() {
        let buffer = ThoughtBlockBuffer::default();
        buffer.add_segment("Hello world");
        buffer.add_segment("Testing buffer");

        let block = buffer.flush();
        assert!(block.is_some());

        let b = block.unwrap();
        assert_eq!(b.segments.len(), 2);
        assert!(b.combined_text.contains("Hello world"));
        assert!(b.combined_text.contains("Testing buffer"));
        assert!(buffer.flush().is_none());
    }
}
