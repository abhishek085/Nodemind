//! Today commands: the friction brief + focus sessions.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::engine::llm;
use crate::error::Result;
use crate::state::AppState;
use crate::store::db::{self, FocusSession};
use crate::time;

/// The daily "friction" insight for the Today view.
///
/// Compares the mind's activity (thoughts, open tasks) against the day's
/// structure (calendar, focus). The insight costs an LLM call, so it is
/// cached per day and regenerated after each capture (see
/// [`refresh_brief_in_background`]) or when `refresh` is set; the counts are
/// always live. Falls back to a rule-based insight when the LLM isn't ready.
#[tauri::command]
pub async fn get_today_brief(
    state: State<'_, Arc<AppState>>,
    refresh: Option<bool>,
) -> Result<TodayBrief> {
    let snap = snapshot(&state)?;
    let cached = if refresh.unwrap_or(false) { None } else { cached_insight(&state, &snap.day) };
    let (friction, suggestion) = match cached {
        Some(insight) => insight,
        None => {
            let insight = generate_insight(&snap).await;
            store_insight(&state, &snap.day, &insight);
            insight
        }
    };
    Ok(snap.into_brief(friction, suggestion))
}

/// Regenerate the cached insight off the request path (after a capture).
pub fn refresh_brief_in_background(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let Ok(snap) = snapshot(&state) else {
            return;
        };
        let (friction, suggestion) = generate_insight(&snap).await;
        store_insight(&state, &snap.day, &(friction.clone(), suggestion.clone()));
        state.emit("today-brief", &serde_json::to_value(snap.into_brief(friction, suggestion)).unwrap_or_default());
    });
}

/// Everything the brief is computed from — all cheap local reads.
struct Snapshot {
    day: String,
    open_tasks: Vec<crate::store::db::Task>,
    thoughts_today: usize,
    events_today: usize,
    scheduled_hours: f64,
    focus_minutes: i64,
    fog_json: String,
}

impl Snapshot {
    fn into_brief(self, friction: Option<String>, suggestion: Option<String>) -> TodayBrief {
        TodayBrief {
            friction,
            suggestion,
            open_tasks: self.open_tasks.len(),
            thoughts_today: self.thoughts_today,
            events_today: self.events_today,
            scheduled_hours: self.scheduled_hours,
            focus_minutes: self.focus_minutes,
        }
    }
}

fn snapshot(state: &AppState) -> Result<Snapshot> {
    let day = time::local_day_key();
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    let events = db::get_calendar_day(&conn, &day).unwrap_or_default();
    Ok(Snapshot {
        open_tasks: db::get_tasks(&conn, false).unwrap_or_default(),
        thoughts_today: db::get_thoughts_for_day(&conn, &day).unwrap_or_default().len(),
        events_today: events.len(),
        scheduled_hours: crate::calendar::apple::scheduled_hours(&events),
        focus_minutes: db::focus_minutes_today(&conn, &day),
        // Recent fog signals (written by the pipeline).
        fog_json: db::get_meta(&conn, "fog_today").ok().flatten().unwrap_or_else(|| "[]".into()),
        day,
    })
}

async fn generate_insight(snap: &Snapshot) -> (Option<String>, Option<String>) {
    let task_list = snap
        .open_tasks
        .iter()
        .take(12)
        .map(|t| format!("\"{}\"", t.title))
        .collect::<Vec<_>>()
        .join(", ");
    let input = llm::BriefInput {
        open_tasks: snap.open_tasks.len(),
        open_task_list: if task_list.is_empty() { "(none)".into() } else { task_list },
        thoughts_today: snap.thoughts_today,
        events_today: snap.events_today,
        scheduled_hours: snap.scheduled_hours,
        focus_minutes: snap.focus_minutes,
        fog_json: snap.fog_json.clone(),
    };
    match llm::daily_brief(input).await.ok() {
        Some(b) if b.friction.is_some() || b.suggestion.is_some() => (b.friction, b.suggestion),
        _ => rule_based_brief(
            &snap.open_tasks,
            snap.thoughts_today,
            snap.events_today,
            snap.scheduled_hours,
            snap.focus_minutes,
        ),
    }
}

