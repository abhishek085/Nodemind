//! Schema + CRUD. One bundled SQLite file keeps the whole app self-contained.
//!
//! Schema is created idempotently on open (`init_schema`); columns that older
//! builds may lack are added on the fly, and a one-time import (`migrate_legacy`)
//! carries pre-v2 data (thoughts/notes, graph, tasks, settings) into the
//! current tables. The import is gated by `meta.schema_version` and leaves the
//! legacy tables in place (harmless, inspectable via sqlite).

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use serde::{Deserialize, Serialize};

use crate::error::Result;


// ── Domain types ────────────────────────────────────────────────────────────

/// A captured brain-dump: the raw transcript plus what the LLM made of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thought {
    pub id: String,
    pub text: String,
    pub summary: Option<String>,
    pub language: String,
    pub day: String, // YYYY-MM-DD (local)
    pub started_at: String, // RFC3339
    pub duration_secs: i64,
    pub created_at: String,
    /// "note" (a brain-dump) or "meeting" (labelled You/Them transcript).
    pub kind: String,
    /// Meeting minutes as JSON (`engine::meeting::Minutes`), meetings only.
    pub minutes: Option<String>,
    /// Fog signals as JSON (`engine::fog::FogSignal[]`), when any were found.
    pub fog: Option<String>,
    /// Nothing worth keeping was said (small talk, a test): hidden from Today and the map.
    pub noise: bool,
}

/// A concrete to-do extracted from a thought (or added by hand).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub project: Option<String>,
    pub thought_id: Option<String>,
    pub done: bool,
    pub created_at: String,
}

/// A node in the thought graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    /// "thought" | "task" | "topic" | "person" | "goal"
    pub node_type: String,
    pub label: String,
    pub mentions: i64,
    pub last_seen: Option<String>,
    pub sentiment: String, // positive | neutral | negative
}

/// An edge in the thought graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub from_node: String,
    pub to_node: String,
    /// "relates_to" | "part_of" | "triggers" | "involves" | "blocks" | "derived_from"
    pub edge_type: String,
    pub weight: f64,
}

/// A calendar event synced from Apple Calendar (cached per day).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: String, // RFC3339 local
    pub end: String,
    pub calendar: String,
    pub day: String,
}

// ── Lifecycle ───────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS thoughts (
  id            TEXT PRIMARY KEY,
  text          TEXT NOT NULL,
  summary       TEXT,
  language      TEXT NOT NULL DEFAULT 'en',
  day           TEXT NOT NULL,
  started_at    TEXT NOT NULL,
  duration_secs INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  kind          TEXT NOT NULL DEFAULT 'note',
  minutes       TEXT,
  fog           TEXT,
  noise         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_thoughts_day ON thoughts(day);
CREATE INDEX IF NOT EXISTS idx_thoughts_created ON thoughts(created_at);

