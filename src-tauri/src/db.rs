use rusqlite::{Connection, Result, params};
use chrono::{Local, Utc, Timelike};
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FocusSession {
    pub id: String,
    pub title: String,
    pub date: String,        // YYYY-MM-DD
    pub start_time: String,  // HH:MM
    pub end_time: String,    // HH:MM
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StoredSuggestion {
    pub id: String,
    #[serde(rename = "type")]
    pub suggestion_type: String, // "task" | "focus" | "goal"
    pub text: String,
    pub priority: String,        // "high" | "medium" | "low"
    pub status: String,          // "pending" | "accepted" | "dismissed"
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub project: Option<String>,
    pub due_hint: Option<String>,
    pub done: bool,
    pub created_at: String,
    pub source: String, // "ambient" | "command"
    pub status: String, // "official" | "archived"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub created_at: String,
    pub status: String, // "active" | "completed" | "archived"
    pub horizon: String, // "spark" | "milestone" | "north_star"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub person: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
    pub action_items: Option<String>,
    pub transcript: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranscriptChunk {
    pub id: String,
    pub text: String,
    pub language: String,
    pub timestamp: String,
    pub meeting_id: Option<String>,
    pub fog_tags: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FogEntry {
    pub id: String,
    pub tag: String,
    pub context: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphNode {
    pub id: String,
    pub node_type: String, // "person" | "project" | "goal" | "topic" | "fog_pattern" | "task"
    pub label: String,
    pub data: Option<String>, // JSON metadata
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphEdge {
    pub id: String,
    pub from_node: String,
    pub to_node: String,
    pub edge_type: String, // "blocks" | "relates_to" | "works_with" | "mentions" | "triggers"
    pub weight: f64,
    pub last_updated: String,
}

fn get_table_sql(conn: &Connection, table: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

fn migrate_graph_schema_if_needed(conn: &Connection) -> Result<()> {
    let nodes_sql = get_table_sql(conn, "graph_nodes").unwrap_or_default().to_lowercase();
    let edges_sql = get_table_sql(conn, "graph_edges").unwrap_or_default().to_lowercase();

    let nodes_ready = nodes_sql.contains("'task'") || nodes_sql.is_empty();
    let edges_ready = edges_sql.contains("'triggers'") || edges_sql.is_empty();
    if nodes_ready && edges_ready {
        return Ok(());
    }

    conn.execute_batch(
        "
        PRAGMA foreign_keys=OFF;

        ALTER TABLE graph_edges RENAME TO graph_edges_old;
        ALTER TABLE graph_nodes RENAME TO graph_nodes_old;

        CREATE TABLE graph_nodes (
            id TEXT PRIMARY KEY,
            node_type TEXT NOT NULL CHECK(node_type IN ('person','project','goal','topic','fog_pattern','task')),
            label TEXT NOT NULL,
            data TEXT,
            created_at TEXT NOT NULL
        );

        CREATE TABLE graph_edges (
            id TEXT PRIMARY KEY,
            from_node TEXT NOT NULL,
            to_node TEXT NOT NULL,
            edge_type TEXT NOT NULL CHECK(edge_type IN ('blocks','relates_to','works_with','mentions','triggers')),
            weight REAL NOT NULL DEFAULT 1.0,
            last_updated TEXT NOT NULL,
            FOREIGN KEY(from_node) REFERENCES graph_nodes(id),
            FOREIGN KEY(to_node) REFERENCES graph_nodes(id)
        );

        INSERT INTO graph_nodes (id,node_type,label,data,created_at)
        SELECT id,node_type,label,data,created_at FROM graph_nodes_old;

        INSERT INTO graph_edges (id,from_node,to_node,edge_type,weight,last_updated)
        SELECT id,from_node,to_node,edge_type,weight,last_updated FROM graph_edges_old;

        DROP TABLE graph_edges_old;
        DROP TABLE graph_nodes_old;

        PRAGMA foreign_keys=ON;
        ",
    )?;

    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FogEntryExtended {
    pub id: String,
    pub chunk_id: String,
    pub tag: String,
    pub context: String,
    pub resolution: String,
    pub strength: f64, // 0.0 - 1.0
    pub markers: String, // JSON array of markers
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LiveNote {
    pub id: String,
    pub day_key: String,
    pub window_started_at: String,
    pub window_ended_at: String,
    pub summary: String,
    pub highlights: Vec<String>,
    pub ideas: Vec<String>,
    pub tasks: Vec<String>,
    pub fingerprint: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoricalNote {
    pub id: String,
    pub day_key: String,
    pub summary: String,
    pub highlights: Vec<String>,
    pub ideas: Vec<String>,
    pub tasks: Vec<String>,
    pub note_count: i64,
    pub created_at: String,
}

pub fn open_db() -> Result<Connection> {
    let home = std::env::var("HOME").unwrap_or(".".to_string());
    let db_path = format!("{}/Library/Application Support/com.nokast.nodemind/nodemind.db", home);
    // Ensure directory exists
    let dir = std::path::Path::new(&db_path).parent().unwrap();
    std::fs::create_dir_all(dir).ok();
    Connection::open(&db_path)
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;

        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            project TEXT,
            due_hint TEXT,
            done INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'ambient',
            status TEXT NOT NULL DEFAULT 'official' CHECK(status IN ('official','archived'))
        );

        CREATE TABLE IF NOT EXISTS goals (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            description TEXT,
            created_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','completed','archived')),
            horizon TEXT NOT NULL DEFAULT 'milestone' CHECK(horizon IN ('spark','milestone','north_star'))
        );

        CREATE TABLE IF NOT EXISTS meetings (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            person TEXT,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            summary TEXT,
            action_items TEXT,
            transcript TEXT
        );

        CREATE TABLE IF NOT EXISTS transcript_chunks (
            id TEXT PRIMARY KEY,
            text TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT 'en',
            timestamp TEXT NOT NULL,
            meeting_id TEXT,
            fog_tags TEXT
        );

        CREATE TABLE IF NOT EXISTS fog_entries (
            id TEXT PRIMARY KEY,
            tag TEXT NOT NULL,
            context TEXT NOT NULL,
            timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS fog_entries_extended (
            id TEXT PRIMARY KEY,
            chunk_id TEXT NOT NULL,
            tag TEXT NOT NULL DEFAULT '',
            context TEXT NOT NULL DEFAULT '',
            resolution TEXT NOT NULL DEFAULT '',
            strength REAL NOT NULL DEFAULT 0.0,
            markers TEXT NOT NULL DEFAULT '[]',
            timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS graph_nodes (
            id TEXT PRIMARY KEY,
            node_type TEXT NOT NULL CHECK(node_type IN ('person','project','goal','topic','fog_pattern','task')),
            label TEXT NOT NULL,
            data TEXT,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS graph_edges (
            id TEXT PRIMARY KEY,
            from_node TEXT NOT NULL,
            to_node TEXT NOT NULL,
            edge_type TEXT NOT NULL CHECK(edge_type IN ('blocks','relates_to','works_with','mentions','triggers')),
            weight REAL NOT NULL DEFAULT 1.0,
            last_updated TEXT NOT NULL,
            FOREIGN KEY(from_node) REFERENCES graph_nodes(id),
            FOREIGN KEY(to_node) REFERENCES graph_nodes(id)
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS focus_sessions (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            date TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT NOT NULL,
            notes TEXT,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS suggestions (
            id TEXT PRIMARY KEY,
            suggestion_type TEXT NOT NULL,
            text TEXT NOT NULL,
            priority TEXT NOT NULL DEFAULT 'medium',
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS live_notes (
            id TEXT PRIMARY KEY,
            day_key TEXT NOT NULL,
            window_started_at TEXT NOT NULL,
            window_ended_at TEXT NOT NULL,
            summary TEXT NOT NULL,
            highlights TEXT NOT NULL DEFAULT '[]',
            ideas TEXT NOT NULL DEFAULT '[]',
            tasks TEXT NOT NULL DEFAULT '[]',
            fingerprint TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(day_key, fingerprint)
        );

        CREATE TABLE IF NOT EXISTS historical_notes (
            id TEXT PRIMARY KEY,
            day_key TEXT NOT NULL UNIQUE,
            summary TEXT NOT NULL,
            highlights TEXT NOT NULL DEFAULT '[]',
            ideas TEXT NOT NULL DEFAULT '[]',
            tasks TEXT NOT NULL DEFAULT '[]',
            note_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS extractor_debug_runs (
            id TEXT PRIMARY KEY,
            timestamp TEXT NOT NULL,
            word_count INTEGER NOT NULL,
            reason TEXT NOT NULL,
            raw_json TEXT NOT NULL
        );
    ")?;

    // Backward-compatible migration for existing local DBs.
    let _ = conn.execute("ALTER TABLE fog_entries_extended ADD COLUMN tag TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE fog_entries_extended ADD COLUMN context TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE fog_entries_extended ADD COLUMN resolution TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE goals ADD COLUMN horizon TEXT NOT NULL DEFAULT 'milestone'", []);
    // Track which live notes have been processed by the batch analysis job.
    let _ = conn.execute("ALTER TABLE live_notes ADD COLUMN processed_at TEXT", []);
    let _ = migrate_graph_schema_if_needed(conn);

    Ok(())
}

fn parse_json_string_list(raw: String) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
}

fn to_json_string_list(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

fn dedupe_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_lowercase()))
        .collect()
}

pub fn local_today_key() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn fingerprint_live_note(summary: &str, highlights: &[String], ideas: &[String], tasks: &[String]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    summary.trim().to_lowercase().hash(&mut hasher);
    for item in highlights {
        item.trim().to_lowercase().hash(&mut hasher);
    }
    for item in ideas {
        item.trim().to_lowercase().hash(&mut hasher);
    }
    for item in tasks {
        item.trim().to_lowercase().hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

// ── Settings ──────────────────────────────────────────────────────────────────

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ).ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

pub fn insert_task(conn: &Connection, task: &Task) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO tasks (id,title,project,due_hint,done,created_at,source,status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![task.id, task.title, task.project, task.due_hint, task.done as i32, task.created_at, task.source, task.status],
    )?;
    Ok(())
}

pub fn get_tasks(conn: &Connection) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare(
        "SELECT id,title,project,due_hint,done,created_at,source,status FROM tasks ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        let raw_status: String = row.get(7)?;
        let status = if raw_status == "shadow" {
            "official".to_string()
        } else {
            raw_status
        };
        Ok(Task {
            id: row.get(0)?,
            title: row.get(1)?,
            project: row.get(2)?,
            due_hint: row.get(3)?,
            done: row.get::<_, i32>(4)? != 0,
            created_at: row.get(5)?,
            source: row.get(6)?,
            status,
        })
    })?;
    rows.collect()
}

pub fn mark_task_done(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("UPDATE tasks SET done=1 WHERE id=?1", params![id])?;
    Ok(())
}

// ── Goals ─────────────────────────────────────────────────────────────────────

pub fn insert_goal(conn: &Connection, goal: &Goal) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO goals (id,title,description,created_at,status,horizon) VALUES (?1,?2,?3,?4,?5,?6)",
        params![goal.id, goal.title, goal.description, goal.created_at, goal.status, goal.horizon],
    )?;
    Ok(())
}

pub fn get_goals(conn: &Connection) -> Result<Vec<Goal>> {
    let mut stmt = conn.prepare(
        "SELECT id,title,description,created_at,status,COALESCE(horizon,'milestone') FROM goals ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        let raw_horizon: String = row.get(5)?;
        let horizon = match raw_horizon.as_str() {
            "spark" | "milestone" | "north_star" => raw_horizon,
            _ => "milestone".to_string(),
        };
        Ok(Goal {
            id: row.get(0)?,
            title: row.get(1)?,
            description: row.get(2)?,
            created_at: row.get(3)?,
            status: row.get(4)?,
            horizon,
        })
    })?;
    rows.collect()
}

// ── Meetings ──────────────────────────────────────────────────────────────────

pub fn upsert_meeting(conn: &Connection, m: &Meeting) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meetings (id,title,person,started_at,ended_at,summary,action_items,transcript) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![m.id, m.title, m.person, m.started_at, m.ended_at, m.summary, m.action_items, m.transcript],
    )?;
    Ok(())
}

pub fn get_meetings(conn: &Connection) -> Result<Vec<Meeting>> {
    let mut stmt = conn.prepare(
        "SELECT id,title,person,started_at,ended_at,summary,action_items,transcript FROM meetings ORDER BY started_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Meeting {
            id: row.get(0)?,
            title: row.get(1)?,
            person: row.get(2)?,
            started_at: row.get(3)?,
            ended_at: row.get(4)?,
            summary: row.get(5)?,
            action_items: row.get(6)?,
            transcript: row.get(7)?,
        })
    })?;
    rows.collect()
}

// ── Transcript chunks ─────────────────────────────────────────────────────────

pub fn insert_chunk(conn: &Connection, chunk: &TranscriptChunk) -> Result<()> {
    conn.execute(
        "INSERT INTO transcript_chunks (id,text,language,timestamp,meeting_id,fog_tags) VALUES (?1,?2,?3,?4,?5,?6)",
        params![chunk.id, chunk.text, chunk.language, chunk.timestamp, chunk.meeting_id, chunk.fog_tags],
    )?;
    Ok(())
}

pub fn get_recent_chunks(conn: &Connection, minutes: i64) -> Result<Vec<TranscriptChunk>> {
    let cutoff = (Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id,text,language,timestamp,meeting_id,fog_tags FROM transcript_chunks WHERE timestamp > ?1 ORDER BY timestamp ASC"
    )?;
    let rows = stmt.query_map(params![cutoff], |row| {
        Ok(TranscriptChunk {
            id: row.get(0)?,
            text: row.get(1)?,
            language: row.get(2)?,
            timestamp: row.get(3)?,
            meeting_id: row.get(4)?,
            fog_tags: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Returns all transcript chunks recorded since the start of today (local midnight UTC).
pub fn get_today_chunks(conn: &Connection) -> Result<Vec<TranscriptChunk>> {
    let now = Utc::now();
    let midnight = (now - chrono::Duration::seconds(
        (now.num_seconds_from_midnight()) as i64,
    )).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id,text,language,timestamp,meeting_id,fog_tags FROM transcript_chunks WHERE timestamp >= ?1 ORDER BY timestamp ASC"
    )?;
    let rows = stmt.query_map(params![midnight], |row| {
        Ok(TranscriptChunk {
            id: row.get(0)?,
            text: row.get(1)?,
            language: row.get(2)?,
            timestamp: row.get(3)?,
            meeting_id: row.get(4)?,
            fog_tags: row.get(5)?,
        })
    })?;
    rows.collect()
}

// ── Fog entries ───────────────────────────────────────────────────────────────

pub fn insert_fog(conn: &Connection, entry: &FogEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO fog_entries (id,tag,context,timestamp) VALUES (?1,?2,?3,?4)",
        params![entry.id, entry.tag, entry.context, entry.timestamp],
    )?;
    Ok(())
}

pub fn get_fog_stats(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT tag, COUNT(*) as cnt FROM fog_entries GROUP BY tag ORDER BY cnt DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.collect()
}

// ── Focus sessions ────────────────────────────────────────────────────────────

pub fn insert_focus_session(conn: &Connection, s: &FocusSession) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO focus_sessions (id,title,date,start_time,end_time,notes,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![s.id, s.title, s.date, s.start_time, s.end_time, s.notes, s.created_at],
    )?;
    Ok(())
}

pub fn get_focus_sessions_in_range(conn: &Connection, start_date: &str, end_date: &str) -> Result<Vec<FocusSession>> {
    let mut stmt = conn.prepare(
        "SELECT id,title,date,start_time,end_time,notes,created_at FROM focus_sessions WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC, start_time ASC"
    )?;
    let rows = stmt.query_map(params![start_date, end_date], |row| {
        Ok(FocusSession {
            id: row.get(0)?,
            title: row.get(1)?,
            date: row.get(2)?,
            start_time: row.get(3)?,
            end_time: row.get(4)?,
            notes: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn delete_focus_session(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM focus_sessions WHERE id = ?1", params![id])?;
    Ok(())
}

// ── Suggestions ───────────────────────────────────────────────────────────────

pub fn insert_suggestion(conn: &Connection, s: &StoredSuggestion) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO suggestions (id,suggestion_type,text,priority,status,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![s.id, s.suggestion_type, s.text, s.priority, s.status, s.created_at],
    )?;
    Ok(())
}

/// Mark all currently pending suggestions as dismissed (so a fresh batch can replace them).
pub fn dismiss_pending_suggestions(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE suggestions SET status='dismissed' WHERE status='pending'", [])?;
    Ok(())
}

pub fn get_suggestions(conn: &Connection) -> Result<Vec<StoredSuggestion>> {
    let mut stmt = conn.prepare(
        "SELECT id,suggestion_type,text,priority,status,created_at FROM suggestions WHERE status != 'dismissed' ORDER BY created_at DESC LIMIT 30"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(StoredSuggestion {
            id: row.get(0)?,
            suggestion_type: row.get(1)?,
            text: row.get(2)?,
            priority: row.get(3)?,
            status: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

// ── Live Notes ───────────────────────────────────────────────────────────────

pub fn insert_live_note(conn: &Connection, note: &LiveNote) -> Result<bool> {
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO live_notes (id,day_key,window_started_at,window_ended_at,summary,highlights,ideas,tasks,fingerprint,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            note.id,
            note.day_key,
            note.window_started_at,
            note.window_ended_at,
            note.summary,
            to_json_string_list(&note.highlights),
            to_json_string_list(&note.ideas),
            to_json_string_list(&note.tasks),
            note.fingerprint,
            note.created_at,
        ],
    )?;
    Ok(inserted > 0)
}

pub fn get_live_notes_for_day(conn: &Connection, day_key: &str) -> Result<Vec<LiveNote>> {
    let mut stmt = conn.prepare(
        "SELECT id,day_key,window_started_at,window_ended_at,summary,highlights,ideas,tasks,fingerprint,created_at FROM live_notes WHERE day_key = ?1 ORDER BY window_started_at DESC, created_at DESC"
    )?;
    let rows = stmt.query_map(params![day_key], |row| {
        Ok(LiveNote {
            id: row.get(0)?,
            day_key: row.get(1)?,
            window_started_at: row.get(2)?,
            window_ended_at: row.get(3)?,
            summary: row.get(4)?,
            highlights: parse_json_string_list(row.get(5)?),
            ideas: parse_json_string_list(row.get(6)?),
            tasks: parse_json_string_list(row.get(7)?),
            fingerprint: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn delete_live_notes_for_day(conn: &Connection, day_key: &str) -> Result<()> {
    conn.execute("DELETE FROM live_notes WHERE day_key = ?1", params![day_key])?;
    Ok(())
}

pub fn archive_previous_live_notes(conn: &Connection) -> Result<usize> {
    let today = local_today_key();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT day_key FROM live_notes WHERE day_key < ?1 ORDER BY day_key ASC"
    )?;
    let day_rows = stmt.query_map(params![today], |row| row.get::<_, String>(0))?;
    let mut archived = 0usize;
    for day_key in day_rows.flatten() {
        if rollup_live_notes_for_day(conn, &day_key)? {
            archived += 1;
        }
    }
    Ok(archived)
}

fn build_historical_summary(notes: &[LiveNote]) -> String {
    let summaries = dedupe_strings(notes.iter().map(|note| note.summary.clone()));
    if summaries.is_empty() {
        return "No summary captured for this day.".to_string();
    }
    summaries.join(" ")
}

pub fn rollup_live_notes_for_day(conn: &Connection, day_key: &str) -> Result<bool> {
    let notes = get_live_notes_for_day(conn, day_key)?;
    if notes.is_empty() {
        return Ok(false);
    }

    let highlights = dedupe_strings(notes.iter().flat_map(|note| note.highlights.clone()));
    let ideas = dedupe_strings(notes.iter().flat_map(|note| note.ideas.clone()));
    let tasks = dedupe_strings(notes.iter().flat_map(|note| note.tasks.clone()));
    let historical = HistoricalNote {
        id: format!("history-{}", day_key),
        day_key: day_key.to_string(),
        summary: build_historical_summary(&notes),
        highlights,
        ideas,
        tasks,
        note_count: notes.len() as i64,
        created_at: Utc::now().to_rfc3339(),
    };

    upsert_historical_note(conn, &historical)?;
    delete_live_notes_for_day(conn, day_key)?;
    Ok(true)
}

// ── Historical Notes ─────────────────────────────────────────────────────────

pub fn upsert_historical_note(conn: &Connection, note: &HistoricalNote) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO historical_notes (id,day_key,summary,highlights,ideas,tasks,note_count,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            note.id,
            note.day_key,
            note.summary,
            to_json_string_list(&note.highlights),
            to_json_string_list(&note.ideas),
            to_json_string_list(&note.tasks),
            note.note_count,
            note.created_at,
        ],
    )?;
    Ok(())
}

pub fn get_historical_notes(conn: &Connection) -> Result<Vec<HistoricalNote>> {
    let mut stmt = conn.prepare(
        "SELECT id,day_key,summary,highlights,ideas,tasks,note_count,created_at FROM historical_notes ORDER BY day_key DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(HistoricalNote {
            id: row.get(0)?,
            day_key: row.get(1)?,
            summary: row.get(2)?,
            highlights: parse_json_string_list(row.get(3)?),
            ideas: parse_json_string_list(row.get(4)?),
            tasks: parse_json_string_list(row.get(5)?),
            note_count: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect()
}

pub fn update_suggestion_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    conn.execute("UPDATE suggestions SET status=?1 WHERE id=?2", params![status, id])?;
    Ok(())
}

pub fn insert_extractor_debug_run(
    conn: &Connection,
    id: &str,
    timestamp: &str,
    word_count: i64,
    reason: &str,
    raw_json: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO extractor_debug_runs (id,timestamp,word_count,reason,raw_json) VALUES (?1,?2,?3,?4,?5)",
        params![id, timestamp, word_count, reason, raw_json],
    )?;
    Ok(())
}

// ── Graph Nodes & Edges ───────────────────────────────────────────────────────

pub fn insert_node(conn: &Connection, node: &GraphNode) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO graph_nodes (id,node_type,label,data,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![node.id, node.node_type, node.label, node.data, node.created_at],
    )?;
    Ok(())
}

pub fn get_nodes(conn: &Connection) -> Result<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        "SELECT id,node_type,label,data,created_at FROM graph_nodes ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GraphNode {
            id: row.get(0)?,
            node_type: row.get(1)?,
            label: row.get(2)?,
            data: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn insert_edge(conn: &Connection, edge: &GraphEdge) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO graph_edges (id,from_node,to_node,edge_type,weight,last_updated) VALUES (?1,?2,?3,?4,?5,?6)",
        params![edge.id, edge.from_node, edge.to_node, edge.edge_type, edge.weight, edge.last_updated],
    )?;
    Ok(())
}

pub fn get_edges(conn: &Connection) -> Result<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        "SELECT id,from_node,to_node,edge_type,weight,last_updated FROM graph_edges ORDER BY last_updated DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GraphEdge {
            id: row.get(0)?,
            from_node: row.get(1)?,
            to_node: row.get(2)?,
            edge_type: row.get(3)?,
            weight: row.get(4)?,
            last_updated: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn decay_edge_weights(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE graph_edges SET weight = weight * 0.9", [])?;
    Ok(())
}

/// Increment (or create) a numeric field inside a graph node's data JSON.
/// Used for `mentions_count` (branch fattening) and the leaf-merge `+N` badge.
pub fn increment_node_field(conn: &Connection, node_id: &str, field: &str, delta: i64) -> Result<()> {
    let existing: Option<String> = conn.query_row(
        "SELECT data FROM graph_nodes WHERE id = ?1",
        params![node_id],
        |row| row.get(0),
    ).ok().flatten();

    let mut map: serde_json::Map<String, serde_json::Value> = existing
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let current = map.get(field).and_then(|v| v.as_i64()).unwrap_or(0);
    let new_val = serde_json::Value::Number(serde_json::Number::from(current + delta));
    map.insert(field.to_string(), new_val);

    conn.execute(
        "UPDATE graph_nodes SET data = ?2 WHERE id = ?1",
        params![node_id, serde_json::Value::Object(map).to_string()],
    )?;
    Ok(())
}

// ── Extended Fog Entries ──────────────────────────────────────────────────────

pub fn insert_fog_extended(conn: &Connection, entry: &FogEntryExtended) -> Result<()> {
    conn.execute(
        "INSERT INTO fog_entries_extended (id,chunk_id,tag,context,resolution,strength,markers,timestamp) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![entry.id, entry.chunk_id, entry.tag, entry.context, entry.resolution, entry.strength, entry.markers, entry.timestamp],
    )?;
    Ok(())
}

pub fn get_fog_entries_extended(conn: &Connection, hours: i64) -> Result<Vec<FogEntryExtended>> {
    let cutoff = (Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id,chunk_id,tag,context,resolution,strength,markers,timestamp FROM fog_entries_extended WHERE timestamp > ?1 ORDER BY timestamp DESC"
    )?;
    let rows = stmt.query_map(params![cutoff], |row| {
        Ok(FogEntryExtended {
            id: row.get(0)?,
            chunk_id: row.get(1)?,
            tag: row.get(2)?,
            context: row.get(3)?,
            resolution: row.get(4)?,
            strength: row.get(5)?,
            markers: row.get(6)?,
            timestamp: row.get(7)?,
        })
    })?;
    rows.collect()
}

// ── Context Anchoring ─────────────────────────────────────────────────────────

/// Get the last 3 active topics from the mental map nodes.
/// This is used as context for LLM prompts to ground ambiguous references.
pub fn get_last_active_topics(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT label FROM graph_nodes WHERE node_type='topic' ORDER BY created_at DESC LIMIT 3"
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

// ── Batch analysis tracking ────────────────────────────────────────────────────

/// Return today's live notes that have never been processed by the batch analysis job.
pub fn get_unprocessed_notes_for_today(conn: &Connection) -> Result<Vec<LiveNote>> {
    let today = local_today_key();
    let mut stmt = conn.prepare(
        "SELECT id,day_key,window_started_at,window_ended_at,summary,highlights,ideas,tasks,fingerprint,created_at \
         FROM live_notes WHERE day_key = ?1 AND processed_at IS NULL ORDER BY window_started_at ASC"
    )?;
    let rows = stmt.query_map(params![today], |row| {
        Ok(LiveNote {
            id: row.get(0)?,
            day_key: row.get(1)?,
            window_started_at: row.get(2)?,
            window_ended_at: row.get(3)?,
            summary: row.get(4)?,
            highlights: parse_json_string_list(row.get(5)?),
            ideas: parse_json_string_list(row.get(6)?),
            tasks: parse_json_string_list(row.get(7)?),
            fingerprint: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    rows.collect()
}

/// Return how many live notes for today are still pending batch analysis.
pub fn count_unprocessed_notes_for_today(conn: &Connection) -> Result<i64> {
    let today = local_today_key();
    conn.query_row(
        "SELECT COUNT(*) FROM live_notes WHERE day_key = ?1 AND processed_at IS NULL",
        params![today],
        |row| row.get(0),
    )
}

/// Stamp a batch of live notes as processed so the batch job skips them next time.
pub fn mark_notes_processed(conn: &Connection, ids: &[String]) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    for id in ids {
        conn.execute(
            "UPDATE live_notes SET processed_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
    }
    Ok(())
}

/// Return the text of all pending suggestions created today (for deduplication).
pub fn get_pending_suggestion_texts(conn: &Connection) -> Result<Vec<String>> {
    // today midnight in UTC as a rough filter
    let now = Utc::now();
    let midnight = (now - chrono::Duration::seconds(
        now.num_seconds_from_midnight() as i64,
    )).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT text FROM suggestions WHERE status = 'pending' AND created_at >= ?1"
    )?;
    let rows = stmt.query_map(params![midnight], |row| row.get::<_, String>(0))?;
    rows.collect()
}