fn cached_insight(state: &AppState, day: &str) -> Option<(Option<String>, Option<String>)> {
    let conn = state.db.lock().ok()?;
    let raw = db::get_meta(&conn, "today_brief").ok().flatten()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    (v["day"].as_str() == Some(day)).then(|| {
        (
            v["friction"].as_str().map(str::to_string),
            v["suggestion"].as_str().map(str::to_string),
        )
    })
}

fn store_insight(state: &AppState, day: &str, insight: &(Option<String>, Option<String>)) {
    if let Ok(conn) = state.db.lock() {
        let v = serde_json::json!({ "day": day, "friction": insight.0, "suggestion": insight.1 });
        let _ = db::set_meta(&conn, "today_brief", &v.to_string());
    }
}

/// Deterministic fallback when the LLM can't answer.
fn rule_based_brief(
    open_tasks: &[crate::store::db::Task],
    thoughts: usize,
    events: usize,
    scheduled_hours: f64,
    focus_minutes: i64,
) -> (Option<String>, Option<String>) {
    let loud_mind = thoughts >= 3 || open_tasks.len() >= 4;
    let empty_day = events == 0 && focus_minutes == 0;

    let friction = if loud_mind && empty_day {
        Some(format!(
            "Your mind is loud ({thoughts} brain-dumps, {} open tasks) but the day is empty — 0 h scheduled, 0 min focused.",
            open_tasks.len()
        ))
    } else if loud_mind && scheduled_hours >= 6.0 {
        Some(format!(
            "{scheduled_hours:.0} h of a day already committed while {} tasks wait — the calendar is outpacing the checklist.",
            open_tasks.len()
        ))
    } else {
        None
    };

    let suggestion = if let Some(task) = open_tasks.first() {
        Some(format!("Clear the top of the list: \"{}\"", truncate(&task.title, 40)))
    } else if empty_day {
        Some("Carve one 25-minute focus block — the mind settles when it has a container.".into())
    } else {
        Some("You're balanced. Ride the momentum.".into())
    };

    (friction, suggestion)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayBrief {
    pub friction: Option<String>,
    pub suggestion: Option<String>,
    pub open_tasks: usize,
    pub thoughts_today: usize,
    pub events_today: usize,
    pub scheduled_hours: f64,
    pub focus_minutes: i64,
}

// ── Focus sessions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusStarted {
    pub id: String,
    pub started_at: String,
    pub minutes: i64,
}

#[tauri::command]
pub fn start_focus(state: State<'_, Arc<AppState>>, minutes: Option<u32>) -> Result<FocusStarted> {
    let minutes = minutes.unwrap_or(25) as i64;
    let session = FocusSession {
        id: uuid::Uuid::new_v4().to_string(),
        title: None,
        day: time::local_day_key(),
        started_at: chrono::Utc::now().to_rfc3339(),
        ended_at: None,
        minutes,
    };
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::insert_focus(&conn, &session)?;
    Ok(FocusStarted {
        id: session.id,
        started_at: session.started_at,
        minutes,
    })
}

#[tauri::command]
pub fn finish_focus(state: State<'_, Arc<AppState>>, id: String, minutes: Option<i64>) -> Result<()> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    let ended = chrono::Utc::now().to_rfc3339();
    db::finish_focus(&conn, &id, &ended, minutes.unwrap_or(0))?;
    state.emit("focus-updated", &serde_json::json!({}));
    Ok(())
}

#[tauri::command]
pub fn get_focus_today(state: State<'_, Arc<AppState>>) -> Result<Vec<FocusSession>> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::get_focus_today(&conn, &time::local_day_key())
}