CREATE TABLE IF NOT EXISTS tasks (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  project    TEXT,
  thought_id TEXT,
  done       INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_done ON tasks(done);
CREATE INDEX IF NOT EXISTS idx_tasks_thought ON tasks(thought_id);

CREATE TABLE IF NOT EXISTS graph_nodes (
  id         TEXT PRIMARY KEY,
  node_type  TEXT NOT NULL,
  label      TEXT NOT NULL,
  mentions   INTEGER NOT NULL DEFAULT 1,
  last_seen  TEXT,
  sentiment  TEXT NOT NULL DEFAULT 'neutral'
);
CREATE INDEX IF NOT EXISTS idx_nodes_type ON graph_nodes(node_type);

CREATE TABLE IF NOT EXISTS graph_edges (
  id         TEXT PRIMARY KEY,
  from_node  TEXT NOT NULL,
  to_node    TEXT NOT NULL,
  edge_type  TEXT NOT NULL,
  weight     REAL NOT NULL DEFAULT 1.0,
  UNIQUE(from_node, to_node, edge_type)
);

CREATE TABLE IF NOT EXISTS embeddings (
  id       TEXT PRIMARY KEY,
  ref_kind TEXT NOT NULL,      -- 'thought' | 'topic' | 'person' | 'goal' | 'task' | 'label'
  ref_id   TEXT NOT NULL,      -- the entity row id / slug the vector belongs to
  dim      INTEGER NOT NULL,
  model    TEXT NOT NULL,      -- embedding model key, so mixes never compare
  vec      BLOB NOT NULL       -- little-endian f32
);
CREATE INDEX IF NOT EXISTS idx_embeddings_ref ON embeddings(ref_kind, ref_id);

CREATE TABLE IF NOT EXISTS items (
  id         TEXT PRIMARY KEY,
  thought_id TEXT NOT NULL,
  kind       TEXT NOT NULL,
  text       TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_items_thought ON items(thought_id);

CREATE TABLE IF NOT EXISTS calendar_events (
  id       TEXT PRIMARY KEY,
  title    TEXT NOT NULL,
  start    TEXT NOT NULL,
  end      TEXT NOT NULL,
  calendar TEXT NOT NULL,
  day      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_calendar_day ON calendar_events(day);

CREATE TABLE IF NOT EXISTS focus_sessions (
  id         TEXT PRIMARY KEY,
  title      TEXT,
  day        TEXT NOT NULL,
  started_at TEXT NOT NULL,
  ended_at   TEXT,
  minutes    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

/// Schema version stamped into `meta` once the legacy import has run.
const SCHEMA_VERSION: &str = "2";

/// Open (and initialise) the database at `data_dir/nodemind.db`.
pub fn open_database(data_dir: &Path) -> Result<Connection> {
    let path = data_dir.join("nodemind.db");
    let mut conn = Connection::open(&path).map_err(|e| crate::error::Error::Store(e.to_string()))?;
    init_schema(&mut conn)?;
    Ok(conn)
}

/// Idempotent schema creation, plus upgrades for databases made by older builds.
///
/// `normalize_shapes` runs *before* the batch: on a legacy file the batch's
/// `CREATE INDEX ... ON tasks(thought_id)` would fail on the pre-upgrade
/// table, while the normalizer is a no-op on a fresh database (nothing exists
/// yet to rebuild). Afterwards the batch creates whatever is missing —
/// including the named indexes on the rebuilt tables.
pub fn init_schema(conn: &mut Connection) -> Result<()> {
    // Superseded by `items` (search now indexes statements, not passages).
    let _ = conn.execute_batch("DROP TABLE IF EXISTS passages;");
    normalize_shapes(conn)?;
    conn.execute_batch(SCHEMA).map_err(|e| crate::error::Error::Store(e.to_string()))?;
    migrate_legacy(conn)?;
    Ok(())
}

// ── Schema upgrades ─────────────────────────────────────────────────────────

/// Tables whose shape older builds may have drifted on. (embeddings,
/// calendar_events and meta are v2-only and always come out canonical.)
const UPGRADEABLE: &[&str] = &[
    "thoughts",
    "tasks",
    "graph_nodes",
    "graph_edges",
    "focus_sessions",
];

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// The columns `table` currently has (empty when the table is absent).
fn existing_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    if !table_exists(conn, table) {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(rows
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| crate::error::Error::Store(e.to_string()))?)
}

/// Pull `CREATE TABLE {table} (...);` out of SCHEMA, retargeted at `as_name`.
fn create_stmt_for(table: &str, as_name: &str) -> Result<String> {
    let marker = format!("CREATE TABLE IF NOT EXISTS {table} (");
    let start = SCHEMA
        .find(&marker)
        .ok_or_else(|| crate::error::Error::Store(format!("no DDL for {table}")))?;
    // Walk to the matching closing paren (no nested parens appear in the DDL
    // outside the UNIQUE constraint, which is balanced on its own line).
    let mut depth = 0;
    let mut end = None;
    for (i, b) in SCHEMA[start..].bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end
        .ok_or_else(|| crate::error::Error::Store(format!("unbalanced DDL for {table}")))?;
    Ok(format!(
        "{};",
        &SCHEMA[start..end].replace(&format!("IF NOT EXISTS {table}"), as_name)
    ))
}

/// Column names declared by a `CREATE TABLE` statement (constraint lines like
/// `UNIQUE(...)` — and the bare `(` line — are skipped).
fn columns_of_create(create_sql: &str) -> Vec<String> {
    let open = create_sql.find('(').unwrap_or(0);
    let close = create_sql.rfind(')').unwrap_or(create_sql.len());
    create_sql[open..close]
        .lines()
        .map(|l| l.trim())
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("UNIQUE")
                && !l.starts_with("PRIMARY")
                && !l.starts_with("CONSTRAINT")
        })
        .map(|l| l.split_whitespace().next().unwrap_or("").trim_end_matches(',').to_string())
        // A real column name starts with an identifier character; this drops
        // the stray "(" left on the opening line.
        .filter(|n| {
            n.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect()
}

/// Legacy column names that hold the same data as a newer canonical column,
/// keyed by table. Used when rebuilding drifted tables so a row's values move
/// across the rename instead of being zeroed (v1's `date`/`start_time`/
/// `end_time` are the same data as `day`/`started_at`/`ended_at`).
const LEGACY_ALIASES: &[(&str, &str, &str)] = &[
    ("focus_sessions", "day", "date"),
    ("focus_sessions", "started_at", "start_time"),
    ("focus_sessions", "ended_at", "end_time"),
    ("graph_nodes", "mentions", "mentions_count"),
    ("graph_nodes", "last_seen", "last_mentioned_at"),
];

/// The DDL line declaring `col` inside a `CREATE TABLE` statement (or "").
fn ddl_line<'a>(create_sql: &'a str, col: &str) -> &'a str {
    let open = create_sql.find('(').unwrap_or(0);
    let close = create_sql.rfind(')').unwrap_or(create_sql.len());
    create_sql[open..close]
        .lines()
        .find(|l| l.split_whitespace().next() == Some(col))
        .unwrap_or("")
}

/// SQL literal for a canonical column that has no source column at all.
///
/// An explicit NULL still violates NOT NULL (DEFAULTs only apply to *omitted*
/// columns), so for NOT-NULL columns we reuse the declared DEFAULT when the
/// DDL has one, else a type-safe non-NULL literal.
fn fallback_literal(ddl: &str) -> String {
    let upper = ddl.to_uppercase();
    if upper.contains("NOT NULL") {
        if let Some(pos) = upper.find("DEFAULT") {
            let value = ddl[pos + "DEFAULT".len()..]
                .trim()
                .trim_end_matches(',')
                .trim()
                .trim_matches('\'')
                .to_string();
            if !value.is_empty() {
                return format!("'{value}'"); // type affinity coerces numerics
            }
        }
        if upper.contains("INTEGER") || upper.contains("REAL") {
            return "0".into();
        }
        return "''".into();
    }
    "NULL".into()
}

/// Rebuild any `UPGRADEABLE` table whose column set drifted from the current
/// schema.
///
/// Older builds left extra columns behind — including NOT NULL ones (v1's
/// `focus_sessions.date`/`created_at`, `graph_nodes.created_at`) that would
/// break every new insert, since the app no longer writes them. The rebuild
/// copies each canonical column from itself, its legacy alias (when the
/// alias's column exists), or a safe literal — so live data survives and no
/// constraint is violated; only the legacy-only columns go. (The SCHEMA batch
/// runs afterwards in `init_schema`, which restores the named indexes that
/// died with the old tables.)
fn normalize_shapes(conn: &mut Connection) -> Result<()> {
    for table in UPGRADEABLE {
        if !table_exists(conn, table) {
            continue; // created fresh by the batch, already canonical
        }
        let probe = create_stmt_for(table, "_probe")?;
        let canonical = columns_of_create(&probe);
        let existing = existing_columns(conn, table)?;
        let same: std::collections::BTreeSet<&str> =
            canonical.iter().map(String::as_str).collect();
        let have: std::collections::BTreeSet<&str> =
            existing.iter().map(String::as_str).collect();
        if same == have {
            continue;
        }

        let temp = format!("_new_{table}");
        let create = create_stmt_for(table, &temp)?;
        let sources: Vec<String> = canonical
            .iter()
            .map(|col| {
                if have.iter().any(|h| *h == col.as_str()) {
                    return col.clone();
                }
                // `e.0`/`e.1`/`e.2` on the referenced tuple yield plain &strs.
                let aliased = LEGACY_ALIASES
                    .iter()
                    .filter(|e| e.0 == *table && e.1 == col.as_str())
                    .map(|e| e.2)
                    .find(|legacy| have.iter().any(|h| *h == *legacy));
                aliased
                    .map(String::from)
                    .unwrap_or_else(|| fallback_literal(ddl_line(&probe, col)).to_string())
            })
            .collect();
        let cols_csv = canonical.join(", ");
        let srcs_csv = sources.join(", ");
        let copy = format!("INSERT INTO {temp} ({cols_csv}) SELECT {srcs_csv} FROM {table}");
        conn.execute_batch(&format!(
            "{create};\n{copy};\nDROP TABLE {table};\nALTER TABLE {temp} RENAME TO {table};"
        ))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    }
    Ok(())
}

/// One-time import of pre-v2 data into the current tables (see module docs).
///
/// Carried over: `graph_nodes_old`/`graph_edges_old` (the v1 graph),
/// `historical_notes` (→ thoughts + their task lists), task-typed
/// `suggestions` (→ tasks), and `settings` (→ meta). Raw `transcript_chunks`
/// are intentionally left behind — they are pre-distillation fragments, and
/// the distilled notes are what the new UI shows.
fn migrate_legacy(conn: &mut Connection) -> Result<()> {
    if get_meta(conn, "schema_version")?.as_deref() == Some(SCHEMA_VERSION) {
        return Ok(());
    }

    let tx = conn
        .transaction()
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;

    // v1 graph (it stored the live graph in `*_old` tables after a rename).
    if table_exists(&tx, "graph_nodes_old") {
        tx.execute_batch(
            "INSERT OR IGNORE INTO graph_nodes (id, node_type, label, mentions, last_seen, sentiment)
             SELECT id,
                    CASE node_type
                        WHEN 'person' THEN 'person'
                        WHEN 'goal' THEN 'goal'
                        WHEN 'task' THEN 'task'
                        WHEN 'project' THEN 'goal'
                        ELSE 'topic'
                    END,
                    label,
                    COALESCE(mentions_count, 1),
                    last_mentioned_at,
                    COALESCE(sentiment, 'neutral')
             FROM graph_nodes_old;",
        )
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    }
    if table_exists(&tx, "graph_edges_old") {
        // Only edges whose endpoints survived the import (avoids dangling nodes).
        tx.execute_batch(
            "INSERT OR IGNORE INTO graph_edges (id, from_node, to_node, edge_type, weight)
             SELECT e.id, e.from_node, e.to_node, e.edge_type, COALESCE(e.weight, 1.0)
             FROM graph_edges_old e
             WHERE EXISTS (SELECT 1 FROM graph_nodes a WHERE a.id = e.from_node)
               AND EXISTS (SELECT 1 FROM graph_nodes b WHERE b.id = e.to_node);",
        )
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    }

    // v1 daily digests → thoughts (and their JSON task lists → tasks).
    let mut imported_titles: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        if let Ok(mut stmt) = tx.prepare("SELECT title FROM tasks") {
            if let Ok(rows) = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .and_then(|r| r.collect::<SqlResult<Vec<_>>>())
            {
                for t in rows {
                    set.insert(t.trim().to_lowercase());
                }
            }
        }
        set
    };

    if table_exists(&tx, "historical_notes") {
        let mut stmt = tx
            .prepare("SELECT id, day_key, summary, tasks, created_at FROM historical_notes")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let notes: Vec<(String, String, String, String, String)> = rows
            .collect::<SqlResult<Vec<_>>>()
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        for (id, day, summary, tasks_json, created_at) in notes {
            tx.execute(
                "INSERT OR IGNORE INTO thoughts (id, text, summary, language, day, started_at, duration_secs, created_at)
                 VALUES (?1, ?2, ?2, 'en', ?3, ?4, 0, ?4)",
                params![id, summary, day, created_at],
            )
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
            if let Ok(items) = serde_json::from_str::<Vec<String>>(&tasks_json) {
                for item in items {
                    let title = item.trim();
                    let key = title.to_lowercase();
                    if title.len() < 3 || !imported_titles.insert(key) {
                        continue;
                    }
                    tx.execute(
                        "INSERT OR IGNORE INTO tasks (id, title, project, thought_id, done, created_at)
                         VALUES (?1, ?2, NULL, ?3, 0, ?4)",
                        params![uuid::Uuid::new_v4().to_string(), title, id, created_at],
                    )
                    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
                }
            }
        }
    }

    // v1 'task' suggestions (only the ones still pending/kept) → tasks.
    if table_exists(&tx, "suggestions") {
        let mut stmt = tx
            .prepare("SELECT id, text, status FROM suggestions WHERE suggestion_type = 'task'")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let rows2 = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        let rows: Vec<(String, String, String)> = rows2
            .collect::<SqlResult<Vec<_>>>()
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        for (id, text, status) in rows {
            let title = text.trim();
            let key = title.to_lowercase();
            if title.len() < 3 || !imported_titles.insert(key) {
                continue;
            }
            let done = if status == "pending" { 0 } else { 1 };
            tx.execute(
                "INSERT OR IGNORE INTO tasks (id, title, project, thought_id, done, created_at)
                 VALUES (?1, ?2, NULL, NULL, ?3, datetime('now'))",
                params![id, title, done],
            )
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
        }
    }

    // v1 settings → meta (insert-or-ignore: current values win on conflict).
    if table_exists(&tx, "settings") {
        tx.execute_batch("INSERT OR IGNORE INTO meta (key, value) SELECT key, value FROM settings;")
            .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    }

    set_meta(&tx, "schema_version", SCHEMA_VERSION)?;
    tx.commit().map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

// ── Thoughts ────────────────────────────────────────────────────────────────

pub fn insert_thought(conn: &Connection, t: &Thought) -> Result<()> {
    conn.execute(
        "INSERT INTO thoughts (id, text, summary, language, day, started_at, duration_secs, created_at, kind, minutes, fog, noise)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            t.id,
            t.text,
            t.summary,
            t.language,
            t.day,
            t.started_at,
            t.duration_secs,
            t.created_at,
            t.kind,
            t.minutes,
            t.fog,
            t.noise
        ],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

fn thought_from_row(row: &rusqlite::Row) -> SqlResult<Thought> {
    Ok(Thought {
        id: row.get(0)?,
        text: row.get(1)?,
        summary: row.get(2)?,
        language: row.get(3)?,
        day: row.get(4)?,
        started_at: row.get(5)?,
        duration_secs: row.get(6)?,
        created_at: row.get(7)?,
        kind: row.get(8)?,
        minutes: row.get(9)?,
        fog: row.get(10)?,
        noise: row.get(11)?,
    })
}

pub const THOUGHT_COLS: &str =
    "id, text, summary, language, day, started_at, duration_secs, created_at, kind, minutes, fog, noise";

/// Recent thoughts, newest first.
#[cfg(test)]
pub fn get_thoughts(conn: &Connection, limit: i64) -> Result<Vec<Thought>> {
    let mut stmt = conn
        .prepare(&format!("SELECT {THOUGHT_COLS} FROM thoughts ORDER BY created_at DESC LIMIT ?1"))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map(params![limit], thought_from_row)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    rows.collect::<SqlResult<Vec<_>>>()
        .map_err(|e| crate::error::Error::Store(e.to_string()))
}

pub fn get_thought(conn: &Connection, id: &str) -> Result<Option<Thought>> {
    let mut stmt = conn
        .prepare(&format!("SELECT {THOUGHT_COLS} FROM thoughts WHERE id = ?1"))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let one = stmt
        .query_row(params![id], thought_from_row)
        .optional()
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(one)
}

pub fn get_thoughts_for_day(conn: &Connection, day: &str) -> Result<Vec<Thought>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {THOUGHT_COLS} FROM thoughts WHERE day = ?1 ORDER BY created_at DESC"
        ))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map(params![day], thought_from_row)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    rows.collect::<SqlResult<Vec<_>>>()
        .map_err(|e| crate::error::Error::Store(e.to_string()))
}

/// Delete a thought and everything derived from it (tasks + vectors).
pub fn delete_thought(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM embeddings WHERE ref_kind = 'task' AND ref_id IN (SELECT id FROM tasks WHERE thought_id = ?1)",
        params![id],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    conn.execute("DELETE FROM tasks WHERE thought_id = ?1", params![id])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    conn.execute("DELETE FROM items WHERE thought_id = ?1", params![id])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    conn.execute(
        "DELETE FROM embeddings WHERE ref_kind = 'thought' AND ref_id = ?1",
        params![id],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    conn.execute("DELETE FROM thoughts WHERE id = ?1", params![id])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

// ── Items (ideas, learnings, questions, decisions) ──────────────────────────

/// Something worth keeping that was said (see `engine::sort`). Tasks live in
/// the `tasks` table; everything else lands here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub thought_id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
}

fn item_from_row(r: &rusqlite::Row) -> SqlResult<Item> {
    Ok(Item { id: r.get(0)?, thought_id: r.get(1)?, kind: r.get(2)?, text: r.get(3)?, created_at: r.get(4)? })
}

pub fn insert_item(conn: &Connection, item: &Item) -> Result<()> {
    conn.execute(
        "INSERT INTO items (id, thought_id, kind, text, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![item.id, item.thought_id, item.kind, item.text, item.created_at],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

pub fn get_item(conn: &Connection, id: &str) -> Result<Option<Item>> {
    conn.query_row("SELECT id, thought_id, kind, text, created_at FROM items WHERE id = ?1", params![id], item_from_row)
        .optional()
        .map_err(|e| crate::error::Error::Store(e.to_string()))
}

pub fn items_for_thought(conn: &Connection, thought_id: &str) -> Result<Vec<Item>> {
    let mut stmt = conn
        .prepare("SELECT id, thought_id, kind, text, created_at FROM items WHERE thought_id = ?1 ORDER BY rowid")
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map(params![thought_id], item_from_row)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// Items and open tasks that have no search vector yet: (ref_kind, id, text).
pub fn unindexed_statements(conn: &Connection, model: &str) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT 'item', id, text FROM items i WHERE NOT EXISTS
               (SELECT 1 FROM embeddings e WHERE e.ref_kind = 'item' AND e.ref_id = i.id AND e.model = ?1)
             UNION ALL
             SELECT 'task', id, title FROM tasks t WHERE NOT EXISTS
               (SELECT 1 FROM embeddings e WHERE e.ref_kind = 'task' AND e.ref_id = t.id AND e.model = ?1)",
        )
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map(params![model], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(rows.flatten().collect())
}

// ── Tasks ───────────────────────────────────────────────────────────────────

pub fn insert_task(conn: &Connection, t: &Task) -> Result<()> {
    conn.execute(
        "INSERT INTO tasks (id, title, project, thought_id, done, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![t.id, t.title, t.project, t.thought_id, t.done as i64, t.created_at],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

fn task_from_row(row: &rusqlite::Row) -> SqlResult<Task> {
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        project: row.get(2)?,
        thought_id: row.get(3)?,
        done: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
    })
}

pub const TASK_COLS: &str = "id, title, project, thought_id, done, created_at";

pub fn get_tasks(conn: &Connection, include_done: bool) -> Result<Vec<Task>> {
    let sql = if include_done {
        format!("SELECT {TASK_COLS} FROM tasks ORDER BY done ASC, created_at DESC LIMIT 200")
    } else {
        format!("SELECT {TASK_COLS} FROM tasks WHERE done = 0 ORDER BY created_at DESC LIMIT 200")
    };
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], task_from_row)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    rows.collect::<SqlResult<Vec<_>>>()
        .map_err(|e| crate::error::Error::Store(e.to_string()))
}

pub fn set_task_done(conn: &Connection, id: &str, done: bool) -> Result<()> {
    conn.execute("UPDATE tasks SET done = ?2 WHERE id = ?1", params![id, done as i64])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

pub fn rename_task(conn: &Connection, id: &str, title: &str) -> Result<()> {
    conn.execute("UPDATE tasks SET title = ?2 WHERE id = ?1", params![id, title])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

pub fn delete_task(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    conn.execute("DELETE FROM embeddings WHERE ref_kind = 'task' AND ref_id = ?1", params![id])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

// ── Calendar cache ──────────────────────────────────────────────────────────

pub fn replace_calendar_day(conn: &Connection, day: &str, events: &[CalendarEvent]) -> Result<()> {
    conn.execute("DELETE FROM calendar_events WHERE day = ?1", params![day])
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    for e in events {
        conn.execute(
            "INSERT INTO calendar_events (id, title, start, end, calendar, day)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![e.id, e.title, e.start, e.end, e.calendar, e.day],
        )
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    }
    Ok(())
}

// ── Meta ────────────────────────────────────────────────────────────────────

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| crate::error::Error::Store(e.to_string()))
}

/// Convenience: current app version in the store (used for update checks).
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kept alive for the process: if the TempDir dropped, the DB file would
    /// vanish and SQLite would start refusing writes.
    static TMP_DIRS: std::sync::Mutex<Vec<tempfile::TempDir>> =
        std::sync::Mutex::new(Vec::new());

    fn tmp_db() -> Connection {
        let dir = tempfile::tempdir().unwrap();
        let conn = super::open_database(dir.path()).unwrap();
        TMP_DIRS.lock().unwrap().push(dir);
        conn
    }

    #[test]
    fn thought_roundtrip() {
        let conn = tmp_db();
        let t = Thought {
            id: "t1".into(),
            text: "need to call the bank".into(),
            summary: None,
            language: "en".into(),
            day: crate::time::local_day_key(),
            started_at: "2026-10-13T10:00:00Z".into(),
            duration_secs: 30,
            created_at: "2026-10-13T10:00:00Z".into(),
            kind: "note".into(),
            minutes: None,
            fog: None,
            noise: false,
        };
        insert_thought(&conn, &t).unwrap();
        assert_eq!(get_thoughts(&conn, 10).unwrap().len(), 1);
        delete_thought(&conn, "t1").unwrap();
        assert!(get_thought(&conn, "t1").unwrap().is_none());
    }

    #[test]
    fn task_done_toggle() {
        let conn = tmp_db();
        let t = Task {
            id: "k1".into(),
            title: "Write report".into(),
            project: None,
            thought_id: None,
            done: false,
            created_at: "2026-10-13T10:00:00Z".into(),
        };
        insert_task(&conn, &t).unwrap();
        set_task_done(&conn, "k1", true).unwrap();
        assert!(get_tasks(&conn, false).unwrap().is_empty());
        assert_eq!(get_tasks(&conn, true).unwrap().len(), 1);
    }

    /// A database made by the pre-v2 app (different column shapes) must open
    /// cleanly, be upgraded in place, and have its data imported.
    #[test]
    fn legacy_database_upgrades_and_imports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodemind.db");

        // The v1 shapes (as found in the wild — note the NOT NULLs the new
        // code no longer writes, e.g. graph_nodes.created_at).
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY, title TEXT NOT NULL, project TEXT, due_hint TEXT,
                done INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'ambient', status TEXT NOT NULL DEFAULT 'official');
             INSERT INTO tasks VALUES ('t1', 'Legacy task', NULL, NULL, 0,
               '2026-04-01T00:00:00Z', 'ambient', 'official');
             CREATE TABLE graph_nodes (
                id TEXT PRIMARY KEY, node_type TEXT NOT NULL, label TEXT NOT NULL,
                data TEXT, created_at TEXT NOT NULL, mentions_count INTEGER NOT NULL DEFAULT 0,
                last_mentioned_at TEXT, sentiment TEXT NOT NULL DEFAULT 'neutral');
             CREATE TABLE focus_sessions (
                id TEXT PRIMARY KEY, title TEXT NOT NULL, date TEXT NOT NULL,
                start_time TEXT NOT NULL, end_time TEXT NOT NULL, notes TEXT,
                created_at TEXT NOT NULL);
             INSERT INTO focus_sessions VALUES ('f1', 'Legacy focus', '2026-04-01',
               '2026-04-01T09:00:00Z', '2026-04-01T09:25:00Z', NULL, '2026-04-01T09:00:00Z');
             CREATE TABLE graph_nodes_old (
                id TEXT PRIMARY KEY, node_type TEXT, label TEXT, data TEXT, created_at TEXT,
                mentions_count INTEGER, last_mentioned_at TEXT, sentiment TEXT);
             INSERT INTO graph_nodes_old VALUES
               ('person-rich', 'person', 'Rich Carapathy', '{}', '2026-04-05T00:00:00Z', 1,
                '2026-04-05T00:00:00Z', 'neutral'),
               ('project-nodemind', 'project', 'Nodemind', '{}', '2026-04-05T00:00:00Z', 2,
                '2026-04-05T00:00:00Z', 'neutral');
             CREATE TABLE graph_edges_old (
                id TEXT PRIMARY KEY, from_node TEXT, to_node TEXT, edge_type TEXT,
                weight REAL, last_updated TEXT);
             INSERT INTO graph_edges_old VALUES ('e1', 'person-rich', 'project-nodemind',
               'relates_to', 1.0, '2026-04-05T00:00:00Z');
             CREATE TABLE historical_notes (
                id TEXT PRIMARY KEY, day_key TEXT, summary TEXT, highlights TEXT, ideas TEXT,
                tasks TEXT, note_count INTEGER, created_at TEXT);
             INSERT INTO historical_notes VALUES ('history-1', '2026-04-04',
               'Day summary about Nodemind and No Cast', '[]', '[]',
               '[\"Call the bank\", \"Email Sam\"]', 1, '2026-04-04T10:00:00Z');
             CREATE TABLE suggestions (
                id TEXT PRIMARY KEY, suggestion_type TEXT, text TEXT, priority TEXT,
                status TEXT, created_at TEXT);
             INSERT INTO suggestions VALUES ('s1', 'task', 'Explore TRL library', 'medium',
               'pending', '2026-04-05T00:00:00Z');
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO settings VALUES ('llm_model', 'gemma4:12b-mlx');",
        )
        .unwrap();
        drop(conn);

        // Open: used to panic ("no such column: thought_id").
        let conn = super::open_database(dir.path()).unwrap();

        // Legacy row survives with the new column added.
        let tasks = get_tasks(&conn, true).unwrap();
        let legacy = tasks.iter().find(|t| t.id == "t1").expect("legacy task kept");
        assert!(!legacy.done);
        assert!(legacy.thought_id.is_none());

        // Imported: 2 note tasks + 1 suggestion; deduped by title.
        let titles: Vec<_> = tasks.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"Call the bank"));
        assert!(titles.contains(&"Explore TRL library"));
        assert_eq!(tasks.len(), 4, "1 legacy + 2 note + 1 suggestion: {titles:?}");

        // Notes became thoughts (the map shows items, not recordings).
        assert_eq!(get_thoughts(&conn, 10).unwrap().len(), 1);
        let graph_nodes = crate::store::graph::get_nodes(&conn).unwrap();
        let kinds: Vec<_> = graph_nodes.iter().map(|n| n.node_type.as_str()).collect();
        assert!(kinds.contains(&"person"));
        assert!(kinds.contains(&"goal"), "project remapped to goal: {kinds:?}");

        // Edges imported only when both endpoints exist.
        let edges = crate::store::graph::get_graph(&conn, 100).unwrap().edges;
        assert_eq!(edges.len(), 1);

        // Settings + version stamp.
        assert_eq!(get_meta(&conn, "llm_model").unwrap().as_deref(), Some("gemma4:12b-mlx"));
        assert_eq!(get_meta(&conn, "schema_version").unwrap().as_deref(), Some("2"));

        // The rebuilt focus table accepts the new write pattern (nullable
        // title; the v1 NOT NULL date/start_time columns are gone) and the
        // legacy row survived the rebuild.
        conn.execute(
            "INSERT INTO focus_sessions (id, title, day, started_at, ended_at, minutes)
             VALUES ('f2', NULL, '2026-04-02', '2026-04-02T09:00:00Z', NULL, 0)",
            [],
        )
        .unwrap();
        // The legacy row kept its data across the column rename (date → day).
        let legacy_title: Option<String> = conn
            .query_row("SELECT title FROM focus_sessions WHERE day = '2026-04-01'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(legacy_title.as_deref(), Some("Legacy focus"));
        drop(conn);

        // Second open: idempotent — no duplicate imports, no re-stamp churn.
        let conn = super::open_database(dir.path()).unwrap();
        assert_eq!(get_tasks(&conn, true).unwrap().len(), 4);
        assert_eq!(get_thoughts(&conn, 10).unwrap().len(), 1);
        assert_eq!(crate::store::graph::get_nodes(&conn).unwrap().len(), 2);
        let focus_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM focus_sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(focus_count, 2);
        let _ = dir;
    }
}
